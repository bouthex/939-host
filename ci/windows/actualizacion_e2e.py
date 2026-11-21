"""Actualización automática real en Windows (CI), en un entorno aislado.

Usa la app instalada por los pasos anteriores (compilación de prueba 1.1.0), una distribución local
en 127.0.0.1 firmada con una clave de prueba generada en la CI (nunca la real) y el instalador y la
portable de esta misma compilación. La app se hace pasar por 1.0.0 (solo existe en la compilación de
prueba) para recibir 1.1.0 como actualización.

Escenarios:
  A  instalada: detecta, descarga, verifica, instala, reinicia y la versión nueva confirma; datos intactos
  B  paquete corrupto: se rechaza y no se toca nada
  C  sin conexión: la app abre normal, sin pantalla de actualización
  D  la versión nueva no abre: vuelta atrás automática a la anterior, aviso, sin reintento solo; reintento manual
  F  actualización cortada (apagado de Windows): se informa y se puede reintentar
  E  portable: se actualiza en su carpeta
  H  portable en una carpeta sin permiso de escritura: se explica y no se toca nada
"""
import http.server, json, os, pathlib, shutil, subprocess, sys, threading, time, winreg, zipfile
import psutil
from playwright.sync_api import sync_playwright

sys.stdout.reconfigure(encoding="utf-8")
LOCAL = pathlib.Path(os.environ["LOCALAPPDATA"])
INST = LOCAL / "Programs" / "939 Host"
EXE = INST / "939Host.exe"
DATA = LOCAL / "939Host"
STATE = DATA / "actualizacion" / "estado.json"
WORK = pathlib.Path("act-e2e").resolve()
OUT = pathlib.Path(os.environ.get("OUT", "capturas-actualizacion")).resolve()
DIST = pathlib.Path("dist").resolve()
PRUEBA = pathlib.Path("dist-prueba").resolve()
PORT, DEAD, CDP = 8939, 8941, 9333
WORK.mkdir(exist_ok=True); OUT.mkdir(exist_ok=True)
results = []


def log(ok, what, extra=""):
    results.append(ok)
    print(("✔ " if ok else "✘ ") + what + (f" — {extra}" if extra else ""), flush=True)


# ---- Distribución local -----------------------------------------------------------------------
FILES = {p.name: p.read_bytes() for p in list(PRUEBA.glob("*.json")) + list(DIST.glob("939Host-*-instalador.exe")) + list(DIST.glob("939Host-*-portable.zip"))}
MODE = {"corrupto": False}


class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        name = self.path.lstrip("/")
        b = FILES.get(name)
        if b is None:
            self.send_response(404); self.end_headers(); return
        if MODE["corrupto"] and (name.endswith(".exe") or name.endswith(".zip")):
            b = bytearray(b); b[len(b) // 2] ^= 0xFF; b = bytes(b)
        self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)

    def log_message(self, *a): pass


srv = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), H)
threading.Thread(target=srv.serve_forever, daemon=True).start()
dist_json = json.loads((PRUEBA / "distribucion.json").read_text())
(WORK / "dist.json").write_text(json.dumps(dist_json))
(WORK / "dist-sin-conexion.json").write_text(json.dumps({**dist_json, "url": f"http://127.0.0.1:{DEAD}"}))
NEW = json.loads(__import__("base64").b64decode(json.loads(FILES["estable.json"])["payload"]))["version"]


# ---- Utilidades -------------------------------------------------------------------------------
def procs(name="939Host.exe"):
    out = []
    for p in psutil.process_iter(["name", "cmdline", "exe", "create_time"]):
        try:
            if (p.info["name"] or "").lower() == name.lower(): out.append(p)
        except psutil.Error: pass
    return out


def kill_all():
    for n in ["939Host.exe", "939Host-vigia.exe"]:
        subprocess.run(["taskkill", "/F", "/T", "/IM", n], capture_output=True)
    time.sleep(3)


def reset():
    kill_all()
    for p in [STATE, DATA / "actualizacion" / "registro.txt"]:
        p.unlink(missing_ok=True)
    shutil.rmtree(DATA / "cache" / "actualizaciones", ignore_errors=True)


def env(extra=None, version="1.0.0", dist="dist.json"):
    e = dict(os.environ)
    e["H939_ACTUALIZACIONES_PRUEBA"] = str(WORK / dist)
    if version: e["H939_PRUEBA_VERSION_INSTALADA"] = version
    e["H939_E2E_DEVTOOLS_PORT"] = str(CDP)
    e.update(extra or {})
    return e


def start(exe, e):
    return subprocess.Popen([str(exe)], env=e, cwd=str(exe.parent))


def state():
    try: return json.loads(STATE.read_text(encoding="utf-8"))
    except Exception: return None


def wait(pred, timeout, step=0.5):
    end = time.time() + timeout
    while time.time() < end:
        v = pred()
        if v: return v
        time.sleep(step)
    return None


def page(pw, timeout=60):
    end = time.time() + timeout
    while time.time() < end:
        try:
            b = pw.chromium.connect_over_cdp(f"http://127.0.0.1:{CDP}")
            for c in b.contexts:
                for p in c.pages:
                    if p.url.startswith("http://tauri.localhost") or p.url.startswith("tauri://") or "index.html" in p.url:
                        return b, p
            b.close()
        except Exception:
            pass
        time.sleep(1)
    return None, None


def js(p, expr, default=None):
    try: return p.evaluate(expr)
    except Exception: return default


def data_fingerprint():
    worlds = sorted(DATA.glob("servers/*/server/world/level.dat"))
    marker = None
    if worlds:
        marker = worlds[0].parent / "939-marcador-actualizacion.txt"
        if not marker.exists(): marker.write_text("datos de la persona")
    fp = {}
    for f in [*(worlds[:1]), marker, DATA / "identity" / "node.key"]:
        if f and f.exists(): fp[str(f)] = vigia_sha(f)
    try: fp["consoleFontSize"] = json.loads((DATA / "settings.json").read_text(encoding="utf-8-sig")).get("consoleFontSize")
    except Exception: pass
    fp["servidores"] = len(list(DATA.glob("servers/*/server.json")) or list(DATA.glob("servers/*")))
    return fp


def vigia_sha(p):
    import hashlib
    return hashlib.sha256(p.read_bytes()).hexdigest()


def registry_version():
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Uninstall\939Host") as k:
            return winreg.QueryValueEx(k, "DisplayVersion")[0]
    except OSError:
        return None


def running_with(flag, exe_dir):
    for p in procs():
        try:
            if flag in (p.info["cmdline"] or []) and pathlib.Path(p.info["exe"]).parent.resolve() == exe_dir.resolve(): return p
        except Exception: pass
    return None


def registro():
    try: return (DATA / "actualizacion" / "registro.txt").read_text(encoding="utf-8")
    except Exception: return ""


# ---- Escenarios -------------------------------------------------------------------------------
with sync_playwright() as pw:
    # A) Instalada: todo el camino, con capturas de la pantalla real.
    reset()
    before = data_fingerprint()
    t0 = time.time()
    start(EXE, env())
    b, p = page(pw)
    seen = []
    if p:
        if wait(lambda: js(p, "!!document.getElementById('upd-screen')"), 40):
            shot = 0
            while time.time() - t0 < 120:
                t = js(p, "document.getElementById('upd-status')?.textContent || ''", None)
                if t is None: break
                if t and (not seen or seen[-1] != t):
                    seen.append(t)
                    if shot < 4:
                        try: p.screenshot(path=str(OUT / f"actualizacion-windows-{shot}.png")); shot += 1
                        except Exception: pass
                time.sleep(0.15)
        try: b.close()
        except Exception: pass
    log(bool(seen), "A: al abrir apareció sola la pantalla de actualización", " → ".join(seen))
    log("Reiniciando 939 Host…" in seen and any("Descargando" in s or "Verificando" in s for s in seen), "A: estados reales en orden (descarga/verificación → instalación → reinicio)")
    st = wait(lambda: (s := state()) and s.get("phase") in ("ok", "revertida", "fallida") and s, 200)
    log(bool(st) and st["phase"] == "ok", "A: la versión nueva abrió y confirmó", json.dumps(st, ensure_ascii=False)[:400] if st else "sin estado")
    log(bool(st) and st.get("from") == "1.0.0" and st.get("to") == NEW and st.get("mode") == "instalada", f"A: 1.0.0 → {NEW} (instalada)")
    log("instalador terminó: exit code: 0" in registro(), "A: el instalador verificado se ejecutó y terminó bien")
    log(bool(running_with("--tras-actualizar", INST)), "A: 939 Host quedó abierto con la versión nueva")
    log(registry_version() == NEW, f"A: Programas instalados muestra {NEW}", str(registry_version()))
    after = data_fingerprint()
    log(before == after, "A: servidores, mundo, identidad y preferencias intactos", json.dumps({k: (before.get(k) == after.get(k)) for k in before}, ensure_ascii=False))
    b, p = page(pw, 20)
    if p:
        time.sleep(3)
        try: p.screenshot(path=str(OUT / "actualizacion-windows-completada.png"))
        except Exception: pass
        b.close()
    log(time.time() - t0 < 240, f"A: todo el proceso tardó {time.time() - t0:.0f} s")

    # B) Paquete corrupto: se rechaza y no se toca nada.
    reset()
    MODE["corrupto"] = True
    mtime = EXE.stat().st_mtime
    app = start(EXE, env())
    b, p = page(pw)
    msg = wait(lambda: p and (t := js(p, "document.getElementById('upd-status')?.textContent || ''")) and "coincide" in t and t, 60) if p else None
    if p:
        try: p.screenshot(path=str(OUT / "actualizacion-windows-corrupto.png"))
        except Exception: pass
        b.close()
    log(bool(msg), "B: paquete alterado rechazado por SHA-256", msg or "")
    log(app.poll() is None and state() is None and EXE.stat().st_mtime == mtime, "B: la app siguió abierta y no se cambió nada")
    MODE["corrupto"] = False

    # C) Sin conexión: abre normal, sin pantalla.
    reset()
    app = start(EXE, env(dist="dist-sin-conexion.json"))
    b, p = page(pw)
    ready = wait(lambda: p and js(p, "!!window.__h939Ready"), 30) if p else None
    time.sleep(6)
    screen = js(p, "!!document.getElementById('upd-screen')") if p else None
    status = js(p, "window.__TAURI__.core.invoke('update_status').then(v => v.status)") if p else None
    if p: b.close()
    log(bool(ready) and not screen and app.poll() is None, "C: sin conexión la app abre normal, sin pantalla de actualización", f"estado: {status}")

    # D) La versión nueva no abre: vuelta atrás automática.
    reset()
    leeme = INST / "LEEME-INSTALACION.txt"
    leeme.write_text("marca de la versión anterior", encoding="utf-8")
    start(EXE, env({"H939_PRUEBA_FALLAR_TRAS_ACTUALIZAR": "1"}))
    st = wait(lambda: (s := state()) and s.get("phase") in ("ok", "revertida", "fallida") and s, 240)
    log(bool(st) and st["phase"] == "revertida", "D: la versión nueva no abrió y se volvió atrás sola", (st or {}).get("message", "")[:300])
    log(leeme.read_text(encoding="utf-8") == "marca de la versión anterior", "D: los archivos anteriores quedaron restaurados")
    log(registry_version() == "1.0.0", "D: Programas instalados vuelve a mostrar la versión anterior", str(registry_version()))
    log(bool(wait(lambda: running_with("--tras-revertir", INST), 30)), "D: se volvió a abrir la versión anterior")
    b, p = page(pw)
    dlg = wait(lambda: p and "no se completó" in (js(p, "document.getElementById('modal-root')?.textContent || ''") or ""), 30) if p else None
    if p:
        try: p.screenshot(path=str(OUT / "actualizacion-windows-revertida.png"))
        except Exception: pass
        b.close()
    log(bool(dlg), "D: se avisa qué pasó y que los datos están intactos")
    # Al volver a abrir, esa versión no se reinstala sola.
    kill_all()
    start(EXE, env())
    time.sleep(25)
    log((state() or {}).get("phase") == "revertida", "D: al abrir de nuevo no se reintenta sola la versión que falló")
    # Reintento manual: ahora la versión nueva abre bien.
    b, p = page(pw)
    if p:
        wait(lambda: js(p, "window.__TAURI__.core.invoke('update_status').then(v => v.status)") == "lista", 60)
        js(p, "A['upd-now']()")
        b.close()
    st = wait(lambda: (s := state()) and s.get("phase") == "ok" and s, 200)
    log(bool(st), "D: «Reintentar» instala la versión y abre bien")

    # F) Actualización cortada (por ejemplo, se apagó la PC a mitad): se informa.
    reset()
    STATE.parent.mkdir(parents=True, exist_ok=True)
    STATE.write_text(json.dumps({"from": NEW, "to": "9.9.9", "mode": "instalada", "appDir": str(INST), "package": "x", "files": [], "phase": "instalando", "message": "", "at": "2026-01-01T00:00:00-03:00", "shown": True}))
    start(EXE, env(version=None, dist="dist-sin-conexion.json"))
    b, p = page(pw)
    dlg = wait(lambda: p and "interrumpió" in (js(p, "document.getElementById('modal-root')?.textContent || ''") or ""), 40) if p else None
    if p: b.close()
    log((state() or {}).get("phase") == "fallida" and bool(dlg), "F: una actualización cortada se informa y se puede reintentar")

    # E) Portable: se actualiza en su propia carpeta.
    reset()
    port = WORK / "portable"
    shutil.rmtree(port, ignore_errors=True)
    with zipfile.ZipFile(next(DIST.glob("939Host-*-portable.zip"))) as z: z.extractall(port)
    pm = (port / "939Host.exe").stat().st_mtime
    time.sleep(2)
    start(port / "939Host.exe", env())
    st = wait(lambda: (s := state()) and s.get("phase") in ("ok", "revertida", "fallida") and s, 200)
    log(bool(st) and st["phase"] == "ok" and st.get("mode") == "portable", "E: portable actualizada y abierta", json.dumps(st, ensure_ascii=False)[:300] if st else "")
    log((port / "939Host.exe").stat().st_mtime != pm and bool(running_with("--tras-actualizar", port)), "E: reemplazó sus archivos y abrió desde su carpeta")
    log(not list(port.glob("*.nuevo")), "E: sin archivos a medio escribir")

    # H) Portable en una carpeta sin permiso de escritura: se explica, no se toca nada.
    reset()
    ro = WORK / "portable-solo-lectura"
    shutil.rmtree(ro, ignore_errors=True)
    shutil.copytree(port, ro)
    user = os.environ.get("USERNAME")
    subprocess.run(["icacls", str(ro), "/deny", f"{user}:(WD,AD)"], capture_output=True)
    app = start(ro / "939Host.exe", env())
    b, p = page(pw)
    msg = wait(lambda: p and (t := js(p, "document.getElementById('upd-status')?.textContent || ''")) and "escribir" in t and t, 90) if p else None
    if p:
        try: p.screenshot(path=str(OUT / "actualizacion-windows-sin-permiso.png"))
        except Exception: pass
        b.close()
    log(bool(msg) and app.poll() is None and state() is None, "H: carpeta sin permiso: se explica y no se cambia nada", msg or "")
    kill_all()
    subprocess.run(["icacls", str(ro), "/remove:d", user], capture_output=True)

kill_all()
srv.shutdown()
ok = all(results)
print(f"\n{sum(results)}/{len(results)} comprobaciones de actualización correctas")
sys.exit(0 if ok else 1)
