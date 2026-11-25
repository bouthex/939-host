"""Prueba entre dos máquinas Windows reales y separadas de GitHub.

Cada máquina corre un nodo `939node` con relays habilitados como la app, y servidores de
Minecraft reales descargados por 939 Host. Se intercambian la invitación y las señales con
archivos que el flujo de CI pasa como artefactos.

  anfitriona: crea el servidor, lo comparte, invita, lo enciende, marca el mundo y espera.
              Acepta la solicitud que llegue (flujo real «Solicitar ser anfitrión»).
  invitada:   se une, entra por su ruta local a través de P2P, pide ser anfitriona, recibe el
              servidor con el mundo intacto y la anfitriona vieja entra ahora por su propia ruta.
"""
import json, os, subprocess, sys, time, threading, queue

ROLE = sys.argv[1]           # anfitriona | invitada
SIG = sys.argv[2]            # carpeta de señales (entrada/salida)
NODE = os.environ["NODE_BIN"]
ROOT = os.path.abspath(f"pc-{ROLE}")
sys.stdout.reconfigure(encoding="utf-8", line_buffering=True)
os.makedirs(SIG, exist_ok=True)
results = []


def log(ok, what):
    results.append({"ok": ok, "paso": what})
    print(("✔ " if ok else "✘ ") + what, flush=True)


class Node:
    def __init__(self, name):
        args = [NODE, "--root", ROOT, "--name", name] + ([] if os.environ.get("PC_SIN_RELAYS") else ["--relays"])
        if os.environ.get("PC_FAKE"): args.append("--fake")
        self.p = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.STDOUT, text=True, encoding="utf-8", env={**os.environ})
        self.q = queue.Queue()
        self.notes = []
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.p.stdout:
            line = line.rstrip("\n")
            if line.startswith("@@ "):
                self.q.put(line[3:])
            elif line.startswith("## "):
                self.notes.append(line[3:])
                print("    · " + line[3:], flush=True)

    def failed_handover(self):
        return next((x for x in self.notes if "no se pudo completar" in x), None)

    def cmd(self, c, timeout=900):
        self.p.stdin.write(c + "\n"); self.p.stdin.flush()
        r = json.loads(self.q.get(timeout=timeout))
        return r

    def ok(self, c, timeout=900):
        r = self.cmd(c, timeout)
        if not r.get("ok"):
            raise RuntimeError(f"{c}: {r.get('err')}")
        return r.get("v")


def slp(port, tries=15):
    import socket, struct
    def varint(v):
        out = b""
        while True:
            b = v & 0x7F; v >>= 7
            if v == 0: return out + bytes([b])
            out += bytes([b | 0x80])
    last = ""
    for _ in range(tries):
        try:
            s = socket.create_connection(("127.0.0.1", port), timeout=10)
            body = b"\x00" + varint(767) + varint(9) + b"127.0.0.1" + struct.pack(">H", port) + varint(1)
            s.sendall(varint(len(body)) + body + b"\x01\x00")
            data = b""
            while len(data) < 4096:
                ch = s.recv(4096)
                if not ch: break
                data += ch
                if b"}" in ch: break
            s.close()
            t = data.decode("utf-8", "replace")
            if '"version"' in t or "description" in t:
                return t
            last = t or "(vacío)"
        except Exception as e:
            last = str(e)
        time.sleep(2)
    return "SIN RESPUESTA: " + last


def wait_file(name, timeout):
    path = os.path.join(SIG, name)
    end = time.time() + timeout
    while time.time() < end:
        if os.path.exists(path):
            return open(path, encoding="utf-8").read().strip()
        time.sleep(3)
    raise TimeoutError(f"no llegó {name}")


def put_file(name, text):
    open(os.path.join(SIG, name), "w", encoding="utf-8").write(text)


def wait_until(what, timeout, f):
    end = time.time() + timeout
    while time.time() < end:
        try:
            if f(): return True
        except RuntimeError:
            raise
        except Exception:
            pass
        time.sleep(2)
    raise TimeoutError(what)


def anfitriona():
    n = Node("Anfitriona")
    sid = n.ok("create Casas939 25811"); log(True, f"servidor creado {sid}")
    n.ok(f"share {sid}")
    tok = n.ok(f"invite {sid} coanfitrion")
    put_file("invitacion.txt", f"{sid}\n{tok}")
    log(True, "invitación generada")
    t0 = time.time(); n.ok(f"start {sid}", timeout=1200)
    log(True, f"servidor real encendido en esta PC ({time.time() - t0:.0f} s, incluye descargas)")
    if not os.environ.get("PC_FAKE"):
        n.ok(f"cmd {sid} scoreboard objectives add casas dummy")
        n.ok(f"cmd {sid} scoreboard players set marcador casas 939")
    put_file("encendido.txt", "1")
    # Solicitud real de la otra PC → se acepta (reutiliza el traspaso en dos fases).
    wait_until("llega la solicitud", 900, lambda: n.ok(f"status {sid}").get("incomingRequest"))
    names = n.ok(f"lsdir {sid} config/desde-invitada")
    log("subida.bin" in names and not any("cortado" in x or ".939-subida" in x for x in names), f"en esta PC quedó lo subido y nada a medias: {names}")
    stt = n.ok(f"status {sid}")
    inc = stt["incomingRequest"]
    mem = next((m for m in stt.get("members", []) if m.get("nodeId") == inc.get("nodeId")), {})
    inc["path"], inc["rttMs"] = mem.get("path"), mem.get("rttMs")
    log(True, f"llegó la solicitud de {inc.get('name')} · conexión {inc.get('path')} · {inc.get('rttMs')} ms · avisos {inc.get('warnings')}")
    n.ok(f"answer {sid} si", timeout=1200)
    log(True, "solicitud aceptada; traspaso hecho")
    wait_until("esta PC queda apagada", 120, lambda: not n.ok(f"running {sid}"))
    log(True, "esta PC dejó de alojar (un solo escritor)")
    route = n.ok(f"route {sid} 41912")
    st = slp(route)
    log('"version"' in st or "description" in st, f"la PC vieja entra por su dirección fija y llega al servidor nuevo: {st[:120]}")
    put_file("anfitriona-fin.txt", json.dumps(results))
    wait_file("invitada-fin.txt", 1800)
    n.cmd("exit", 30)


def invitada():
    sid, tok = wait_file("invitacion.txt", 1800).splitlines()
    n = Node("Invitada")
    t0 = time.time(); n.ok(f"join {tok}", timeout=300)
    log(True, f"se unió al grupo por P2P en {time.time() - t0:.1f} s")
    n.ok(f"setport {sid} 25812")
    route = n.ok(f"route {sid} 41911")
    wait_file("encendido.txt", 1800)
    st = slp(route)
    log('"version"' in st or "description" in st, f"entra por 127.0.0.1:{route} y llega al servidor en la otra PC: {st[:120]}")
    stt = n.ok(f"status {sid}")
    via = [m for m in stt.get("members", []) if m.get("online")]
    print("    miembros:", json.dumps(via)[:400])
    # Administración remota de archivos entre dos PCs reales (la anfitriona está en otra región).
    import hashlib
    blob = os.path.join(ROOT, "subida.bin")
    os.makedirs(ROOT, exist_ok=True)
    with open(blob, "wb") as f: f.write(bytes((i * 7) % 251 for i in range(8 * 1024 * 1024 + 77)))
    h = hashlib.sha1(open(blob, "rb").read()).hexdigest()
    ls = n.ok(f"fls {sid}")
    log(any(e["name"] == "world" and e["locked"] for e in ls), f"lista los archivos de la otra PC ({len(ls)} entradas; el mundo protegido porque está encendido)")
    n.ok(f"fmkdir {sid} config/desde-invitada")
    t0 = time.time(); n.ok(f"fput {sid} {blob} config/desde-invitada/subida.bin", timeout=600)
    up = time.time() - t0
    back = os.path.join(ROOT, "bajada.bin")
    t0 = time.time(); n.ok(f"fget {sid} config/desde-invitada/subida.bin {back}", timeout=600)
    down = time.time() - t0
    log(hashlib.sha1(open(back, "rb").read()).hexdigest() == h, f"8 MB subidos en {up:.1f} s y bajados en {down:.1f} s con el mismo SHA-1")
    r = n.cmd(f"frm {sid} world/level.dat")
    log(not r.get("ok") and "encendido" in (r.get("err") or ""), f"no deja borrar el mundo con el servidor encendido: {str(r.get('err'))[:90]}")
    r = n.cmd(f"fputcancel {sid} {blob} config/desde-invitada/cortado.bin", timeout=600)
    log(not r.get("ok"), "subida cancelada a mitad de camino")
    t0 = time.time()
    n.ok(f"request {sid}")
    log(True, "pidió ser anfitriona")
    def alojando():
        bad = n.failed_handover()
        if bad: raise RuntimeError(bad)
        return n.ok(f"online {sid}")
    try:
        wait_until("esta PC queda alojando", 1500, alojando)
    except TimeoutError:
        raise RuntimeError(n.failed_handover() or "no quedó alojando")
    log(True, f"recibió el servidor y está en línea ({time.time() - t0:.0f} s desde la solicitud, incluye descargas)")
    if not os.environ.get("PC_FAKE"):
        n.ok(f"cmd {sid} scoreboard players get marcador casas")
    time.sleep(3)
    logp = os.path.join(ROOT, "Servers", sid, "server", "logs", "latest.log")
    txt = open(logp, encoding="utf-8", errors="replace").read() if os.path.exists(logp) else ""
    log("marcador has 939" in txt, "el mundo llegó intacto (marcador = 939)")
    wait_file("anfitriona-fin.txt", 900)
    n.ok(f"stop {sid}", timeout=180)
    put_file("invitada-fin.txt", json.dumps(results))
    n.cmd("exit", 30)


try:
    anfitriona() if ROLE == "anfitriona" else invitada()
except Exception as e:
    log(False, f"{type(e).__name__}: {e}")
    # Avisar a la otra PC para que no espere de más.
    put_file(f"{ROLE}-fin.txt", json.dumps(results))
json.dump(results, open(f"resultado-{ROLE}.json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)
sys.exit(0 if results and all(r["ok"] for r in results) else 1)
