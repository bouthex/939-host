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
                try:
                    await shot("ERROR-" + what.replace(" ", "-")[:40], 100)
                except Exception:
                    pass  # la app puede haberse cerrado a propósito (apagado simulado)
                log(False, what, f"{type(e).__name__}: {str(e).splitlines()[0][:300] if str(e) else ''}")
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

        async def people():
            await p.click('[data-a=nav][data-route=people]')
            await p.wait_for_selector('h1:has-text("Amigos")', timeout=15000)
            await p.click('[data-a=ppl-invite]')
            await p.wait_for_selector('.dialog .addr', timeout=15000)
            code = (await p.inner_text('.dialog .addr')).strip()
            await shot("amigos-invitar", 200)
            await p.click('.dialog .btn.pri')
            if not code.startswith("amigo:"):
                raise RuntimeError(f"código inesperado: {code[:20]}")
            return "sección Amigos y código de invitación"
        await step("amigos: abrir la sección e invitar", people())
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
                # Como la X o «Cerrar ventana» de la barra de tareas: WM_CLOSE a la ventana.
                import ctypes
                u = ctypes.windll.user32
                hwnd = u.FindWindowW(None, "939 Host")
                if not hwnd:
                    raise RuntimeError("no se encontró la ventana «939 Host»")
                u.PostMessageW(hwnd, 0x0010, 0, 0)
                await p.wait_for_selector('.dialog', timeout=10000)
                await shot("cerrar-ventana-aviso", 300)
                await p.click('.dialog .btn.pri')
                await asyncio.sleep(6)
                procs = medir.processes()
                metrics["bandeja_con_servidor"] = {**medir.memory(), "cpu_pct": medir.cpu(10), "guardian_mb": medir.memory_of("guardian"), "java_mb": medir.memory_of("java")}
                if not procs.get("guardian"):
                    raise RuntimeError("no hay guardián con el servidor encendido")
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

        # ── Cierre inesperado y apagado de Windows con el servidor encendido (fase final §4–§5) ──
        import subprocess, ctypes

        def server_dir():
            base = os.path.join(os.environ["LOCALAPPDATA"], "939Host", "servers")
            return [os.path.join(base, d) for d in os.listdir(base) if os.path.isdir(os.path.join(base, d, "server"))][0]

        def latest_log():
            return os.path.join(server_dir(), "server", "logs", "latest.log")

        def log_since(start):
            with open(latest_log(), "rb") as f:
                data = f.read()
            return data[start:].decode("utf-8", "replace") if len(data) >= start else data.decode("utf-8", "replace")

        async def wait_procs(cond, timeout, what):
            end = time.time() + timeout
            while time.time() < end:
                if cond(medir.processes()):
                    return
                await asyncio.sleep(1)
            raise TimeoutError(f"{what}: {medir.processes()}")

        async def reopen_app():
            nonlocal b, p
            subprocess.Popen([os.path.join(os.environ["LOCALAPPDATA"], "Programs", "939 Host", "939Host.exe")])
            b, p = await find_page(pw)
            p.on("pageerror", lambda e: errors.append(str(e)))
            await p.wait_for_selector(".nav-wrap", timeout=60000)

        async def open_server():
            await p.click('[data-a=nav][data-route=servers]')
            await p.click('.li[data-id]')
            await p.click('.tab[data-v=summary]')

        def read_record():
            path = os.path.join(server_dir(), "ultimo-cierre.json")
            if not os.path.exists(path):
                raise RuntimeError("no quedó anotado qué pasó (ultimo-cierre.json)")
            return json.load(open(path, encoding="utf-8"))

        def saved_in(text):
            # 26.3 ya no escribe «All dimensions are saved»: la secuencia de cierre es Saving players → Saving worlds.
            return any(k in text for k in ("All dimensions are saved", "All chunks are saved", "Saved the game")) or ("Saving players" in text and "Saving worlds" in text)

        if ok:
            async def crash():
                await open_server()
                await p.click('.ctl [data-a=start]')
                await wait_state(["Listo para jugar"], 600)
                start = os.path.getsize(latest_log())
                if not medir.processes().get("guardian"):
                    raise RuntimeError("no hay guardián con el servidor encendido")
                main = medir.main_process()
                t0 = time.time()
                main.kill()  # TerminateProcess solo del proceso principal (como el Administrador de tareas)
                await wait_procs(lambda pr: not pr.get("java"), 660, "Java sigue encendido")
                secs = round(time.time() - t0, 1)
                await wait_procs(lambda pr: not pr.get("guardian") and not pr.get("939Host"), 30, "quedó un proceso de 939 Host")
                rec = read_record()
                tail = log_since(start)
                if "Stopping" not in tail or not saved_in(tail):
                    raise RuntimeError("Minecraft no registró un guardado y detención limpios")
                if rec.get("outcome") != "guardado" or rec.get("cause") != "cierre-inesperado":
                    raise RuntimeError(f"registro inesperado: {rec}")
                return f"Java guardó y se detuvo {secs} s después de matar 939 Host; no quedó ningún proceso"
            crashed = await step("cierre inesperado de 939 Host: el guardián guarda y detiene el servidor", crash())
            if crashed:
                async def after_crash():
                    await reopen_app()
                    await p.wait_for_selector('.toast', timeout=20000)
                    txt = (await (await p.query_selector('.toast')).inner_text()).strip().replace("\n", " ")
                    await shot("aviso-ultimo-cierre", 200)
                    if os.path.exists(os.path.join(server_dir(), "ultimo-cierre.json")):
                        raise RuntimeError("el aviso no se mostró (sigue pendiente)")
                    await open_server()
                    await p.click('.ctl [data-a=start]')
                    st = await wait_state(["Listo para jugar"], 600)
                    return f"aviso «{txt[:90]}»; vuelve a encender: {st}"
                again = await step("después del cierre inesperado: aviso y el servidor vuelve a encender", after_crash())
                if again:
                    async def session_end():
                        start = os.path.getsize(latest_log())
                        u = ctypes.windll.user32
                        u.SendMessageW.restype = ctypes.c_ssize_t
                        u.SendMessageW.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_size_t, ctypes.c_ssize_t]
                        hwnd = u.FindWindowW("939HostSesion", None)
                        if not hwnd:
                            raise RuntimeError("no está la ventana que recibe el aviso de apagado")
                        # Lo que manda Windows antes de apagar o reiniciar.
                        r = u.SendMessageW(hwnd, 0x0011, 0, 0)
                        if r != 0:
                            raise RuntimeError(f"no pidió tiempo para guardar (respondió {r})")
                        t0 = time.time()
                        await wait_procs(lambda pr: not pr.get("java") and not pr.get("939Host") and not pr.get("guardian"), 240, "no terminó el cierre ordenado")
                        rec = read_record()
                        if not saved_in(log_since(start)):
                            raise RuntimeError("Minecraft no registró el guardado")
                        if rec.get("cause") != "apagado-windows" or rec.get("outcome") != "guardado":
                            raise RuntimeError(f"registro inesperado: {rec}")
                        return f"pidió tiempo a Windows, guardó y terminó en {round(time.time() - t0, 1)} s"
                    await step("apagado de Windows simulado: pide tiempo, guarda, detiene y termina", session_end())
                    async def no_servers():
                        await reopen_app()
                        u = ctypes.windll.user32
                        u.SendMessageW.restype = ctypes.c_ssize_t
                        u.SendMessageW.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_size_t, ctypes.c_ssize_t]
                        await p.wait_for_timeout(2000)
                        hwnd = u.FindWindowW("939HostSesion", None)
                        r = u.SendMessageW(hwnd, 0x0011, 0, 0)
                        if r != 1:
                            raise RuntimeError(f"demoró el apagado sin servidores encendidos (respondió {r})")
                        return "sin servidores encendidos no demora el apagado"
                    await step("apagado de Windows sin servidores: no lo demora", no_servers())

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
