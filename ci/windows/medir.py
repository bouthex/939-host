"""Mediciones reales de recursos de 939 Host en Windows (CI): memoria del proceso y de su
WebView2, CPU en reposo, arranque, tamaño instalado y de datos. Se llama desde ui_e2e.py."""
import os, time, json
import psutil


def tree():
    """939Host.exe y sus procesos hijos (WebView2 y el guardián), sin Java (se mide aparte)."""
    roots = [p for p in psutil.process_iter(["name"]) if (p.info["name"] or "").lower() == "939host.exe" and not _is_guardian(p)]
    out = []
    for r in roots:
        out.append(r)
        try: out += [c for c in r.children(recursive=True) if (c.name() or "").lower() not in ("java.exe", "javaw.exe")]
        except psutil.Error: pass
    return out


def memory():
    """MB: memoria privada (lo que la app ocupa de verdad) y conjunto de trabajo (en RAM ahora)."""
    priv = ws = 0
    n = 0
    for p in tree():
        try:
            m = p.memory_full_info() if hasattr(p, "memory_full_info") else p.memory_info()
            priv += getattr(m, "private", getattr(m, "uss", 0)); ws += m.rss; n += 1
        except psutil.Error: pass
    return {"privada_mb": round(priv / 1048576, 1), "en_ram_mb": round(ws / 1048576, 1), "procesos": n}


def cpu(seconds=10):
    """% de un núcleo promedio durante `seconds` (sumando la app y su WebView2)."""
    ps = tree()
    t0 = {}
    for p in ps:
        try: c = p.cpu_times(); t0[p.pid] = c.user + c.system
        except psutil.Error: pass
    time.sleep(seconds)
    used = 0.0
    for p in ps:
        try: c = p.cpu_times(); used += (c.user + c.system) - t0.get(p.pid, c.user + c.system)
        except psutil.Error: pass
    return round(used / seconds * 100, 2)


def _is_guardian(p):
    try: return "--guardian" in (p.cmdline() or [])
    except psutil.Error: return False


def processes():
    """Cuántos procesos hay de 939 Host (sin contar el guardián), del guardián, de Java y de WebView2."""
    out = {}
    for p in psutil.process_iter(["name"]):
        n = (p.info["name"] or "").lower().removesuffix(".exe")
        key = {"939host": "939Host", "java": "java", "javaw": "java", "msedgewebview2": "msedgewebview2"}.get(n)
        if key == "939Host" and _is_guardian(p): key = "guardian"
        if key: out[key] = out.get(key, 0) + 1
    return out


def main_process():
    """El proceso principal de 939 Host (no el guardián)."""
    for p in psutil.process_iter(["name"]):
        if (p.info["name"] or "").lower() == "939host.exe" and not _is_guardian(p):
            return p
    return None


def memory_of(kind):
    """MB de memoria privada de un tipo de proceso: "guardian" | "java"."""
    total = 0
    for p in psutil.process_iter(["name"]):
        n = (p.info["name"] or "").lower()
        hit = (kind == "guardian" and n == "939host.exe" and _is_guardian(p)) or (kind == "java" and n in ("java.exe", "javaw.exe"))
        if not hit: continue
        try:
            m = p.memory_full_info() if hasattr(p, "memory_full_info") else p.memory_info()
            total += getattr(m, "private", getattr(m, "uss", 0))
        except psutil.Error: pass
    return round(total / 1048576, 1)


def started_at():
    r = [p for p in psutil.process_iter(["name", "create_time"]) if (p.info["name"] or "").lower() == "939host.exe"]
    return min(p.info["create_time"] for p in r) if r else None


def dir_mb(path):
    total = 0
    for root, _, files in os.walk(path):
        for f in files:
            try: total += os.path.getsize(os.path.join(root, f))
            except OSError: pass
    return round(total / 1048576, 1)


def sizes():
    local = os.environ.get("LOCALAPPDATA", "")
    inst = os.path.join(local, "Programs", "939 Host")
    data = os.path.join(local, "939Host")
    out = {"instalado_mb": dir_mb(inst), "exe_mb": round(os.path.getsize(os.path.join(inst, "939Host.exe")) / 1048576, 1) if os.path.exists(os.path.join(inst, "939Host.exe")) else None}
    if os.path.isdir(data):
        out["datos_total_mb"] = dir_mb(data)
        for sub in ["cache", "java", "servers", "Papelera"]:
            p = os.path.join(data, sub)
            if os.path.isdir(p): out[f"datos_{sub.lower()}_mb"] = dir_mb(p)
    return out
