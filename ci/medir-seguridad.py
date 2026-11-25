#!/usr/bin/env python3
"""Mide el costo de la seguridad con tres «PCs» de prueba (939node, servidores simulados).

Uso: python3 ci/medir-seguridad.py RUTA_A_939node [salida.json]

Mide: tiempo de arranque del nodo, unión al grupo, RAM y CPU en reposo (60 s), traspaso del
anfitrión, sincronización de 64 MB y latencia de una consulta de control. Corre igual contra la
versión anterior y la nueva para comparar. Solo usa localhost y datos ficticios.
"""
import json, os, subprocess, sys, tempfile, threading, time, queue

BIN = sys.argv[1]
OUT = sys.argv[2] if len(sys.argv) > 2 else None


class Node:
    def __init__(self, name, root, port_hint):
        env = dict(os.environ, H939_FAST="1")
        env.pop("H939_DEBUG", None)
        t0 = time.time()
        self.p = subprocess.Popen([BIN, "--root", root, "--name", name, "--fake"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, bufsize=1, env=env)
        self.q = queue.Queue()
        self.ready = threading.Event()
        threading.Thread(target=self._read, daemon=True).start()
        self.ready.wait(60)
        self.start_s = time.time() - t0
        self.name = name

    def _read(self):
        for line in self.p.stdout:
            if line.startswith("## listo"):
                self.ready.set()
            if line.startswith("@@"):
                self.q.put(json.loads(line[3:]))

    def cmd(self, c, timeout=600):
        self.p.stdin.write(c + "\n"); self.p.stdin.flush()
        r = self.q.get(timeout=timeout)
        if not r["ok"]:
            raise RuntimeError(f"{self.name}: {c}: {r.get('err')}")
        return r["v"]

    def rss_cpu(self):
        with open(f"/proc/{self.p.pid}/status") as f:
            rss = next(int(l.split()[1]) for l in f if l.startswith("VmRSS")) / 1024
        with open(f"/proc/{self.p.pid}/stat") as f:
            parts = f.read().split(")")[1].split()
        ticks = int(parts[11]) + int(parts[12])
        return rss, ticks / os.sysconf("SC_CLK_TCK")

    def close(self):
        try: self.p.kill()
        except Exception: pass


def wait(f, t=60):
    end = time.time() + t
    while time.time() < end:
        if f(): return True
        time.sleep(0.2)
    raise TimeoutError


def main():
    res = {}
    tmp = tempfile.mkdtemp(prefix="medir-")
    a = Node("A", f"{tmp}/a", 0); b = Node("B", f"{tmp}/b", 0); c = Node("C", f"{tmp}/c", 0)
    res["arranque_nodo_s"] = round(sum(n.start_s for n in (a, b, c)) / 3, 3)
    try:
        sid = a.cmd("create Medir 26101")
        a.cmd(f"share {sid}")
        t = a.cmd(f"invite {sid} coanfitrion")
        t0 = time.time(); b.cmd(f"join {t}"); res["union_s"] = round(time.time() - t0, 3)
        b.cmd(f"setport {sid} 26102")
        t = a.cmd(f"invite {sid} jugador"); c.cmd(f"join {t}")
        a.cmd(f"start {sid}")
        # 64 MB de datos de mundo (aleatorios: no se comprimen ni deduplican).
        world = f"{tmp}/a/Servers/{sid}/server/world"
        with open(f"{world}/region-prueba.bin", "wb") as f: f.write(os.urandom(64 * 1024 * 1024))
        t0 = time.time()
        s1 = a.cmd(f"snapshot {sid}")
        res["punto_64mb_s"] = round(time.time() - t0, 3)
        key = (s1["epoch"], s1["seq"])
        t0 = time.time()
        wait(lambda: (lambda m: m and (m["epoch"], m["seq"]) == key)(b.cmd(f"status {sid}")["myLatest"]), 120)
        res["sincronizar_64mb_s"] = round(time.time() - t0, 3)
        # Reposo.
        time.sleep(5)
        before = [n.rss_cpu() for n in (a, b, c)]
        time.sleep(60)
        after = [n.rss_cpu() for n in (a, b, c)]
        res["ram_reposo_mb"] = [round(x[0], 1) for x in after]
        res["cpu_reposo_pct"] = [round((y[1] - x[1]) / 60 * 100, 2) for x, y in zip(before, after)]
        # Latencia de una consulta de control (status de B incluye consultas reales cada 0,7 s).
        lat = []
        for _ in range(20):
            t0 = time.time(); b.cmd(f"sync {sid}"); lat.append(time.time() - t0)
        res["vuelta_sync_ms_mediana"] = round(sorted(lat)[len(lat) // 2] * 1000, 1)
        # Traspaso.
        ib = b.cmd("id")
        t0 = time.time(); a.cmd(f"handover {sid} {ib}")
        wait(lambda: b.cmd(f"running {sid}"), 120)
        res["traspaso_s"] = round(time.time() - t0, 3)
    finally:
        for n in (a, b, c): n.close()
    print(json.dumps(res, indent=2, ensure_ascii=False))
    if OUT:
        with open(OUT, "w") as f: json.dump(res, f, indent=2, ensure_ascii=False)


main()
