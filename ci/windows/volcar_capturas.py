"""Imprime las capturas como JPEG reducidos en base64 dentro del registro de la ejecución,
para revisarlas sin descargar artefactos. Uso: python volcar_capturas.py capturas"""
import base64, io, os, sys
from PIL import Image
d = sys.argv[1]
for f in sorted(os.listdir(d)):
    if not f.endswith(".png"):
        continue
    im = Image.open(os.path.join(d, f)).convert("RGB")
    im.thumbnail((1280, 1280))
    b = io.BytesIO(); im.save(b, "JPEG", quality=72)
    s = base64.b64encode(b.getvalue()).decode()
    print(f"@@IMG {f} {len(s)}")
    for i in range(0, len(s), 4000):
        print("@@" + s[i:i + 4000])
    print("@@END")
