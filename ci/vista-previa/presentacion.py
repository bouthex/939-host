"""Animación inicial: cuadros en distintos momentos, estado final idéntico con «menos movimiento»
y comprobación de que no retrasa el arranque (la pantalla queda usable y el splash se va).
Uso: python3 ci/vista-previa/presentacion.py [salida]"""
import pathlib, shutil, sys
from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/vista-previa"); OUT.mkdir(parents=True, exist_ok=True)
datos = (ROOT / "ci/vista-previa/datos.json").read_text()
mock = (ROOT / "ci/vista-previa/mock.js").read_text()
chrome = shutil.which("google-chrome") or shutil.which("chromium")
fallos = []
with sync_playwright() as p:
    b = p.chromium.launch(executable_path=chrome, args=["--allow-file-access-from-files"])
    for theme in ["light", "dark"]:
        # Sin boot.js el splash queda fijo: sirve para ver la animación cuadro por cuadro.
        for motion in ["no-preference", "reduce"]:
            ctx = b.new_context(viewport={"width": 1024, "height": 680}, reduced_motion=motion, color_scheme=theme)
            pg = ctx.new_page()
            pg.route("**/boot.js", lambda r: r.fulfill(body="", content_type="text/javascript"))
            pg.goto((ROOT / "ui/index.html").as_uri())
            prev = 0
            for t in ([60, 200, 420, 1100] if motion == "no-preference" else [0]):
                pg.wait_for_timeout(t - prev); prev = t
                pg.screenshot(path=str(OUT / f"splash-{theme}-{motion}-{t}.png"), clip={"x": 312, "y": 240, "width": 400, "height": 200})
                if motion == "reduce":
                    op = pg.eval_on_selector_all(".sp-cube", "els => els.map(e => getComputedStyle(e).opacity + '|' + getComputedStyle(e).transform)")
                    if any(o != "1|none" for o in op): fallos.append(f"con menos movimiento los cuadros no están quietos: {op}")
            ctx.close()
    # Con la app real (simulada): el splash se va en cuanto la pantalla está lista.
    ctx = b.new_context(viewport={"width": 1280, "height": 820})
    pg = ctx.new_page(); pg.add_init_script(f"window.__DATOS__ = {datos};\n{mock}")
    pg.goto((ROOT / "ui/index.html").as_uri())
    pg.wait_for_function("window.__h939Ready", timeout=5000)
    pg.wait_for_timeout(450)
    if pg.query_selector("#splash"): fallos.append("el splash sigue en pantalla después de cargar")
    ctx.close()
    b.close()
print("\n".join(fallos) if fallos else "presentación: sin errores")
sys.exit(1 if fallos else 0)
