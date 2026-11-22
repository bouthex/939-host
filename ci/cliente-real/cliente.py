"""Descarga el cliente oficial de Minecraft desde Mojang y entra a un servidor, como un jugador.

Uso: python cliente.py <version> <host:puerto> <carpeta> <nombre> [segundos]

Todo queda en <carpeta> (aislada). Cuenta sin conexión (el servidor debe tener online-mode=false).
Se usa en CI con una pantalla virtual (xvfb) y OpenGL por software (Mesa). Escribe el registro del
juego en <carpeta>/game/logs/latest.log y termina cuando pasa el tiempo indicado.
"""
import concurrent.futures, hashlib, json, os, platform, subprocess, sys, time, urllib.request, uuid, zipfile

ver, addr, root, name = sys.argv[1:5]
secs = int(sys.argv[5]) if len(sys.argv) > 5 else 90
os.makedirs(root, exist_ok=True)
OSN = {"Linux": "linux", "Windows": "windows", "Darwin": "osx"}[platform.system()]


def get(url, path=None, sha1=None):
    if path and os.path.exists(path) and (not sha1 or hashlib.sha1(open(path, "rb").read()).hexdigest() == sha1):
        return path
    for i in range(4):
        try:
            data = urllib.request.urlopen(url, timeout=60).read()
            break
        except Exception:
            if i == 3: raise
            time.sleep(2)
    if sha1 and hashlib.sha1(data).hexdigest() != sha1:
        raise SystemExit(f"hash distinto: {url}")
    if path is None:
        return data
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, "wb").write(data)
    return path


def allowed(rules):
    if not rules: return True
    ok = False
    for r in rules:
        if "features" in r: continue
        if "os" in r and r["os"].get("name") not in (None, OSN): continue
        ok = r["action"] == "allow"
    return ok


man = json.loads(get("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"))
entry = next(v for v in man["versions"] if v["id"] == ver)
vj = json.loads(get(entry["url"], sha1=entry["sha1"]))
java = vj.get("javaVersion", {}).get("majorVersion")
print(f"Minecraft {ver} · Java {java}", flush=True)
client = get(vj["downloads"]["client"]["url"], f"{root}/versions/{ver}.jar", vj["downloads"]["client"]["sha1"])
cp, natives = [], f"{root}/natives"
os.makedirs(natives, exist_ok=True)
jobs = []
for lib in vj["libraries"]:
    if not allowed(lib.get("rules")): continue
    d = lib.get("downloads", {})
    if "artifact" in d:
        a = d["artifact"]; p = f"{root}/libraries/{a['path']}"
        jobs.append((a["url"], p, a["sha1"])); cp.append(p)
    cl = lib.get("natives", {}).get(OSN)
    if cl and cl in d.get("classifiers", {}):
        a = d["classifiers"][cl]; p = f"{root}/libraries/{a['path']}"
        jobs.append((a["url"], p, a["sha1"]))
        lib["_native"] = p
idx = vj["assetIndex"]
index_path = get(idx["url"], f"{root}/assets/indexes/{idx['id']}.json", idx["sha1"])
index = json.load(open(index_path))
for o in index["objects"].values():
    h = o["hash"]
    jobs.append((f"https://resources.download.minecraft.net/{h[:2]}/{h}", f"{root}/assets/objects/{h[:2]}/{h}", h))
t0 = time.time()
with concurrent.futures.ThreadPoolExecutor(32) as ex:
    list(ex.map(lambda j: get(*j), jobs))
print(f"descargados {len(jobs)} archivos en {time.time() - t0:.0f} s", flush=True)
for lib in vj["libraries"]:
    if "_native" in lib:
        with zipfile.ZipFile(lib["_native"]) as z:
            for n in z.namelist():
                if not n.startswith("META-INF") and not n.endswith("/"):
                    z.extract(n, natives)
cp.append(client)
offline = uuid.UUID(bytes=hashlib.md5(f"OfflinePlayer:{name}".encode()).digest(), version=3)
host, port = addr.rsplit(":", 1)
game = f"{root}/game"
os.makedirs(game, exist_ok=True)
# Opciones mínimas: sin música, sin tutorial, pantalla chica.
opts = "onboardAccessibility:false\ntutorialStep:none\nsoundCategory_master:0.0\nrenderDistance:2\nsimulationDistance:5\nmaxFps:30\njoinedFirstServer:true\nskipMultiplayerWarning:true\n"
# Resource pack opcional ya instalado en el perfil del jugador (como lo deja «Preparar para jugar»).
if os.environ.get("PACK_OPCIONAL"):
    import shutil
    os.makedirs(f"{game}/resourcepacks", exist_ok=True)
    shutil.copy(os.environ["PACK_OPCIONAL"], f"{game}/resourcepacks/opcional.zip")
    opts += 'resourcePacks:["vanilla","file/opcional.zip"]\n'
open(f"{game}/options.txt", "w").write(opts)
# Lista de servidores con la elección de «Resource packs del servidor» para esta dirección:
# ACEPTAR_PACKS=1 → aceptar sin preguntar (como un jugador que eligió «Activados»); 0 → «Desactivados».
if os.environ.get("ACEPTAR_PACKS") in ("0", "1"):
    def s(t): b = t.encode(); return len(b).to_bytes(2, "big") + b
    e = b"\x08" + s("ip") + s(addr) + b"\x08" + s("name") + s("Prueba") + b"\x01" + s("acceptTextures") + bytes([int(os.environ["ACEPTAR_PACKS"])]) + b"\x00"
    open(f"{game}/servers.dat", "wb").write(b"\x0a\x00\x00\x09" + s("servers") + b"\x0a" + (1).to_bytes(4, "big") + e + b"\x00")
java_bin = os.environ.get("JAVA_BIN", "java")
args = [java_bin, "-Xmx2G", f"-Djava.library.path={natives}", "-Dorg.lwjgl.util.Debug=false", "-cp", os.pathsep.join(cp), vj["mainClass"],
        "--username", name, "--version", ver, "--gameDir", game, "--assetsDir", f"{root}/assets", "--assetIndex", idx["id"],
        "--uuid", offline.hex, "--accessToken", "0", "--userType", "legacy", "--width", "854", "--height", "480",
        "--quickPlayMultiplayer", f"{host}:{port}"]
print("abriendo el juego…", flush=True)
p = subprocess.Popen(args, stdout=open(f"{root}/salida.txt", "w"), stderr=subprocess.STDOUT)
end = time.time() + secs
while time.time() < end and p.poll() is None:
    time.sleep(2)
if p.poll() is None:
    p.terminate()
    try: p.wait(20)
    except Exception: p.kill()
print(f"el juego terminó (código {p.returncode})", flush=True)
