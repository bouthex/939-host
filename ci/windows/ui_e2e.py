"""Prueba de punta a punta de la app real en Windows (CI).

Se conecta a la ventana de 939Host.exe por el puerto de depuración de WebView2 y la usa como
una persona: crea un servidor desde el asistente, espera la descarga real, lo enciende, usa la
consola, hace una copia, recorre todas las pantallas en claro y oscuro y lo apaga.

No hay datos simulados: todo pasa por el backend real de la app. Las capturas quedan en
`capturas/` para revisión visual.
"""
import asyncio, os, sys, time, json
sys.path.insert(0, os.path.dirname(__file__))
import medir

sys.stdout.reconfigure(encoding="utf-8")
from playwright.async_api import async_playwright

OUT = os.environ.get("OUT", "capturas")
os.makedirs(OUT, exist_ok=True)
results = []


def make_png(path, w=300, h=200):
    """PNG real (degradado) para probar la carga de imágenes."""
    import zlib, struct
    raw = b"".join(b"\x00" + b"".join(bytes((x * 255 // w, y * 255 // h, 128)) for x in range(w)) for y in range(h))
    def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
    open(path, "wb").write(data)
    return path


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
        metrics = {}
        try:
            ready = await p.evaluate("window.__h939Ready || null")
            t0 = medir.started_at()
            if ready and t0: metrics["arranque_s"] = round(ready / 1000 - t0, 2)
        except Exception as e:
            metrics["arranque_error"] = str(e)
        await p.wait_for_timeout(8000)
        metrics["reposo"] = {**medir.memory(), "cpu_pct": medir.cpu(10)}
        try:
            await p.evaluate("window.__TAURI__.window.getCurrentWindow().minimize()")
            await asyncio.sleep(12)
            metrics["minimizado"] = {**medir.memory(), "cpu_pct": medir.cpu(10)}
            await p.evaluate("(async () => { const w = window.__TAURI__.window.getCurrentWindow(); await w.unminimize(); await w.setFocus(); })()")
            await p.wait_for_timeout(1500)
        except Exception as e:
            metrics["minimizado_error"] = str(e).splitlines()[0]
        print("MÉTRICAS (inicio): " + json.dumps(metrics, ensure_ascii=False), flush=True)

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
            # «Listo para jugar» solo aparece después de que 939 Host consultó de verdad al servidor.
            return await wait_state(["Listo para jugar"], 600)

        ok = await step("encender y llegar a «Listo para jugar» (conexión comprobada)", start())
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
            for tab in ["players", "mods", "world", "files", "backups", "group", "settings"]:
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
        for tab in ["summary", "console", "players", "mods", "world", "files", "backups", "group", "settings"]:
            await p.click(f'.tab[data-v={tab}]')
            await shot(f"oscuro-{tab}", 900)

        async def archivos():
            # Administrador de archivos real: crear una carpeta, verla, mandarla a la Papelera.
            await p.click('.tab[data-v=files]')
            await p.wait_for_selector('.fm-row', timeout=15000)
            p.once("dialog", lambda d: None)
            await p.click('[data-a=fm-mkdir]')
            await p.fill('#fm-n', 'carpeta-de-prueba')
            await p.click('.dialog .btn.pri')
            await p.wait_for_selector('.fm-row[data-p="carpeta-de-prueba"]', timeout=15000)
            await p.click('.fm-row[data-p="carpeta-de-prueba"] .fm-chk')
            await shot("oscuro-archivos-elegido", 400)
            await p.click('[data-a=fm-delete]')
            await p.click('.dialog .btn.pri')
            await p.wait_for_selector('.fm-row[data-p="carpeta-de-prueba"]', state='detached', timeout=15000)
            locked = await p.query_selector('.fm-row.locked')
            return "carpeta creada y enviada a la Papelera" + ("; mundo protegido con el servidor encendido" if locked else "")
        await step("administrador de archivos (crear y borrar a la Papelera)", archivos())

        # ── Logo: «Subir imagen» oculta los diseños, elegir una imagen funciona y se guarda ──
        png = make_png(os.path.abspath(os.path.join(OUT, "prueba-logo.png")))
        async def logo():
            await p.click('.tab[data-v=summary]')
            await p.click('[data-a=logo-edit]')
            await p.wait_for_selector('#lg-presets')
            await p.click('[data-lm=upload]')
            if await p.is_visible('#lg-presets .logo-grid'):
                raise RuntimeError("los diseños siguen visibles en «Subir imagen»")
            await p.set_input_files('#lg-file', png)
            await p.wait_for_selector('#lg-crop:not([hidden])', timeout=10000)
            box = await p.eval_on_selector('#pv-md img', "i => { const r = i.getBoundingClientRect(), b = i.parentElement.getBoundingClientRect(); return [r.width, r.height, b.width, b.height]; }")
            if box[0] > box[2] + 1 or box[1] > box[3] + 1:
                raise RuntimeError(f"la vista previa chica se sale de su recuadro: {box}")
            await shot("logo-subir-imagen", 300)
            await p.click('.dialog .btn.pri')
            await p.wait_for_selector('.srv-title .srv-ic.logo img', timeout=15000)
            data = await p.evaluate("p => window.__TAURI__.core.invoke('image_read', { path: p })", png)
            if not data.startswith("data:image/png;base64,"):
                raise RuntimeError("image_read no devolvió la imagen")
            try:
                await p.evaluate("p => window.__TAURI__.core.invoke('image_read', { path: p })", os.path.abspath(__file__))
                raise RuntimeError("image_read aceptó un archivo que no es imagen")
            except Exception as e:
                if "aceptó" in str(e): raise
            return "imagen elegida, recortada y guardada; lectura por ruta (arrastre) validada"
        await step("logo del servidor desde una imagen de la PC", logo())

        async def photo():
            await p.click('[data-a=nav][data-route=settings]')
            await p.click('[data-a=photo-edit]')
            await p.set_input_files('#pf-file', png)
            await p.wait_for_selector('#pf-crop:not([hidden])', timeout=10000)
            await shot("foto-de-perfil", 300)
            await p.click('.dialog .btn.pri')
            await p.wait_for_selector('#foot-ic .pcav.photo img', timeout=15000)
            return "foto guardada y visible en la barra lateral"
        await step("foto de perfil desde una imagen de la PC", photo())
        await p.click('[data-a=nav][data-route=servers]')
        await p.click('.li[data-id]')
        await p.click('[data-a=nav][data-route=trash]')
        await shot("oscuro-papelera", 700)
        await p.click('[data-a=nav][data-route=servers]')
        await p.click('[data-a=nav][data-route=modpack]')
        await shot("oscuro-modpack", 700)
        await p.click('[data-a=nav][data-route=friends]')
        if await p.query_selector('.dialog .btn.pri'):
            await p.keyboard.press('Escape')
        await shot("oscuro-amigos")

        # ── Cerrar la ventana con el servidor encendido: el servidor sigue (§14) ──
        if ok:
            async def close_window_keeps_server():
                await p.click('[data-a=nav][data-route=servers]')
                await p.evaluate("window.__TAURI__.window.getCurrentWindow().close()")
                await p.wait_for_selector('.dialog', timeout=10000)
                await shot("cerrar-ventana-aviso", 300)
                await p.click('.dialog .btn.pri')
                await asyncio.sleep(6)
                procs = medir.processes()
                if not procs.get("939Host"):
                    raise RuntimeError("939 Host terminó al cerrar la ventana")
                if not procs.get("java"):
                    raise RuntimeError("el servidor se detuvo al cerrar la ventana")
                return f"ventana cerrada; siguen 939 Host ({procs['939Host']}) y Java ({procs['java']}); WebView2: {procs.get('msedgewebview2', 0)}"
            closed = await step("cerrar la ventana no detiene el servidor", close_window_keeps_server())
            if closed:
                async def reopen():
                    nonlocal b, p
                    import subprocess
                    subprocess.Popen([os.path.join(os.environ["LOCALAPPDATA"], "Programs", "939 Host", "939Host.exe")])
                    b, p = await find_page(pw)
                    p.on("pageerror", lambda e: errors.append(str(e)))
                    await p.wait_for_selector(".nav-wrap", timeout=60000)
                    await p.click('.li[data-id]')
                    await p.click('.tab[data-v=summary]')
                    return await wait_state(["Listo para jugar", "Funcionando"], 60)
                await step("reabrir la app: recupera el estado real del servidor", reopen())
                await shot("reabierta-estado-real", 800)

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

        try:
            metrics["despues_de_usarla"] = {**medir.memory(), "cpu_pct": medir.cpu(10)}
            metrics["tamanos"] = medir.sizes()
        except Exception as e:
            metrics["final_error"] = str(e)
        print("MÉTRICAS: " + json.dumps(metrics, ensure_ascii=False), flush=True)
        json.dump(metrics, open(f"{OUT}/metricas.json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)
        real_errors = [e for e in errors if "favicon" not in e]
        log(not real_errors, "sin errores de JavaScript", "; ".join(real_errors[:5]))
        json.dump(results, open(f"{OUT}/resultados.json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)
        await b.close()
        if any(not r["ok"] for r in results):
            sys.exit(1)


asyncio.run(main())
