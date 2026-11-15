"""Prueba entre dos máquinas reales de GitHub (Windows y Linux), en redes separadas.

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
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.p.stdout:
            line = line.rstrip("\n")
            if line.startswith("@@ "):
                self.q.put(line[3:])
            elif line.startswith("## "):
                print("    · " + line[3:], flush=True)

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
        except Exception:
            pass
        time.sleep(2)
    raise TimeoutError(what)


def anfitriona():
    n = Node("Windows")
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
    n = Node("Linux")
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
    t0 = time.time()
    n.ok(f"request {sid}")
    log(True, "pidió ser anfitriona")
    wait_until("esta PC queda alojando", 1500, lambda: n.ok(f"online {sid}"))
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
json.dump(results, open(f"resultado-{ROLE}.json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)
sys.exit(0 if results and all(r["ok"] for r in results) else 1)
