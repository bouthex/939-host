"""Capturas de la interfaz con el backend simulado (vista previa local, sin Tauri).
Uso: python3 ci/vista-previa/capturas.py [salida]   (usa Chrome o Chromium del sistema)"""
import json, os, sys, pathlib, shutil
from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/vista-previa")
OUT.mkdir(parents=True, exist_ok=True)
datos = (ROOT / "ci/vista-previa/datos.json").read_text()
mock = (ROOT / "ci/vista-previa/mock.js").read_text()
chrome = shutil.which("google-chrome") or shutil.which("chromium") or shutil.which("chromium-browser")
RUTAS = [("servers", {}), ("server", {"id": "srv-demo"}), ("people", {}), ("friends", {}), ("settings", {}), ("trash", {}), ("create", {})]
TABS = ["summary", "console", "players", "mods", "world", "files", "backups", "group", "settings"]
TAMANOS = [(1024, 680, 1), (1280, 820, 1), (1600, 1000, 1.5)]
errores = []
with sync_playwright() as p:
    b = p.chromium.launch(executable_path=chrome, args=["--allow-file-access-from-files"])
    for (w, h, s) in TAMANOS:
        for motion in ["no-preference", "reduce"] if w == 1280 else ["no-preference"]:
            ctx = b.new_context(viewport={"width": w, "height": h}, device_scale_factor=s, reduced_motion=motion)
            page = ctx.new_page()
            page.on("pageerror", lambda e: errores.append(f"{w}x{h}: {e}"))
            page.on("console", lambda m: errores.append(f"{w}x{h} consola: {m.text}") if m.type == "error" else None)
            page.add_init_script(f"window.__DATOS__ = {datos};\n{mock}")
            page.goto((ROOT / "ui/index.html").as_uri())
            page.wait_for_function("!document.getElementById('splash')", timeout=8000)
            page.wait_for_timeout(1500 if motion == "no-preference" else 300)
            tag = f"{w}x{h}" + ("-reducido" if motion == "reduce" else "")
            if w == 1280 and motion == "no-preference":
                page.screenshot(path=str(OUT / f"{tag}-00-inicio.png"))
            for r, prm in RUTAS:
                page.evaluate(f"go({json.dumps(r)}, {json.dumps(prm)})")
                page.wait_for_timeout(600)
                if r == "people":
                    page.evaluate("A['ppl-open']({ dataset: { node: 'n1' } })")
                    page.wait_for_timeout(300)
                page.screenshot(path=str(OUT / f"{tag}-{r}.png"))
                if r == "server" and motion == "no-preference":
                    for t in TABS:
                        page.evaluate(f"state.tab = {json.dumps(t)}; drawTab && drawTab()")
                        page.wait_for_timeout(400)
                        page.screenshot(path=str(OUT / f"{tag}-server-{t}.png"))
                        msg = page.evaluate("() => { const n = document.querySelector('#srv-tab .note'); return n && n.innerText.includes('No se pudo cargar') ? n.innerText : '' }")
                        if msg: errores.append(f"{tag} pestaña {t}: {msg}".replace("\n", " | "))
            # Recortes de texto: elementos cuyo contenido no entra (sin elipsis intencional).
            cortes = page.evaluate("""() => [...document.querySelectorAll('button, .btn, h1, h2, b, .li')].filter(e => e.offsetParent && e.scrollWidth > e.clientWidth + 2 && getComputedStyle(e).textOverflow !== 'ellipsis' && getComputedStyle(e).overflow !== 'visible').map(e => (e.className + ': ' + e.textContent.trim()).slice(0, 80)).slice(0, 20)""")
            for c in cortes: errores.append(f"{tag} recorte: {c}")
            ctx.close()
    b.close()
print("\n".join(sorted(set(errores))) or "sin errores")
