"""Pruebas de comportamiento de la interfaz con el backend simulado (sin Tauri).
Uso: python3 ci/vista-previa/comportamiento.py   → sale con código 1 si algo falla."""
import pathlib, shutil, sys
from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parents[2]
datos = (ROOT / "ci/vista-previa/datos.json").read_text()
mock = (ROOT / "ci/vista-previa/mock.js").read_text()
chrome = shutil.which("google-chrome") or shutil.which("chromium") or shutil.which("chromium-browser")
fallas, errores = [], []

def check(cond, msg):
    print(("✔ " if cond else "✘ ") + msg)
    if not cond: fallas.append(msg)

def abrir(b, **ctx):
    c = b.new_context(viewport={"width": 1280, "height": 820}, **ctx)
    pg = c.new_page()
    pg.on("pageerror", lambda e: errores.append(str(e)))
    pg.add_init_script(f"window.__DATOS__ = {datos};\n{mock}")
    pg.goto((ROOT / "ui/index.html").as_uri())
    pg.wait_for_timeout(1200)
    return c, pg

with sync_playwright() as p:
    b = p.chromium.launch(executable_path=chrome)
    c, pg = abrir(b)
    # Navegación: Servidores → servidor (Mods) → Modpacks → Volver: vuelve a Mods del servidor.
    pg.evaluate("document.querySelector('[data-a=open]').click()"); pg.wait_for_timeout(500)
    pg.evaluate("openTab('mods')"); pg.wait_for_timeout(500)
    check(pg.evaluate("!document.querySelector('[data-a=back]')"), "desde la lista de Servidores no aparece una flecha redundante")
    pg.evaluate("document.querySelector('[data-a=msub][data-v=modpacks]').click()"); pg.wait_for_timeout(600)
    check(pg.evaluate("state.route") == "modpack", "Mods → Modpacks abre la pantalla de modpacks")
    check(pg.evaluate("!!document.querySelector('[data-a=back]')"), "en Modpacks aparece «Volver»")
    label = pg.evaluate("document.querySelector('[data-a=back]').textContent")
    check("Mods" in label, f"la flecha dice adónde vuelve: «{label.strip()}»")
    pg.evaluate("document.querySelector('[data-a=back]').click()"); pg.wait_for_timeout(600)
    check(pg.evaluate("state.route") == "server" and pg.evaluate("state.tab") == "mods", "Volver regresa a la pestaña Mods del mismo servidor")
    # El menú lateral empieza de cero.
    pg.evaluate("document.querySelector('#nav [data-route=settings]').click()"); pg.wait_for_timeout(500)
    check(pg.evaluate("!document.querySelector('[data-a=back]')"), "entrar por el menú lateral no deja una flecha")
    # Alt+← vuelve.
    pg.evaluate("go('servers', {}, {root:true})"); pg.wait_for_timeout(300)
    pg.evaluate("goSettings('updates')"); pg.wait_for_timeout(400)
    pg.keyboard.press("Alt+ArrowLeft"); pg.wait_for_timeout(500)
    check(pg.evaluate("state.route") == "servers", "Alt+← vuelve a la pantalla anterior")
    # «Iniciar» muestra enseguida «Iniciando servidor…», aunque el backend tarde en contestar.
    pg.evaluate("""() => { const T = window.__MOCK__.T; T.list_servers[1].status = 'stopped';
        window.__TAURI__.core.invoke = ((orig) => async (cmd, a) => cmd === 'start_server' ? new Promise(r => setTimeout(() => r(null), 3000)) : orig(cmd, a))(window.__TAURI__.core.invoke); }""")
    pg.evaluate("go('servers', {}, {root:true})"); pg.wait_for_timeout(400)
    pg.evaluate("document.querySelector('[data-a=start]').click()"); pg.wait_for_timeout(200)
    txt = pg.evaluate("document.querySelector('.list, main').innerText")
    check("Iniciando servidor…" in txt, "al tocar «Iniciar» aparece «Iniciando servidor…» de inmediato")
    pg.evaluate("window.__MOCK__.T.list_servers[1].status = 'starting'; routes.servers()"); pg.wait_for_timeout(300)
    check("Iniciando servidor…" not in pg.evaluate("document.querySelector('main').innerText"), "cuando llega el estado real, se muestra el real (no queda el provisorio)")
    c.close()
    b.close()
for e in errores: print("error de página:", e)
sys.exit(1 if fallas or errores else 0)
