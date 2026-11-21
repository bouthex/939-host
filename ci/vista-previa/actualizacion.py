"""Pantalla de actualización con el backend simulado: aparece sola al abrir solo si hay una versión
nueva, muestra los estados reales en orden sin parpadear, la barra solo con porcentaje real, el
error con «Reintentar», y respeta «menos movimiento». Sin versión nueva no aparece nada.
Uso: python3 ci/vista-previa/actualizacion.py [salida]"""
import json, pathlib, shutil, sys
from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/vista-previa"); OUT.mkdir(parents=True, exist_ok=True)
datos = (ROOT / "ci/vista-previa/datos.json").read_text()
mock = (ROOT / "ci/vista-previa/mock.js").read_text()
chrome = shutil.which("google-chrome") or shutil.which("chromium")
fallos = []
BASE = {"enabled": True, "channel": "estable", "current": "1.0.0", "mode": "instalada", "notes": "", "lastCheck": None, "failed": None, "notice": None}

def st(**kw):
    d = dict(BASE); d.update(kw); return d

def abrir(b, inicial, motion="no-preference", theme="light", install=None):
    ctx = b.new_context(viewport={"width": 1100, "height": 720}, reduced_motion=motion, color_scheme=theme)
    pg = ctx.new_page()
    extra = f"window.__MOCK__.T.update_status = {json.dumps(inicial)};"
    if install is not None: extra += f"window.__MOCK__.T.update_install = () => {{ window.__INSTALAR__ = (window.__INSTALAR__ || 0) + 1; return {json.dumps(install)}; }};"
    pg.add_init_script(f"window.__DATOS__ = {datos};\n{mock}\n{extra}")
    pg.goto((ROOT / "ui/index.html").as_uri())
    pg.wait_for_function("window.__h939Ready", timeout=6000)
    return ctx, pg

def emitir(pg, v): pg.evaluate(f"window.__MOCK__.emit('update-status', {json.dumps(v)})")
def texto(pg): return pg.evaluate("document.getElementById('upd-status')?.textContent || ''")

with sync_playwright() as p:
    b = p.chromium.launch(executable_path=chrome, args=["--allow-file-access-from-files"])
    # 1) Sin versión nueva: no aparece ninguna pantalla.
    ctx, pg = abrir(b, st(status="al-dia", message="939 Host está al día."))
    pg.wait_for_timeout(2800)
    if pg.query_selector("#upd-screen"): fallos.append("sin actualización apareció la pantalla de actualización")
    ctx.close()
    # 2) Versión nueva al abrir: descarga → verificación → instalación → reinicio, en orden.
    for theme in ["light", "dark"]:
        ctx, pg = abrir(b, st(status="descargando", available="1.1.0", progress=0.0, message="Descargando 939 Host 1.1.0…"), theme=theme, install=[])
        pg.wait_for_selector("#upd-screen", timeout=4000)
        vistos = []
        def mirar():
            t = texto(pg)
            if t and (not vistos or vistos[-1] != t): vistos.append(t)
        for pct in [0.12, 0.47, 0.83]:
            emitir(pg, st(status="descargando", available="1.1.0", progress=pct)); pg.wait_for_timeout(450); mirar()
        pg.wait_for_timeout(1400)
        pg.screenshot(path=str(OUT / f"actualizacion-descargando-{theme}.png"))
        barra = pg.evaluate("document.querySelector('.upd-pct')?.textContent")
        if barra != "83 %": fallos.append(f"la barra no muestra el porcentaje real (muestra {barra!r})")
        emitir(pg, st(status="verificando", available="1.1.0", progress=1.0)); pg.wait_for_timeout(100); mirar()
        # Descarga muy rápida: «lista» llega enseguida; igual se ve «Verificando…» un instante.
        emitir(pg, st(status="lista", available="1.1.0")); pg.wait_for_timeout(200); mirar()
        pg.wait_for_function("window.__INSTALAR__ === 1", timeout=3000)
        emitir(pg, st(status="instalando", available="1.1.0")); 
        for _ in range(12): pg.wait_for_timeout(100); mirar()
        pg.screenshot(path=str(OUT / f"actualizacion-instalando-{theme}.png"))
        emitir(pg, st(status="reiniciando", available="1.1.0"))
        for _ in range(12): pg.wait_for_timeout(100); mirar()
        pg.screenshot(path=str(OUT / f"actualizacion-reiniciando-{theme}.png"))
        orden = ["Descargando actualización…", "Verificando archivos…", "Instalando actualización…", "Reiniciando 939 Host…"]
        reales = [t for t in vistos if t in orden]
        if reales != orden: fallos.append(f"[{theme}] estados en otro orden o faltantes: {vistos}")
        ctx.close()
    # 3) Error de verificación: se explica y se puede reintentar o seguir.
    ctx, pg = abrir(b, st(status="descargando", available="1.1.0", progress=0.5))
    pg.wait_for_selector("#upd-screen", timeout=4000)
    emitir(pg, st(status="error", available="1.1.0", message="el archivo descargado no coincide con el firmado (se descartó). Se vuelve a intentar más tarde."))
    pg.wait_for_timeout(1200)
    pg.screenshot(path=str(OUT / "actualizacion-error.png"))
    if "no coincide" not in texto(pg): fallos.append(f"el error no se muestra: {texto(pg)!r}")
    if not pg.query_selector('[data-a="upd-retry"]') or not pg.query_selector('[data-a="upd-later"]'): fallos.append("falta Reintentar o Continuar en el error")
    pg.click('[data-a="upd-later"]'); pg.wait_for_timeout(700)
    if pg.query_selector("#upd-screen"): fallos.append("«Continuar sin actualizar» no cerró la pantalla")
    ctx.close()
    # 4) La versión que ya falló (y se volvió atrás) no se reinstala sola al abrir; se avisa.
    ctx, pg = abrir(b, st(status="lista", available="1.1.0", failed="1.1.0", notice={"kind": "error", "text": "La versión 1.1.0 se cerró al abrir. Se volvió a la versión 1.0.0 y tus datos quedaron intactos.", "version": "1.1.0"}), install=[])
    pg.wait_for_timeout(3000)
    if pg.query_selector("#upd-screen"): fallos.append("una versión que ya falló se volvió a instalar sola")
    if "no se completó" not in (pg.text_content("#modal-root") or ""): fallos.append("no se avisó que la actualización anterior falló")
    pg.screenshot(path=str(OUT / "actualizacion-revertida.png"))
    ctx.close()
    # 5) Con la app en uso: aviso arriba, sin instalar sola.
    ctx, pg = abrir(b, st(status="al-dia"), install=[])
    pg.wait_for_timeout(2600)
    emitir(pg, st(status="lista", available="1.1.0", message="939 Host 1.1.0 está lista para instalar."))
    pg.wait_for_timeout(300)
    if not pg.query_selector(".updbar"): fallos.append("con la app en uso no apareció el aviso «Actualizar ahora»")
    if pg.evaluate("window.__INSTALAR__ || 0"): fallos.append("con la app en uso se instaló sin que nadie lo pidiera")
    pg.screenshot(path=str(OUT / "actualizacion-aviso.png"), clip={"x": 0, "y": 0, "width": 1100, "height": 160})
    ctx.close()
    # 6) Menos movimiento: sin animaciones en la pantalla de actualización.
    ctx, pg = abrir(b, st(status="descargando", available="1.1.0", progress=0.3), motion="reduce")
    pg.wait_for_selector("#upd-screen", timeout=3000)
    pg.wait_for_timeout(400)
    corriendo = pg.evaluate("document.getElementById('upd-screen').getAnimations({ subtree: true }).filter(a => a.playState === 'running').length")
    if corriendo: fallos.append(f"con menos movimiento hay {corriendo} animaciones en la pantalla de actualización")
    ctx.close()
    b.close()
print("\n".join(fallos) if fallos else "actualización (pantalla): sin errores")
sys.exit(1 if fallos else 0)
