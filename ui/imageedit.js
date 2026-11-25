'use strict';
/* 939 Host — Imágenes elegidas por la persona (logo de servidor y foto de perfil).

   Por qué fallaba antes:
   - «Elegir» usaba URL.createObjectURL (blob:), que la política de seguridad de la ventana no
     permite para imágenes: daba «No se pudo leer la imagen». Ahora se lee como data: con FileReader.
   - «Arrastrar» esperaba el evento del navegador, pero la ventana de 939 Host recibe los archivos
     arrastrados como evento nativo (con la ruta). Ahora se escucha ese evento mientras el editor
     está abierto y el backend lee y valida la imagen (tipo real por contenido, peso máximo).
   Lo que sale de acá es siempre un PNG generado por canvas: no viaja el archivo original. */

const IMG_MAX_BYTES = 20 * 1024 * 1024;
const IMG_TYPES = /^image\/(png|jpeg|webp|gif)$/;

function imgFromDataURL(src) {
  return new Promise((ok, no) => {
    const i = new Image();
    i.onload = () => (i.naturalWidth ? ok(i) : no(new Error('La imagen está vacía o dañada.')));
    i.onerror = () => no(new Error('El archivo no es una imagen válida o está dañado.'));
    i.src = src;
  });
}

/** Valida y carga un File de <input type=file>. */
async function readImageFile(file) {
  if (!file) throw new Error('No se eligió ningún archivo.');
  if (file.type && !IMG_TYPES.test(file.type)) throw new Error('Elegí una imagen PNG, JPG, WebP o GIF.');
  if (file.size > IMG_MAX_BYTES) throw new Error('La imagen pesa más de 20 MB.');
  if (!file.size) throw new Error('El archivo está vacío.');
  const url = await new Promise((ok, no) => { const r = new FileReader(); r.onload = () => ok(r.result); r.onerror = () => no(new Error('No se pudo leer el archivo.')); r.readAsDataURL(file); });
  if (!/^data:image\/(png|jpeg|webp|gif);/.test(url)) throw new Error('Elegí una imagen PNG, JPG, WebP o GIF.');
  return checkDims(await imgFromDataURL(url));
}

/** Carga una imagen arrastrada (ruta de la PC): el backend la lee y valida. */
async function readImagePath(path) {
  const url = await invoke('image_read', { path });
  return checkDims(await imgFromDataURL(url));
}

function checkDims(img, min = 32) {
  if (img.naturalWidth < min || img.naturalHeight < min) throw new Error(`La imagen es muy chica (mínimo ${min}×${min} píxeles).`);
  if (img.naturalWidth > 12000 || img.naturalHeight > 12000) throw new Error('La imagen es demasiado grande (máximo 12000 píxeles por lado).');
  return img;
}

/** Mientras el editor está abierto, los archivos arrastrados a la ventana van a `onPath`. */
async function listenImageDrop(onPath, onOver) {
  const offs = await Promise.all([
    listen('tauri://drag-enter', () => onOver(true)),
    listen('tauri://drag-leave', () => onOver(false)),
    listen('tauri://drag-drop', (ev) => { onOver(false); const p = (ev.payload?.paths || [])[0]; if (p) onPath(p); }),
  ]);
  return () => offs.forEach((f) => f());
}

/** PNG cuadrado (bytes) dibujando `source` (canvas o imagen) al tamaño pedido. */
function squarePNG(source, size, { pixelated = false } = {}) {
  const c = document.createElement('canvas');
  c.width = c.height = size;
  const g = c.getContext('2d');
  g.imageSmoothingEnabled = !pixelated;
  g.imageSmoothingQuality = 'high';
  g.drawImage(source, 0, 0, size, size);
  return new Promise((ok) => c.toBlob(async (b) => ok(Array.from(new Uint8Array(await b.arrayBuffer()))), 'image/png'));
}

/** Copia escalada en un canvas (para las vistas previas fieles al tamaño real). */
function scaledCanvas(source, size) {
  const c = document.createElement('canvas');
  c.width = c.height = size;
  const g = c.getContext('2d');
  g.imageSmoothingQuality = 'high';
  g.drawImage(source, 0, 0, size, size);
  return c;
}

/**
 * Recorte cuadrado con arrastre y zoom. `root` contiene un <canvas data-crop> y un
 * <input type=range data-zoom>. `onChange()` se llama después de cada cambio.
 */
function mountCropper(root, onChange, size = 256) {
  const cv = root.querySelector('[data-crop]');
  const zoom = root.querySelector('[data-zoom]');
  const st = { img: null, zoom: 1, x: 0, y: 0 };
  const paint = () => {
    const g = cv.getContext('2d');
    g.clearRect(0, 0, size, size);
    if (!st.img) return;
    const s = Math.max(size / st.img.naturalWidth, size / st.img.naturalHeight) * st.zoom;
    const w = st.img.naturalWidth * s, h = st.img.naturalHeight * s;
    st.x = Math.min(0, Math.max(size - w, st.x)); st.y = Math.min(0, Math.max(size - h, st.y));
    g.imageSmoothingQuality = 'high';
    g.drawImage(st.img, st.x, st.y, w, h);
    onChange();
  };
  zoom.addEventListener('input', () => {
    const z0 = st.zoom; st.zoom = Number(zoom.value);
    const c = size / 2;
    st.x = c - (c - st.x) * st.zoom / z0; st.y = c - (c - st.y) * st.zoom / z0; paint();
  });
  let drag = null;
  cv.addEventListener('pointerdown', (e) => { if (!st.img) return; drag = { x: e.clientX - st.x, y: e.clientY - st.y }; cv.setPointerCapture(e.pointerId); });
  cv.addEventListener('pointermove', (e) => { if (drag) { st.x = e.clientX - drag.x; st.y = e.clientY - drag.y; paint(); } });
  cv.addEventListener('pointerup', () => { drag = null; });
  cv.addEventListener('keydown', (e) => {
    const step = e.shiftKey ? 16 : 4;
    const k = { ArrowLeft: [step, 0], ArrowRight: [-step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] }[e.key];
    if (k && st.img) { e.preventDefault(); st.x += k[0]; st.y += k[1]; paint(); }
  });
  return {
    set(img) {
      st.img = img; st.zoom = 1; zoom.value = 1;
      const s = Math.max(size / img.naturalWidth, size / img.naturalHeight);
      st.x = (size - img.naturalWidth * s) / 2; st.y = (size - img.naturalHeight * s) / 2;
      paint();
    },
    has: () => !!st.img,
    canvas: cv,
  };
}

/** Bloque HTML de «subir imagen» (zona para elegir o arrastrar + recorte). */
function uploadBlock(prefix, { round = false } = {}) {
  return `<div class="stack" style="gap:12px">
    <label class="drop" id="${prefix}-drop"><input type="file" accept="image/png,image/jpeg,image/webp,image/gif" hidden id="${prefix}-file">${icon('upload', 20)}<b>Elegí una imagen o arrastrala a esta ventana</b><span class="sm t3">PNG, JPG, WebP o GIF · hasta 20 MB · desde cualquier carpeta</span></label>
    <div id="${prefix}-crop" hidden class="stack" style="gap:8px"><div class="crop${round ? ' round' : ''}"><canvas data-crop width="256" height="256" tabindex="0" aria-label="Recorte: arrastrá o usá las flechas para encuadrar"></canvas><span class="crop-mask"></span></div>
      <label class="field">Zoom<input type="range" data-zoom min="1" max="4" step="0.01" value="1"></label>
      <div class="row" style="gap:10px"><span class="hint grow">Arrastrá la imagen para encuadrarla.</span><button class="btn s ghost" type="button" id="${prefix}-other">Elegir otra</button></div></div>
  </div>`;
}

/**
 * Conecta un bloque de «subir imagen» dentro de un diálogo. `onImage()` se llama al cargar una
 * imagen válida. Devuelve { cropper, stop } (stop deja de escuchar el arrastre).
 */
async function wireUpload(d, prefix, onChange, onImage = () => {}) {
  const cropper = mountCropper(d.querySelector(`#${prefix}-crop`), onChange);
  const drop = d.querySelector(`#${prefix}-drop`);
  const show = (img) => { cropper.set(img); drop.hidden = true; d.querySelector(`#${prefix}-crop`).hidden = false; onImage(); };
  const take = async (loader) => {
    drop.setAttribute('aria-busy', 'true');
    try { show(await loader()); } catch (e) { toast(errMsg(e), 'error'); } finally { drop.removeAttribute('aria-busy'); }
  };
  d.querySelector(`#${prefix}-file`).addEventListener('change', (e) => { const f = e.target.files[0]; e.target.value = ''; take(() => readImageFile(f)); });
  d.querySelector(`#${prefix}-other`).addEventListener('click', () => d.querySelector(`#${prefix}-file`).click());
  const stop = await listenImageDrop((path) => take(() => readImagePath(path)), (on) => drop.classList.toggle('over', on));
  return { cropper, stop };
}

/* ── Fotos de perfil de las PCs del grupo (por huella, en caché en memoria) ── */
const AVATARS = {};
const avatarQueue = new Set();
let avatarTimer = null;
function queueAvatar(hash) {
  if (!hash || AVATARS[hash] !== undefined || avatarQueue.has(hash)) return;
  avatarQueue.add(hash);
  clearTimeout(avatarTimer);
  avatarTimer = setTimeout(async () => {
    const list = [...avatarQueue]; avatarQueue.clear();
    const got = await invoke('avatars', { hashes: list }).catch(() => ({}));
    for (const h of list) {
      AVATARS[h] = got[h] || null;
      if (!got[h]) continue;
      document.querySelectorAll(`[data-avh="${h}"]`).forEach((el) => { el.innerHTML = `<img src="${got[h]}" alt="">`; el.classList.add('photo'); });
    }
  }, 30);
}
