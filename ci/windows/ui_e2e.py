"""Prueba de punta a punta de la app real en Windows (CI).

Se conecta a la ventana de 939Host.exe por el puerto de depuración de WebView2 y la usa como
una persona: crea un servidor desde el asistente, espera la descarga real, lo enciende, usa la
consola, hace una copia, recorre todas las pantallas en claro y oscuro y lo apaga.

No hay datos simulados: todo pasa por el backend real de la app. Las capturas quedan en
`capturas/` para revisión visual.
"""
import asyncio, os, sys, time, json
from playwright.async_api import async_playwright

OUT = os.environ.get("OUT", "capturas")
os.makedirs(OUT, exist_ok=True)
results = []


def log(ok, what, extra=""):
    results.append({"ok": ok, "paso": what, "detalle": extra})
    print(("✔ " if ok else "✘ ") + what + (f" — {extra}" if extra else ""), flush=True)


async def find_page(pw):
    last = None
    for _ in range(90):
        try:
            b = await pw.chromium.connect_over_cdp("http://127.0.0.1:9222")
            for c in b.contexts:
                for p in c.pages:
                    if "index.html" in p.url or p.url.startswith("http://tauri.localhost") or p.url.startswith("tauri://"):
                        return b, p
            last = "conectado, páginas: " + ", ".join(p.url for c in b.contexts for p in c.pages)
        except Exception as e:
            last = str(e).splitlines()[0]
        await asyncio.sleep(2)
    raise SystemExit(f"No se pudo conectar con la ventana de 939 Host (WebView2): {last}")


async def main():
    async with async_playwright() as pw:
        b, p = await find_page(pw)
        errors = []
        p.on("pageerror", lambda e: errors.append(str(e)))
        p.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
        await p.wait_for_selector(".nav-wrap, .welcome", timeout=60000)
        await p.wait_for_timeout(1500)
        n = [0]

        async def shot(name, wait=600):
            await p.wait_for_timeout(wait)
            n[0] += 1
            await p.screenshot(path=f"{OUT}/{n[0]:02d}-{name}.png")

        async def step(what, coro):
            try:
                r = await coro
                log(True, what, r or "")
                return True
            except Exception as e:
                await shot("ERROR-" + what.replace(" ", "-")[:40], 100)
                log(False, what, str(e).splitlines()[0][:300])
                return False

        async def state_text():
            el = await p.query_selector(".srv-title .statepill")
            return (await el.inner_text()).strip() if el else ""

        async def wait_state(words, timeout):
            end = time.time() + timeout
            last = ""
            while time.time() < end:
                last = await state_text()
                if any(w in last for w in words):
                    return last
                if "Error" in last:
                    raise RuntimeError(f"estado {last}")
                await p.wait_for_timeout(2000)
            raise TimeoutError(f"estado final «{last}»")

        await shot("bienvenida-real")

        async def set_theme(t):
            await p.click('[data-a=nav][data-route=settings]')
            await p.wait_for_timeout(400)
            await p.click(f'[data-a=set-theme][data-v={t}]')
            await p.wait_for_timeout(500)
            return t

        await step("tema claro", set_theme("light"))
        await p.click('[data-a=nav][data-route=servers]')
        await shot("claro-servidores-vacio")

        # ── Crear servidor real desde el asistente ──
        async def create():
            await p.click('[data-a=new]')
            await p.wait_for_selector('[data-w=name]')
            await p.fill('[data-w=name]', 'Prueba Windows')
            await shot("crear-paso1")
            for i in range(3):
                await p.click('[data-a=wnext]')
                await shot(f"crear-paso{i + 2}", 400)
            await p.check('[data-w=eula]')
            await p.click('[data-a=wcreate]')
            await p.wait_for_selector(".srv-title", timeout=60000)
            return "asistente completo"

        if not await step("crear servidor Vanilla desde el asistente", create()):
            raise SystemExit(1)
        await shot("servidor-instalando", 3000)

        async def installed():
            # La descarga real (Java + servidor) puede tardar; el botón Iniciar se habilita al terminar.
            end = time.time() + 900
            while time.time() < end:
                btn = await p.query_selector('.ctl [data-a=start]')
                if btn and await btn.is_enabled():
                    return await state_text()
                await p.wait_for_timeout(3000)
            raise TimeoutError("la instalación no terminó")

        if not await step("instalación real (Java + servidor de Mojang)", installed()):
            raise SystemExit(1)

        async def start():
            await p.click('.ctl [data-a=start]')
            return await wait_state(["Funcionando"], 600)

        ok = await step("encender y llegar a «Funcionando»", start())
        await shot("claro-resumen-funcionando", 4000)
        if ok:
            async def console():
                await p.click('.tab[data-v=console]')
                await p.wait_for_selector('#con-in:not([disabled])')
                await p.fill('#con-in', 'say hola desde Windows')
                await p.keyboard.press('Enter')
                await p.wait_for_function("document.querySelector('.term')?.innerText.includes('hola desde Windows')", timeout=20000)
                return "comando visto en la consola"
            await step("consola: enviar comando", console())
            await shot("claro-consola")
            for tab in ["players", "mods", "world", "backups", "group", "settings"]:
                await step(f"pestaña {tab}", p.click(f'.tab[data-v={tab}]'))
                await shot(f"claro-{tab}", 900)

            async def backup():
                await p.click('.tab[data-v=backups]')
                await p.wait_for_timeout(500)
                before = await p.locator('[data-a=restore]').count()
                await p.click('[data-a=backup]')
                if await p.query_selector('.dialog .btn.pri'):
                    await p.click('.dialog .btn.pri')
                for _ in range(60):
                    await p.wait_for_timeout(1000)
                    if await p.locator('[data-a=restore]').count() > before:
                        return "copia creada con el servidor encendido"
                raise TimeoutError("no apareció la copia")
            await step("copia de seguridad en caliente", backup())
            await shot("claro-copias")

        # ── Oscuro: recorrido completo ──
        await step("tema oscuro", set_theme("dark"))
        await shot("oscuro-configuracion")
        await p.click('[data-a=set-toc][data-v=privacy]')
        await shot("oscuro-privacidad")
        await p.click('[data-a=nav][data-route=servers]')
        await shot("oscuro-servidores")
        await p.click('.li[data-id]')
        for tab in ["summary", "console", "players", "mods", "world", "backups", "group", "settings"]:
            await p.click(f'.tab[data-v={tab}]')
            await shot(f"oscuro-{tab}", 900)
        await p.click('[data-a=nav][data-route=friends]')
        if await p.query_selector('.dialog .btn.pri'):
            await p.keyboard.press('Escape')
        await shot("oscuro-amigos")

        # ── Apagar con confirmación ──
        await p.click('[data-a=nav][data-route=servers]')
        await p.click('.li[data-id]')
        await p.click('.tab[data-v=summary]')

        async def stop():
            await p.click('.ctl [data-a=stop]')
            await p.wait_for_timeout(500)
            if await p.query_selector('.dialog .btn.pri'):
                await shot("oscuro-confirmar-detener", 100)
                await p.click('.dialog .btn.pri')
            return await wait_state(["Detenido"], 180)
        if ok:
            await step("detener con confirmación y apagado limpio", stop())
        await shot("oscuro-detenido", 1000)

        real_errors = [e for e in errors if "favicon" not in e]
        log(not real_errors, "sin errores de JavaScript", "; ".join(real_errors[:5]))
        json.dump(results, open(f"{OUT}/resultados.json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)
        await b.close()
        if any(not r["ok"] for r in results):
            sys.exit(1)


asyncio.run(main())
