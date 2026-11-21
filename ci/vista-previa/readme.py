"""Capturas del README: la interfaz real de 939 Host con datos de ejemplo (backend simulado).
Uso: python3 ci/vista-previa/readme.py   → escribe docs/img/app-*.png"""
import io, pathlib, shutil
from playwright.sync_api import sync_playwright
from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / "docs/img"
datos = (ROOT / "ci/vista-previa/datos.json").read_text()
mock = (ROOT / "ci/vista-previa/mock.js").read_text()

def save(png: bytes, name: str):
    im = Image.open(io.BytesIO(png)).convert("RGB")
    im.save(OUT / name, optimize=True)

def open_app(b, theme):
    ctx = b.new_context(viewport={"width": 1280, "height": 800}, device_scale_factor=1, color_scheme=theme, reduced_motion="reduce")
    pg = ctx.new_page()
    pg.add_init_script(f"window.__DATOS__ = {datos};\n{mock}")
    pg.goto((ROOT / "ui/index.html").as_uri())
    pg.wait_for_function("!document.getElementById('splash')", timeout=8000)
    pg.add_style_tag(content="*{caret-color:transparent}")
    return ctx, pg

with sync_playwright() as p:
    b = p.chromium.launch(executable_path=shutil.which("google-chrome"), args=["--allow-file-access-from-files"])
    for theme, suf in [("light", "claro"), ("dark", "oscuro")]:
        ctx, pg = open_app(b, theme)
        pg.evaluate("go('server', {id:'srv-demo'})"); pg.wait_for_timeout(900)
        save(pg.screenshot(), f"app-resumen-{suf}.png")
        # Jugadores: controles − y + y la ficha.
        pg.click('.tab:has-text("Jugadores")'); pg.wait_for_timeout(1000)
        save(pg.screenshot(), f"app-jugadores-{suf}.png")
        # Inventario en lista, con un ítem elegido.
        pg.click('[data-a="pf-tab"][data-v="inv"]'); pg.wait_for_timeout(600)
        pg.click('[data-a="inv-view"][data-v="list"]'); pg.wait_for_timeout(300)
        pg.click('[data-key="inv:4"]'); pg.wait_for_timeout(500)
        pg.evaluate("document.querySelector('.inv-wrap').scrollIntoView({block:'start'}); window.scrollBy(0,-90); document.querySelector('main, .main, #view')?.scrollBy?.(0,-90)")
        pg.wait_for_timeout(200)
        save(pg.screenshot(), f"app-inventario-{suf}.png")
        # Conexión por IP.
        pg.click('.tab:has-text("Resumen")'); pg.wait_for_timeout(900)
        pg.set_viewport_size({"width": 1280, "height": 1060})
        pg.click('[data-a="ext-open"]'); pg.wait_for_timeout(700)
        pg.evaluate("document.querySelector('.overlay').scrollTop = 0; document.activeElement?.blur()"); pg.wait_for_timeout(150)
        save(pg.screenshot(), f"app-conexion-ip-{suf}.png")
        pg.keyboard.press("Escape"); pg.wait_for_timeout(300)
        pg.set_viewport_size({"width": 1280, "height": 800})
        # Preparar para jugar (resultado).
        pg.click('[data-a="client-prepare"]'); pg.wait_for_timeout(800)
        save(pg.screenshot(), f"app-preparar-{suf}.png")
        ctx.close()
    b.close()
print("capturas en", OUT)
