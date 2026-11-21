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
                    corriendo = pg.evaluate("document.getElementById('splash').getAnimations({ subtree: true }).filter(a => a.playState === 'running').length")
                    if corriendo: fallos.append(f"con menos movimiento hay {corriendo} animaciones en la presentación")
            ctx.close()
    # Con la app (backend simulado): la app queda usable enseguida (la presentación no frena la
    # carga), la presentación se ve al menos ~2,3 s y se va sola; con menos movimiento, enseguida.
    for motion, minimo, maximo in [("no-preference", 2.0, 3.4), ("reduce", 0.0, 1.0)]:
        ctx = b.new_context(viewport={"width": 1280, "height": 820}, reduced_motion=motion)
        pg = ctx.new_page(); pg.add_init_script(f"window.__DATOS__ = {datos};\n{mock}")
        pg.goto((ROOT / "ui/index.html").as_uri())
        pg.wait_for_function("window.__h939Ready", timeout=5000)
        listo = pg.evaluate("(window.__h939Ready - performance.timeOrigin) / 1000")
        if listo > 2.0: fallos.append(f"la app tardó {listo:.1f} s en quedar usable")
        pg.wait_for_function("!document.getElementById('splash')", timeout=6000)
        fin = pg.evaluate("performance.now() / 1000")
        if not (minimo <= fin <= maximo): fallos.append(f"con movimiento «{motion}» la presentación duró {fin:.1f} s (esperado {minimo}–{maximo})")
        ctx.close()
    b.close()
print("\n".join(fallos) if fallos else "presentación: sin errores")
sys.exit(1 if fallos else 0)
