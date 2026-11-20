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

/* Diseños con las skins del grupo: se arman en esta PC a partir de la skin oficial de cada cuenta
   (Mojang, textures.minecraft.net), con la cara y la capa del casco en píxeles nítidos sobre el
   mismo fondo sobrio que los demás diseños. Al guardar queda como imagen del servidor: no vuelve a
   pedir nada a Mojang y viaja al grupo como cualquier logo. */
const SKIN_PRESETS = ['bouthex', 'Archimak', 'lescano8'];
const skinFaces = {};
async function skinFace(name) {
  if (skinFaces[name] !== undefined) return skinFaces[name];
  const url = await invoke('player_skin', { name }).catch(() => null);
  if (!url) return (skinFaces[name] = null);
  try {
    const img = await imgFromDataURL(url);
    const c = document.createElement('canvas');
    c.width = c.height = 256;
    const g = c.getContext('2d');
    g.fillStyle = '#16181b';
    g.beginPath(); g.roundRect(0, 0, 256, 256, 56); g.fill();
    g.imageSmoothingEnabled = false;
    const f = 144, o = (256 - f) / 2;
    g.fillStyle = 'rgba(0,0,0,.35)';
    g.beginPath(); g.roundRect(o + 6, o + 10, f, f, 10); g.fill();
    g.save(); g.beginPath(); g.roundRect(o, o, f, f, 10); g.clip();
    g.drawImage(img, 8, 8, 8, 8, o, o, f, f);
    // Capa del casco (segunda capa de la cabeza), si la skin la tiene.
    if (img.naturalHeight >= 64 || img.naturalWidth >= 64) g.drawImage(img, 40, 8, 8, 8, o, o, f, f);
    g.restore();
    g.strokeStyle = 'rgba(255,255,255,.14)'; g.lineWidth = 2;
    g.beginPath(); g.roundRect(o, o, f, f, 10); g.stroke();
    return (skinFaces[name] = c);
  } catch { return (skinFaces[name] = null); }
}

A['logo-edit'] = async () => {
  const v = state.server;
  const id = v.config.id;
  const cur = v.config.logo || '';
  const ed = { mode: cur === 'custom' ? 'keep' : cur.startsWith('skin:') ? 'skin' : cur.startsWith('preset:') ? 'preset' : '939', preset: Number((cur.startsWith('preset:') ? cur : 'preset:0').split(':')[1]) || 0, skin: cur.startsWith('skin:') ? cur.slice(5) : null, up: null, tab: 'preset' };
  let presetImgs = [];
  let img939 = null;
  const final = () => {
    if (ed.mode === '939') return img939;
    if (ed.mode === 'preset') return presetImgs[ed.preset] || null;
    if (ed.mode === 'skin') return skinFaces[ed.skin] || null;
    if (ed.mode === 'upload' && ed.up?.cropper.has()) return ed.up.cropper.canvas;
    return null;
  };
  const ok = await dialog({ title: 'Logo del servidor', wide: true,
    body: `<p class="t2" style="margin:0">Se ve en tu lista, en el panel, en la Papelera y en la lista de servidores de Minecraft. Es el mismo en todas las PCs del grupo, aloje quien aloje. El logo de 939 Host no cambia.</p>
      <div class="logo-ed">
        <div class="stack" style="gap:14px">
          <div class="seg" role="tablist"><button class="on" data-lm="preset" role="tab">Diseños</button><button data-lm="upload" role="tab">Subir imagen</button></div>
          <div id="lg-presets" class="stack" style="gap:10px">
            <div class="logo-grid"><button class="lp${ed.mode === '939' ? ' on' : ''}" data-l939 aria-label="Logo de 939 Host" title="Logo de 939 Host"><img src="logo-939.png" alt="" style="width:56px;height:56px;border-radius:12px"></button>${LOGO_PRESETS.map((_, i) => `<button class="lp${ed.mode === 'preset' && i === ed.preset ? ' on' : ''}" data-lp="${i}" aria-label="Diseño ${i + 1}">${presetSVG(i, 56)}</button>`).join('')}</div>
            <span class="xs t3">Skins del grupo</span>
            <div class="logo-grid">${SKIN_PRESETS.map((n) => `<button class="lp skin${ed.mode === 'skin' && ed.skin === n ? ' on' : ''}" data-ls="${esc(n)}" aria-label="Skin de ${esc(n)}" disabled><span class="bar indet" style="width:40px"><i></i></span><span class="lp-name">${esc(n)}</span></button>`).join('')}</div>
            <span class="xs t3" id="lg-skin-note"></span>
          </div>
          <div id="lg-upload" hidden>${uploadBlock('lg')}</div>
          ${ed.mode === 'keep' ? '<span class="xs t3" id="lg-keep">Ahora tiene una imagen propia. Elegí un diseño o subí otra imagen para cambiarla.</span>' : ''}
        </div>
        <div class="logo-prev"><span class="xs t3">Vista previa real</span>
          <div class="row" style="gap:16px;align-items:flex-end;flex-wrap:wrap">
            <span class="stack" style="gap:4px;align-items:center"><span id="pv-lg" class="pv" style="width:96px;height:96px"></span><span class="xs t3">Panel</span></span>
            <span class="stack" style="gap:4px;align-items:center"><span id="pv-md" class="srv-ic logo pv-list" style="width:40px;height:40px"></span><span class="xs t3">Lista</span></span>
            <span class="stack" style="gap:4px;align-items:center"><span id="pv-mc" class="pv mc" style="width:64px;height:64px"></span><span class="xs t3">Minecraft (64×64)</span></span>
          </div></div>
      </div>`,
    buttons: [...(v.config.logo ? [{ label: 'Volver al ícono genérico', value: 'clear' }] : []), { label: 'Guardar logo', value: true, kind: 'pri' }],
    onMount: async (d) => {
      const paint = () => {
        const src = final();
        const keep = ed.mode === 'keep' && v.logoUrl;
        const big = src ? scaledCanvas(src, 256).toDataURL('image/png') : keep ? v.logoUrl : null;
        const mc = src ? scaledCanvas(src, 64).toDataURL('image/png') : keep ? v.logoUrl : null;
        d.querySelector('#pv-lg').innerHTML = big ? `<img src="${big}" alt="">` : icon('cube', 28);
        d.querySelector('#pv-md').innerHTML = big ? `<img src="${big}" alt="">` : icon('cube', 18);
        d.querySelector('#pv-mc').innerHTML = mc ? `<img src="${mc}" alt="">` : icon('cube', 22);
      };
      img939 = await imgFromDataURL('logo-939.png').catch(() => null);
      presetImgs = await Promise.all(LOGO_PRESETS.map((_, i) => imgFromDataURL(`data:image/svg+xml;utf8,${encodeURIComponent(presetSVG(i, 256))}`).catch(() => null)));
      const sel = (btn) => { d.querySelectorAll('.lp').forEach((x) => x.classList.toggle('on', x === btn)); d.querySelector('#lg-keep')?.remove(); paint(); };
      d.querySelectorAll('[data-lm]').forEach((b) => b.addEventListener('click', () => {
        d.querySelectorAll('[data-lm]').forEach((x) => { x.classList.toggle('on', x === b); x.setAttribute('aria-selected', x === b); });
        ed.tab = b.dataset.lm;
        d.querySelector('#lg-presets').hidden = ed.tab !== 'preset';
        d.querySelector('#lg-upload').hidden = ed.tab !== 'upload';
        if (ed.tab === 'upload') ed.mode = ed.up?.cropper.has() ? 'upload' : ed.mode;
        paint();
      }));
      d.querySelectorAll('[data-lp]').forEach((b) => b.addEventListener('click', () => { ed.preset = Number(b.dataset.lp); ed.mode = 'preset'; sel(b); }));
      d.querySelector('[data-l939]').addEventListener('click', (e) => { ed.mode = '939'; sel(e.currentTarget); });
      d.querySelectorAll('[data-ls]').forEach((b) => b.addEventListener('click', () => { ed.skin = b.dataset.ls; ed.mode = 'skin'; sel(b); }));
      ed.up = await wireUpload(d, 'lg', () => { if (ed.mode === 'upload') paint(); }, () => { ed.mode = 'upload'; d.querySelectorAll('.lp').forEach((x) => x.classList.remove('on')); paint(); });
      paint();
      let missing = 0;
      await Promise.all(SKIN_PRESETS.map(async (n) => {
        const c = await skinFace(n);
        const b = d.querySelector(`[data-ls="${n}"]`);
        if (!b) return;
        if (c) { b.disabled = false; b.innerHTML = `<img src="${scaledCanvas(c, 112).toDataURL('image/png')}" alt="" style="width:56px;height:56px;border-radius:12px"><span class="lp-name">${esc(n)}</span>`; }
        else { missing += 1; b.innerHTML = `${icon('user', 20)}<span class="lp-name">${esc(n)}</span>`; b.title = 'No disponible'; }
        if (ed.mode === 'skin' && ed.skin === n) paint();
      }));
      if (missing) d.querySelector('#lg-skin-note').textContent = state.info.settings.skinsEnabled === false
        ? 'Para usar las skins activá «Mostrar skins reales» en Configuración → Privacidad (consulta a Mojang).'
        : 'Algunas skins no se pudieron obtener de Mojang (sin conexión o cuenta no encontrada).';
    } });
  ed.up?.stop?.();
  if (ok === 'clear') {
    if (!(await dialog({ title: 'Volver al ícono genérico', confirm: 'Quitar logo', body: '<p>Se quita el logo de este servidor (también de la lista de Minecraft).</p>' }))) return;
    try { await invoke('server_logo_clear', { id }); toast('Logo quitado'); } catch (e) { fail(e); }
    refreshServer(); return;
  }
  const src = final();
  if (!ok || !src) return;
  try {
    if (ed.mode === '939') {
      await invoke('server_logo_default', { id });
      toast(isActive(v) ? 'Logo guardado. En Minecraft se ve tras reiniciar el servidor.' : 'Logo guardado');
      refreshServer(); return;
    }
    const big = await squarePNG(src, 256);
    const small = await squarePNG(src, 64, { pixelated: ed.mode === 'skin' });
    const kind = ed.mode === 'preset' ? `preset:${ed.preset}` : ed.mode === 'skin' ? `skin:${ed.skin}` : 'custom';
    await invoke('server_logo_set', { id, kind, big, icon: small });
    toast(isActive(v) ? 'Logo guardado. En Minecraft se ve tras reiniciar el servidor.' : 'Logo guardado');
  } catch (e) { fail(e); }
  refreshServer();
};
