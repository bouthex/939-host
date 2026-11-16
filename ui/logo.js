/* 939 Host — Logo de cada servidor: diseños propios de 939, subir una imagen con recorte y zoom,
   vista previa en los tamaños reales y volver al ícono genérico. Nunca toca el logo de la app. */

/* Diseños de 939 Host: formas geométricas en blanco y negro sobre fondos funcionales tenues. */
const LOGO_PRESETS = [
  { bg: '#16181b', fg: '#f4f4f2', d: 'M64 30 98 49v38L64 106 30 87V49Z M64 30v38m0 0 34-19M64 68 30 49' },
  { bg: '#e9ebe6', fg: '#16181b', d: 'M64 26 102 64 64 102 26 64Z M64 46 82 64 64 82 46 64Z' },
  { bg: '#1d2a22', fg: '#cfe0d3', d: 'M64 24c18 22 30 34 30 50a30 30 0 0 1-60 0c0-16 12-28 30-50Z' },
  { bg: '#2a2620', fg: '#eadfc8', d: 'M34 94 64 34l30 60Z M50 94l14-28 14 28' },
  { bg: '#1e2530', fg: '#d3dbe6', d: 'M28 64h72M64 28v72M40 40l48 48M88 40 40 88' },
  { bg: '#2b2023', fg: '#ecd5d8', d: 'M64 30a34 34 0 1 0 0 68 34 34 0 1 0 0-68Z M64 48a16 16 0 1 0 0 32 16 16 0 1 0 0-32Z' },
  { bg: '#f1f0ec', fg: '#1b1c1e', d: 'M34 34h60v60H34Z M50 50h28v28H50Z' },
  { bg: '#131416', fg: '#e8e8e4', d: 'M30 82 52 46l14 20 10-14 22 30Z' },
  { bg: '#232622', fg: '#dfe5da', d: 'M64 28v72M64 52c-14 0-24-10-24-24 14 0 24 10 24 24Zm0 20c14 0 24-10 24-24-14 0-24 10-24 24Z' },
  { bg: '#26232b', fg: '#e2dbea', d: 'M64 26l9 27h29l-23 17 9 28-24-17-24 17 9-28-23-17h29Z' },
  { bg: '#e7e9ec', fg: '#1a1d22', d: 'M30 46h68M30 64h68M30 82h68' },
  { bg: '#1b1b1b', fg: '#f2f2f2', d: 'M40 34h48l-24 30 24 30H40l24-30Z' },
];
const presetSVG = (i, size = 128) => {
  const p = LOGO_PRESETS[i] || LOGO_PRESETS[0];
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128" width="${size}" height="${size}"><rect width="128" height="128" rx="28" fill="${p.bg}"/><path d="${p.d}" fill="none" stroke="${p.fg}" stroke-width="7" stroke-linejoin="round" stroke-linecap="round"/></svg>`;
};

/* Logo de un servidor para listas, encabezado y Papelera. */
function srvLogo(url, size = 40) {
  return url ? `<span class="srv-ic logo" style="width:${size}px;height:${size}px"><img src="${url}" alt="" draggable="false"></span>` : `<span class="srv-ic" style="width:${size}px;height:${size}px">${icon('cube', Math.round(size * 0.45))}</span>`;
}

/* PNG cuadrado a partir de un dibujo en canvas. */
function canvasPNG(draw, size) {
  const c = document.createElement('canvas');
  c.width = c.height = size;
  const g = c.getContext('2d');
  g.imageSmoothingQuality = 'high';
  draw(g, size);
  return new Promise((ok) => c.toBlob(async (b) => ok(Array.from(new Uint8Array(await b.arrayBuffer()))), 'image/png'));
}
const logoImg = (src) => new Promise((ok, no) => { const i = new Image(); i.onload = () => ok(i); i.onerror = () => no(new Error('No se pudo leer la imagen.')); i.src = src; });

A['logo-edit'] = async () => {
  const v = state.server;
  const id = v.config.id;
  const ed = { mode: v.config.logo?.startsWith('preset:') ? 'preset' : v.config.logo === 'custom' ? 'keep' : 'preset', preset: Number((v.config.logo || 'preset:0').split(':')[1]) || 0, img: null, zoom: 1, x: 0, y: 0 };
  const view = () => {
    if (ed.mode === 'preset') return `data:image/svg+xml;utf8,${encodeURIComponent(presetSVG(ed.preset, 256))}`;
    if (ed.mode === 'keep') return v.logoUrl;
    return null;
  };
  let saved = false;
  const ok = await dialog({ title: 'Logo del servidor', wide: true, confirm: 'Guardar logo',
    body: `<p class="t2" style="margin:0">Se ve en tu lista, en el panel, en la Papelera y en la lista de servidores de Minecraft. El logo de 939 Host no cambia.</p>
      <div class="logo-ed">
        <div class="stack" style="gap:14px">
          <div class="seg" role="tablist"><button class="on" data-lm="preset">Diseños de 939</button><button data-lm="upload">Subir imagen</button></div>
          <div id="lg-presets" class="logo-grid">${LOGO_PRESETS.map((_, i) => `<button class="lp${i === ed.preset ? ' on' : ''}" data-lp="${i}" aria-label="Diseño ${i + 1}">${presetSVG(i, 56)}</button>`).join('')}</div>
          <div id="lg-upload" hidden>
            <label class="drop" id="lg-drop"><input type="file" accept="image/png,image/jpeg,image/webp" hidden id="lg-file">${icon('upload', 20)}<b>Elegí o arrastrá una imagen</b><span class="sm t3">PNG, JPG o WebP · se recorta en un cuadrado</span></label>
            <div id="lg-crop" hidden><div class="crop"><canvas id="lg-cv" width="256" height="256"></canvas><span class="crop-mask"></span></div>
              <label class="field">Zoom<input type="range" id="lg-zoom" min="1" max="4" step="0.01" value="1"></label><span class="hint">Arrastrá la imagen para encuadrarla.</span></div>
          </div>
        </div>
        <div class="logo-prev"><span class="xs t3">Vista previa</span><div class="row" style="gap:14px;align-items:flex-end"><span id="pv-lg" class="pv" style="width:96px;height:96px"></span><span id="pv-md" class="pv" style="width:40px;height:40px"></span><span id="pv-mc" class="pv mc" style="width:32px;height:32px"></span></div>
          <span class="xs t3">Panel · lista · Minecraft</span></div>
      </div>`,
    buttons: [...(v.config.logo ? [{ label: 'Volver al ícono genérico', value: 'clear' }] : []), { label: 'Guardar logo', value: true, kind: 'pri' }],
    onMount: (d) => {
      const cv = d.querySelector('#lg-cv');
      const paint = () => {
        let src = view();
        if (ed.mode === 'upload' && ed.img) {
          const g = cv.getContext('2d');
          const s = Math.max(256 / ed.img.width, 256 / ed.img.height) * ed.zoom;
          const w = ed.img.width * s, h = ed.img.height * s;
          ed.x = Math.min(0, Math.max(256 - w, ed.x)); ed.y = Math.min(0, Math.max(256 - h, ed.y));
          g.clearRect(0, 0, 256, 256); g.drawImage(ed.img, ed.x, ed.y, w, h);
          src = cv.toDataURL('image/png');
        }
        d.querySelectorAll('.pv').forEach((p) => { p.innerHTML = src ? `<img src="${src}" alt="">` : icon('cube', 18); });
      };
      d.querySelectorAll('[data-lm]').forEach((b) => b.addEventListener('click', () => {
        d.querySelectorAll('[data-lm]').forEach((x) => x.classList.toggle('on', x === b));
        const up = b.dataset.lm === 'upload';
        d.querySelector('#lg-presets').hidden = up; d.querySelector('#lg-upload').hidden = !up;
        ed.mode = up ? (ed.img ? 'upload' : 'none') : 'preset';
        paint();
      }));
      d.querySelectorAll('[data-lp]').forEach((b) => b.addEventListener('click', () => {
        ed.preset = Number(b.dataset.lp); ed.mode = 'preset';
        d.querySelectorAll('.lp').forEach((x) => x.classList.toggle('on', x === b)); paint();
      }));
      const take = async (file) => {
        if (!file) return;
        if (!/^image\/(png|jpeg|webp)$/.test(file.type)) { toast('Elegí una imagen PNG, JPG o WebP.', 'error'); return; }
        if (file.size > 20 * 1024 * 1024) { toast('La imagen pesa más de 20 MB.', 'error'); return; }
        try {
          const url = URL.createObjectURL(file);
          ed.img = await logoImg(url); URL.revokeObjectURL(url);
          if (ed.img.width < 64 || ed.img.height < 64) { ed.img = null; toast('La imagen es muy chica (mínimo 64×64).', 'error'); return; }
          ed.mode = 'upload'; ed.zoom = 1; ed.x = -1e6; ed.y = -1e6;
          const s = Math.max(256 / ed.img.width, 256 / ed.img.height);
          ed.x = (256 - ed.img.width * s) / 2; ed.y = (256 - ed.img.height * s) / 2;
          d.querySelector('#lg-zoom').value = 1;
          d.querySelector('#lg-drop').hidden = true; d.querySelector('#lg-crop').hidden = false; paint();
        } catch (e) { fail(e); }
      };
      d.querySelector('#lg-file').addEventListener('change', (e) => take(e.target.files[0]));
      const drop = d.querySelector('#lg-drop');
      drop.addEventListener('dragover', (e) => { e.preventDefault(); drop.classList.add('over'); });
      drop.addEventListener('dragleave', () => drop.classList.remove('over'));
      drop.addEventListener('drop', (e) => { e.preventDefault(); drop.classList.remove('over'); take(e.dataTransfer.files[0]); });
      d.querySelector('#lg-zoom').addEventListener('input', (e) => {
        const z0 = ed.zoom; ed.zoom = Number(e.target.value);
        ed.x = 128 - (128 - ed.x) * ed.zoom / z0; ed.y = 128 - (128 - ed.y) * ed.zoom / z0; paint();
      });
      let drag = null;
      cv.addEventListener('pointerdown', (e) => { drag = { x: e.clientX - ed.x, y: e.clientY - ed.y }; cv.setPointerCapture(e.pointerId); });
      cv.addEventListener('pointermove', (e) => { if (drag) { ed.x = e.clientX - drag.x; ed.y = e.clientY - drag.y; paint(); } });
      cv.addEventListener('pointerup', () => { drag = null; });
      paint();
      saved = () => ({ cv });
    } });
  if (ok === 'clear') {
    if (!(await dialog({ title: 'Volver al ícono genérico', confirm: 'Quitar logo', body: '<p>Se quita el logo de este servidor (también de la lista de Minecraft).</p>' }))) return;
    try { await invoke('server_logo_clear', { id }); toast('Logo quitado'); } catch (e) { fail(e); }
    refreshServer(); return;
  }
  if (!ok || ed.mode === 'keep' || ed.mode === 'none') return;
  try {
    let source;
    if (ed.mode === 'preset') source = await logoImg(`data:image/svg+xml;utf8,${encodeURIComponent(presetSVG(ed.preset, 256))}`);
    else source = await logoImg(saved().cv.toDataURL('image/png'));
    const big = await canvasPNG((g, s) => g.drawImage(source, 0, 0, s, s), 256);
    const small = await canvasPNG((g, s) => g.drawImage(source, 0, 0, s, s), 64);
    await invoke('server_logo_set', { id, kind: ed.mode === 'preset' ? `preset:${ed.preset}` : 'custom', big, icon: small });
    toast(isActive(v) ? 'Logo guardado. En Minecraft se ve tras reiniciar el servidor.' : 'Logo guardado');
  } catch (e) { fail(e); }
  refreshServer();
};
