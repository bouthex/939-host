'use strict';
/* Jugadores: lista + perfil con datos reales del servidor (sin mods complementarios).
   - Conectado: el servidor responde a una consulta interna cada 4 s, solo mientras el perfil
     está abierto y la ventana visible. La consulta no aparece en la consola.
   - Desconectado: el último guardado de Minecraft (playerdata).
   Skins reales de Mojang e imágenes originales del juego, ambas desactivables en Configuración. */

const pl = { list: [], sel: null, tab: 'summary', detail: null, err: null, picked: null, poll: null, loading: false, access: null };
const media = { skins: {}, sprites: {}, requested: new Set(), names: null, namesFor: null, assets: null, mods: {}, modsAsked: new Set() };
const DIM = { 'minecraft:overworld': 'Mundo normal', 'minecraft:the_nether': 'Nether', 'minecraft:the_end': 'End' };
const PTABS = [['summary', 'Resumen'], ['inv', 'Inventario'], ['ender', 'Cofre de ender'], ['stats', 'Estadísticas'], ['perm', 'Permisos']];
const roman = (n) => ['', 'I', 'II', 'III', 'IV', 'V', 'VI', 'VII', 'VIII', 'IX', 'X'][n] || String(n);
const bare = (id) => String(id || '').replace(/^minecraft:/, '');
/* Sin imágenes del juego: abreviatura legible (el nombre completo está en el detalle). */
const abbr = (name) => { const w = String(name).split(/[\s_]+/).filter((x) => x.length > 2 || /\d/.test(x)); return (w.length > 1 ? w.slice(0, 3).map((x) => x[0]).join('') : String(name).slice(0, 4)).toUpperCase(); };

/* ───────── skins ───────── */
function loadSkin(name) {
  if (!(name in media.skins)) media.skins[name] = invoke('player_skin', { name }).catch(() => null);
  return media.skins[name];
}
function loadImg(src) { return new Promise((res, rej) => { const i = new Image(); i.onload = () => res(i); i.onerror = rej; i.src = src; }); }
async function faceCanvas(name, size = 8) {
  const url = await loadSkin(name);
  if (!url) return null;
  const img = await loadImg(url).catch(() => null);
  if (!img) return null;
  const c = document.createElement('canvas');
  c.width = c.height = 8;
  const g = c.getContext('2d');
  g.imageSmoothingEnabled = false;
  g.drawImage(img, 8, 8, 8, 8, 0, 0, 8, 8);
  g.drawImage(img, 40, 8, 8, 8, 0, 0, 8, 8);
  c.setAttribute('aria-hidden', 'true');
  return c;
}
async function bodyCanvas(name) {
  const url = await loadSkin(name);
  if (!url) return null;
  const img = await loadImg(url).catch(() => null);
  if (!img) return null;
  const legacy = img.height === 32;
  const c = document.createElement('canvas');
  c.width = 16; c.height = 32;
  const g = c.getContext('2d');
  g.imageSmoothingEnabled = false;
  const d = (sx, sy, w, h, dx, dy, flip = false) => {
    if (!flip) { g.drawImage(img, sx, sy, w, h, dx, dy, w, h); return; }
    g.save(); g.translate(dx + w, dy); g.scale(-1, 1); g.drawImage(img, sx, sy, w, h, 0, 0, w, h); g.restore();
  };
  d(8, 8, 8, 8, 4, 0); d(20, 20, 8, 12, 4, 8); d(44, 20, 4, 12, 0, 8); d(4, 20, 4, 12, 4, 20);
  if (legacy) { d(44, 20, 4, 12, 12, 8, true); d(4, 20, 4, 12, 8, 20, true); }
  else {
    d(36, 52, 4, 12, 12, 8); d(20, 52, 4, 12, 8, 20);
    d(20, 36, 8, 12, 4, 8); d(44, 36, 4, 12, 0, 8); d(52, 52, 4, 12, 12, 8); d(4, 36, 4, 12, 4, 20); d(4, 52, 4, 12, 8, 20);
  }
  d(40, 8, 8, 8, 4, 0);
  c.setAttribute('aria-hidden', 'true');
  return c;
}
/* Reemplaza las iniciales de [data-skin] por la cara de la skin (si están activadas). */
function paintSkins(root = document) {
  root.querySelectorAll('[data-skin]').forEach(async (el) => {
    const c = await faceCanvas(el.dataset.skin);
    if (c && el.isConnected) { el.textContent = ''; el.appendChild(c); }
  });
}

/* ───────── imágenes del juego ───────── */
function assetsVersion() { return state.server?.config.mcVersion; }
async function ensureAssets() {
  const v = assetsVersion();
  try { media.assets = await invoke('game_assets_status', { version: v }); } catch { media.assets = { enabled: false, ready: false }; }
  paintAssetsNote();
  if (media.assets.enabled && !media.assets.ready && !media.assets.preparing) {
    media.assets.preparing = true;
    paintAssetsNote();
    invoke('game_assets_prepare', { version: v }).then((s) => { media.assets = s; media.requested.clear(); loadNames(true); paintAssetsNote(); repaintProfile(); })
      .catch((e) => { media.assets = { ...media.assets, preparing: false, error: errMsg(e) }; paintAssetsNote(); });
  }
  if (media.assets.ready) loadNames();
}
async function loadNames(force = false) {
  const v = assetsVersion();
  if (!force && media.namesFor === v) return;
  media.namesFor = v;
  media.names = await invoke('game_names', { version: v }).catch(() => ({}));
  repaintProfile();
}
function paintAssetsNote() {
  const el = $('#assets-note');
  if (!el) return;
  const a = media.assets;
  if (!a) { el.innerHTML = ''; return; }
  if (!a.enabled) el.innerHTML = `<div class="note">${icon('eye')}<div class="grow"><span class="sm t2">Las imágenes originales del juego están desactivadas: se muestran nombres y formas simples.</span></div><button class="btn s ghost" data-a="settings-section" data-sec="privacy">Configurar</button></div>`;
  else if (a.preparing && !a.ready) el.innerHTML = `<div class="note">${SHAPES.ring}<div class="grow"><b>Descargando las imágenes del juego desde Mojang</b><span class="sm t2" id="assets-msg">Una sola vez para Minecraft ${esc(assetsVersion())}. Quedan en esta PC; no se comparten.</span><div class="bar" style="margin-top:6px"><i id="assets-bar" style="width:0%"></i></div></div></div>`;
  else if (a.error) el.innerHTML = `<div class="note">${icon('alert')}<div class="grow"><b>No se pudieron descargar las imágenes del juego</b><span class="sm t2">${esc(a.error)}. Se muestran nombres.</span></div><button class="btn s" data-a="assets-retry">Reintentar</button></div>`;
  else el.innerHTML = '';
}
A['assets-retry'] = () => { media.assets = null; ensureAssets(); };
/* Pide al backend las imágenes que falten y vuelve a pintar cuando llegan. */
function sprite(key) {
  const v = assetsVersion();
  const k = `${v}|${key}`;
  if (media.sprites[k] !== undefined) return media.sprites[k];
  if (media.assets?.ready && !media.requested.has(k)) { media.requested.add(k); queueSprite(key); }
  return null;
}
let spriteQueue = new Set(), spriteTimer = null;
function queueSprite(key) {
  spriteQueue.add(key);
  clearTimeout(spriteTimer);
  spriteTimer = setTimeout(async () => {
    const v = assetsVersion();
    const keys = [...spriteQueue]; spriteQueue = new Set();
    const got = await invoke('game_sprites', { version: v, keys }).catch(() => ({}));
    for (const key of keys) media.sprites[`${v}|${key}`] = got[key] || null;
    repaintProfile();
  }, 30);
}
/* Ítems de mods: nombre, mod e ícono salen de los .jar instalados en el servidor (backend). */
const isModded = (id) => /^[a-z0-9_.-]+:/.test(String(id || '')) && !String(id).startsWith('minecraft:');
let modQueue = new Set(), modTimer = null;
function modInfo(id) {
  if (!isModded(id)) return null;
  if (id in media.mods) return media.mods[id];
  if (!media.modsAsked.has(id)) {
    media.modsAsked.add(id); modQueue.add(id);
    clearTimeout(modTimer);
    modTimer = setTimeout(async () => {
      const ids = [...modQueue]; modQueue = new Set();
      const got = await invoke('mod_items', { id: state.params.id, ids }).catch(() => ({}));
      for (const i of ids) media.mods[i] = got[i] || { known: false, name: null, modName: null, icon: null };
      repaintProfile();
    }, 30);
  }
  return undefined;
}
const prettyId = (id) => { const t = String(id).split(':').pop().split('/').pop().replace(/_/g, ' '); return t.charAt(0).toUpperCase() + t.slice(1); };
const itemName = (id) => {
  if (isModded(id)) { const m = media.mods[id]; return m?.name || prettyId(id); }
  const b = bare(id); return media.names?.[`item:${b}`] || media.names?.[`block:${b}`] || b.replace(/_/g, ' ');
};
const itemImg = (id) => (isModded(id) ? (modInfo(id)?.icon || null) : (sprite(`item:${bare(id)}`) || icon3d(bare(id))));

/* Objetos que Minecraft dibuja en 3D (escudo, cofres, camas, cabezas): no traen imagen plana de
   ítem. Se arma un ícono recortando la cara que se ve de su textura. */
const BED_COLORS = ['white', 'orange', 'magenta', 'light_blue', 'yellow', 'lime', 'pink', 'gray', 'light_gray', 'cyan', 'purple', 'blue', 'brown', 'green', 'red', 'black'];
function special3d(id) {
  if (id === 'shield') return { tex: ['shield_base_nopattern', 'shield/base_nopattern'], parts: [[2, 2, 12, 22]] };
  const chest = { chest: 'chest/normal', trapped_chest: 'chest/trapped', ender_chest: 'chest/ender' }[id];
  if (chest) return { tex: [chest], parts: [[14, 14, 14, 5], [14, 33, 14, 10]] };
  const bed = id.match(/^(.+)_bed$/);
  if (bed && BED_COLORS.includes(bed[1])) return { tex: [`bed/${bed[1]}`], parts: [[6, 6, 16, 16]] };
  const head = { skeleton_skull: ['skeleton/skeleton'], wither_skeleton_skull: ['skeleton/wither_skeleton'], zombie_head: ['zombie/zombie'], creeper_head: ['creeper/creeper'], player_head: ['player/wide/steve', 'steve'] }[id];
  if (head) return { tex: head, parts: [[8, 8, 8, 8]], overlay: id === 'player_head' || id === 'zombie_head' ? [40, 8, 8, 8] : null };
  if (id === 'piglin_head') return { tex: ['piglin/piglin'], parts: [[8, 8, 10, 8]] };
  return null;
}
const built3d = {};
function icon3d(id) {
  const sp = special3d(id);
  if (!sp) return null;
  const v = assetsVersion(), k = `${v}|${id}`;
  if (k in built3d) return built3d[k];
  const src = sp.tex.map((t) => sprite(`tex:${t}`)).find(Boolean);
  const pending = sp.tex.some((t) => media.sprites[`${v}|tex:${t}`] === undefined);
  if (!src) { if (!pending) built3d[k] = null; return null; }
  built3d[k] = null;
  const img = new Image();
  img.onload = () => {
    const sc = img.width / 64;
    const w = Math.max(...sp.parts.map((p) => p[2])), h = sp.parts.reduce((a, p) => a + p[3], 0);
    const c = document.createElement('canvas');
    c.width = 32; c.height = 32;
    const g = c.getContext('2d');
    g.imageSmoothingEnabled = false;
    const f = Math.min(30 / w, 30 / h);
    let y = (32 - h * f) / 2;
    for (const [px, py, pw, ph] of sp.parts) {
      const x = (32 - pw * f) / 2;
      g.drawImage(img, px * sc, py * sc, pw * sc, ph * sc, x, y, pw * f, ph * f);
      if (sp.overlay) g.drawImage(img, sp.overlay[0] * sc, sp.overlay[1] * sc, sp.overlay[2] * sc, sp.overlay[3] * sc, x, y, pw * f, ph * f);
      y += ph * f;
    }
    built3d[k] = c.toDataURL('image/png');
    repaintProfile();
  };
  img.src = src;
  return null;
}
const effectName = (id) => media.names?.[`effect:${bare(id)}`] || bare(id).replace(/_/g, ' ');
const enchName = (id) => media.names?.[`enchantment:${bare(id)}`] || bare(id).replace(/_/g, ' ');

/* ───────── pestaña ───────── */
tabDraw.players = async (el) => {
  const gg = state.share?.enabled ? state.share.group : null;
  const away = gg && !gg.iAmHost && gg.lease?.state === 'hosting' && !isActive(state.server) ? (gg.members.find((m) => m.isHost)?.name || gg.lease.hostName) : null;
  el.innerHTML = `<div class="stack" style="gap:16px">${away ? `<div class="note">${icon('info')}<div class="grow"><b>Se está jugando en la PC de ${esc(away)}</b><span class="sm t2">Acá ves la última copia sincronizada. Para ver vida, inventario y ubicación en vivo, abrí 939 Host en esa PC.</span></div></div>` : ''}<div id="assets-note"></div><div class="players">
    <div class="card plist-card"><div class="card-head" style="flex-wrap:wrap"><h2>Jugadores</h2><span class="sm t3" id="pl-count"></span></div>
      <div class="pad" style="padding:12px 16px;border-bottom:1px solid var(--line)"><div class="row" style="gap:8px"><input class="input mono" id="pl-add" placeholder="Agregar por nombre" maxlength="16" autocomplete="off" spellcheck="false" aria-label="Nombre de Minecraft"><button class="btn icon" data-a="pl-add" title="Agregar a la lista blanca" aria-label="Agregar a la lista blanca">${icon('plus', 16)}</button></div></div>
      <div class="list plist" id="pl-list"></div></div>
    <div class="card" id="pl-profile"></div></div></div>`;
  $('#pl-add').addEventListener('keydown', (e) => { if (e.key === 'Enter') A['pl-add'](); });
  onLeave(stopPoll);
  media.mods = {}; media.modsAsked = new Set();
  pl.access = null;
  invoke('player_edit_access', { id: state.params.id }).then((a) => { pl.access = a; repaintProfile(); }).catch(() => { pl.access = null; });
  await loadPlayers();
  ensureAssets();
  paintAssetsNote();
};
tabRefresh.players = () => loadPlayers(true);
async function loadPlayers(quiet = false) {
  const before = pl.list.find((p) => p.name === pl.sel);
  const wasLive = !!(before?.online && pl.poll);
  try { pl.list = await invoke('players_list', { id: state.params.id }); } catch (e) { if (!quiet) fail(e); return; }
  pl.list.sort((a, b) => (b.online - a.online) || a.name.localeCompare(b.name));
  const prevSel = pl.sel;
  if (state.playerSel) { pl.sel = state.playerSel; state.playerSel = null; }
  if (pl.sel && !pl.list.some((p) => p.name === pl.sel)) pl.sel = null;
  if (!pl.sel && pl.list.length) pl.sel = (pl.list.find((p) => p.online) || pl.list[0]).name;
  paintList();
  const now = pl.list.find((p) => p.name === pl.sel);
  const liveNow = !!now?.online;
  // Sin cambios en el jugador abierto: no se vuelve a consultar (el sondeo sigue su ritmo).
  if (quiet && prevSel === pl.sel && wasLive === liveNow && pl.detail) { paintProfile(); return; }
  selectPlayer(pl.sel, prevSel === pl.sel);
}
function paintList() {
  const el = $('#pl-list');
  if (!el) return;
  const online = pl.list.filter((p) => p.online).length;
  $('#pl-count').textContent = pl.list.length ? `${online} conectados · ${pl.list.length} en total` : '';
  el.innerHTML = pl.list.length ? pl.list.map((p) => `<button class="li ${p.name === pl.sel ? 'sel' : ''}" data-a="pl-sel" data-name="${esc(p.name)}" style="padding:12px 16px">
      <span class="avatar" data-skin="${esc(p.name)}">${esc(initials(p.name))}</span>
      <div class="grow"><div class="row" style="gap:6px"><b class="ellipsis">${esc(p.name)}</b>${p.opLevel ? '<span class="chip s">Operador</span>' : ''}</div>
      <div class="xs t3 ellipsis">${p.online ? `En línea${p.onlineSince ? ` · ${esc(onFor(p.onlineSince))}` : ''}` : p.banned ? 'Baneado' : p.lastSeen ? `Visto ${esc(ago(p.lastSeen))}` : p.whitelisted ? 'En la lista blanca' : 'Desconectado'}</div></div>
      ${p.online ? '<span class="dot" aria-label="en línea"></span>' : ''}</button>`).join('')
    : `<div class="pad sm t2">Todavía no entró nadie. ${state.server.config.whitelist ? 'La lista blanca está activa: agregá a tus amigos por nombre arriba.' : 'Cualquiera con la dirección puede entrar (la lista blanca está desactivada).'}</div>`;
  paintSkins(el);
}
A['pl-sel'] = (el) => selectPlayer(el.dataset.name);
A['pl-add'] = async () => {
  const inp = $('#pl-add');
  const name = inp.value.trim();
  if (!/^[A-Za-z0-9_]{3,16}$/.test(name)) { toast('Escribí un nombre de Minecraft válido (3 a 16 letras, números o _).', 'error'); return; }
  try { toast(await invoke('player_action', { id: state.params.id, name, action: 'whitelist_add', reason: null })); inp.value = ''; pl.sel = name; } catch (e) { fail(e); }
  setTimeout(() => loadPlayers(true), 500);
};

function selectPlayer(name, keepData = false) {
  const changed = pl.sel !== name;
  pl.sel = name;
  if (changed || !keepData) { pl.detail = null; pl.err = null; pl.picked = null; }
  document.querySelectorAll('#pl-list .li').forEach((b) => b.classList.toggle('sel', b.dataset.name === name));
  stopPoll();
  if (!name) { paintProfile(); return; }
  fetchDetail();
  const p = pl.list.find((x) => x.name === name);
  // Conectado (aunque el servidor lo aloje otra PC del grupo): la ficha y el inventario se leen
  // del juego cada 2,5 s, así lo que recoge, usa o tira aparece solo.
  if (p?.online) pl.poll = setInterval(() => { if (!document.hidden) fetchDetail(true); }, 2500);
  paintProfile();
}
function stopPoll() { if (pl.poll) { clearInterval(pl.poll); pl.poll = null; } }
document.addEventListener('visibilitychange', () => { if (!document.hidden && pl.poll) fetchDetail(true); });
async function fetchDetail(quiet = false) {
  const name = pl.sel;
  if (!name || pl.loading) return;
  pl.loading = true;
  try {
    const d = await invoke('player_detail', { id: state.params.id, name });
    if (pl.sel === name) { pl.detail = d; pl.err = null; }
  } catch (e) {
    if (pl.sel === name) { pl.err = errMsg(e); if (!quiet) pl.detail = null; }
  } finally { pl.loading = false; }
  if (pl.sel === name) repaintProfile();
}
function repaintProfile() { if (pl.dragging) return; if (state.route === 'server' && state.tab === 'players') paintProfile(); }

/* ───────── perfil ───────── */
function paintProfile() {
  const el = $('#pl-profile');
  if (!el) return;
  const p = pl.list.find((x) => x.name === pl.sel);
  if (!p) { el.innerHTML = `<div class="empty" style="border:0">${icon('user', 28)}<h2>Elegí un jugador</h2><p>Vas a ver su vida, inventario, ubicación y estadísticas con datos reales del servidor.</p></div>`; return; }
  const d = pl.detail;
  const src = !d ? (pl.err ? `<span class="t2">${esc(pl.err)}</span>` : 'Cargando…')
    : d.source === 'live' ? '<span class="live"><span class="dot"></span>En vivo · se actualiza solo</span>'
      : d.source === 'saved' ? `Último guardado de Minecraft${d.savedAt ? ` · ${esc(fmtDate(d.savedAt))}` : ''}`
        : 'Sin datos guardados: todavía no entró a este mundo.';
  const scrollY = el.querySelector('.pf-scroll')?.scrollTop || 0;
  el.innerHTML = `<div class="pf-head">
      <span class="avatar lg" data-skin="${esc(p.name)}">${esc(initials(p.name))}</span>
      <div class="grow stack" style="gap:6px"><div class="row wrap" style="gap:8px"><h2 style="font-size:20px">${esc(p.name)}</h2>
        ${p.online ? '<span class="chip fill s">En línea</span>' : '<span class="chip s">Desconectado</span>'}${p.opLevel ? `<span class="chip s">Operador · nivel ${p.opLevel}</span>` : ''}${p.banned ? '<span class="chip s">Baneado</span>' : ''}${d?.gamemode ? `<span class="chip soft s">${MODE[d.gamemode] || esc(d.gamemode)}</span>` : ''}</div>
        <div class="sm t3">${src}</div>${p.uuid ? `<div class="xs t3 mono" title="UUID">${esc(p.uuid)}${p.online && p.onlineSince ? ` · conectado hace ${esc(onFor(p.onlineSince))}` : ''}</div>` : ''}</div>
      ${p.online ? `<button class="btn s" data-a="pl-act" data-act="kick">Expulsar</button>` : ''}</div>
    <div class="pf-tabs tabs" role="tablist">${PTABS.map(([k, l]) => `<button class="tab ${pl.tab === k ? 'on' : ''}" role="tab" aria-selected="${pl.tab === k}" data-a="pf-tab" data-v="${k}">${l}</button>`).join('')}</div>
    <div class="pf-body pf-scroll">${profileBody(p, d)}</div>`;
  el.querySelector('.pf-scroll').scrollTop = scrollY;
  paintSkins(el);
  if (pl.tab === 'inv') bodyCanvas(p.name).then((c) => { const f = $('#pf-figure'); if (c && f) { f.textContent = ''; f.appendChild(c); } });
}
A['pf-tab'] = (el) => { pl.tab = el.dataset.v; pl.picked = null; paintProfile(); };
function profileBody(p, d) {
  if (pl.tab === 'perm') return permBody(p);
  if (!d) return pl.err ? `<div class="note">${icon('alert')}<div class="grow"><b>No se pudieron leer los datos</b><span class="sm t2">${esc(pl.err)}</span></div><button class="btn s" data-a="pl-retry">Reintentar</button></div>` : '<div class="bar indet"><i></i></div>';
  if (d.source === 'none' && pl.tab !== 'stats') return `<div class="empty" style="border:0">${icon('info', 24)}<h2>Sin datos todavía</h2><p>Minecraft guarda los datos de cada jugador la primera vez que entra a este mundo.</p></div>`;
  if (pl.tab === 'inv') return invBody(d);
  if (pl.tab === 'ender') return enderBody(d);
  if (pl.tab === 'stats') return statsBody(d);
  return summaryBody(d);
}
A['pl-retry'] = () => fetchDetail();

function iconsRow(kind, value, max = 20) {
  const n = Math.max(0, Math.min(max, Math.round(value)));
  const hud = { heart: ['heart/full', 'heart/half', 'heart/container'], food: ['food_full', 'food_half', 'food_empty'], armor: ['armor_full', 'armor_half', 'armor_empty'] }[kind];
  let out = '';
  for (let i = 0; i < max / 2; i++) {
    const state_ = n >= (i + 1) * 2 ? 0 : n === i * 2 + 1 ? 1 : 2;
    const img = sprite(`hud:${hud[state_]}`);
    const base = state_ !== 2 && kind === 'heart' ? sprite('hud:heart/container') : null;
    out += img ? `<span style="position:relative;width:18px;height:18px">${base ? `<img class="sprite" src="${base}" alt="" style="position:absolute">` : ''}<img class="sprite" src="${img}" alt="" style="position:absolute"></span>`
      : `<span class="glyph ${kind} ${['', 'half', 'no'][state_]}"></span>`;
  }
  return `<div class="icons-row" role="img" aria-label="${n / 2} de ${max / 2}">${out}</div>`;
}
function summaryBody(d) {
  const pos = d.pos ? d.pos.map((x) => Math.floor(x)) : null;
  return `<div class="pgrid">
    <div class="vitals">
      <div class="sec">Vida</div><div class="row vrow">${iconsRow('heart', d.health ?? 0)}<span class="mono sm">${d.health != null ? (Math.round(d.health * 10) / 20).toString().replace('.', ',') : '—'} / 10</span>${stepper(d, 'health', 'la vida', 'medio corazón')}</div>
      <div class="sec">Hambre</div><div class="row vrow">${iconsRow('food', d.food ?? 0)}<span class="mono sm">${d.food != null ? (d.food / 2).toString().replace('.', ',') : '—'} / 10</span>${stepper(d, 'food', 'el hambre', 'medio muslo')}</div>
      ${d.saturation != null ? `<div class="sec">Saturación</div><div class="row vrow"><div class="bar sat"><i style="width:${Math.round(Math.min(20, d.saturation) / 20 * 100)}%"></i></div><span class="mono sm">${(Math.round(d.saturation * 10) / 10).toString().replace('.', ',')} / ${d.food ?? 20}</span>${stepper(d, 'saturation', 'la saturación', 'medio punto')}</div>` : ''}
      ${d.armorPoints ? `<div class="sec">Armadura</div><div class="row" style="gap:12px">${iconsRow('armor', d.armorPoints)}<span class="mono sm">${d.armorPoints} pts</span></div>` : ''}
      <div class="sec">Experiencia</div><div class="xp"><span class="mono b">Nivel ${d.xpLevel ?? 0}</span><div class="bar"><i style="width:${Math.round((d.xpProgress || 0) * 100)}%"></i></div><span class="mono sm t3">${Math.round((d.xpProgress || 0) * 100)} %</span>${stepper(d, 'xp', 'el nivel', 'un nivel')}</div>
      ${killRow(d)}
    </div>
    <div class="stack">
      ${pos ? `<div class="coords">${['X', 'Y', 'Z'].map((k, i) => `<div><div class="xs t3">${k}</div><div class="v">${pos[i]}</div></div>`).join('')}</div>` : ''}
      <div><div class="kv"><span class="k">Dimensión</span><span>${esc(DIM[d.dimension] || bare(d.dimension) || '—')}</span></div>
      <div class="kv"><span class="k">Mirando hacia</span><span>${esc(d.facing || '—')}</span></div>
      <div class="kv"><span class="k">Punto de aparición</span><span class="mono sm">${d.spawn ? `${d.spawn[1].join(' ')} · ${esc(DIM[d.spawn[0]] || bare(d.spawn[0]))}` : 'El del mundo'}</span></div>
      ${pos ? `<div class="kv"><span class="k">Ir hasta ahí</span><button class="btn s ghost" data-a="copy" data-text="/tp @s ${pos.join(' ')}" data-msg="Comando copiado">${icon('copy', 14)}Copiar /tp</button></div>` : ''}</div>
    </div></div>
    <div class="stack"><div class="sec">Efectos activos</div>${d.effects.length ? `<div class="effects">${d.effects.map((e) => {
      const img = sprite(`effect:${bare(e.id)}`);
      return `<div class="effect">${img ? `<img class="sprite" src="${img}" alt="">` : `<span class="avatar" style="width:24px;height:24px;border-radius:6px">${icon('layers', 14)}</span>`}<span class="grow b">${esc(effectName(e.id))} ${e.amplifier ? roman(e.amplifier + 1) : ''}</span><span class="mono sm t3">${e.durationTicks < 0 ? '∞' : fmtClock(e.durationTicks / 20)}</span></div>`;
    }).join('')}</div>` : '<span class="sm t3">Ninguno.</span>'}</div>`;
}
/* Controles − y + al lado de cada valor: un paso por toque (medio corazón, una unidad de hambre,
   media de saturación, un nivel). Solo si el backend dice que esta PC puede (propietaria, con el
   servidor acá). Conectado, se ofrece solo lo que Minecraft permite con comandos; un cambio a la
   vez por atributo para que los toques rápidos no se dupliquen. */
pl.busy = {};
function stepper(d, stat, what, unit) {
  const a = pl.access;
  if (!a?.allowed || !d || d.source === 'none') return '';
  const live = d.source === 'live';
  let why = '';
  if (live && (stat === 'food' || stat === 'saturation') && !a.liveFeed) why = 'Con el jugador conectado hace falta Minecraft 1.13 o más nuevo. Se puede cuando esté desconectado.';
  else if (live && stat === 'health' && !a.liveHealth) why = 'Cambiar la vida con el jugador conectado necesita Minecraft 1.13 o más nuevo.';
  else if (live && stat === 'xp' && !a.liveXp) why = 'Esta versión de Minecraft no tiene el comando de experiencia.';
  const busy = !!pl.busy[stat];
  const dis = busy || !!why;
  const b = (up) => `<button class="stp" data-a="pl-step" data-stat="${stat}" data-up="${up ? 1 : 0}" ${dis ? 'disabled' : ''} title="${esc(why || `${up ? 'Subir' : 'Bajar'} ${what} (${unit})`)}" aria-label="${up ? 'Subir' : 'Bajar'} ${what}">${up ? '+' : '−'}</button>`;
  return `<span class="steps ${busy ? 'busy' : ''}" role="group" aria-label="Cambiar ${what}" ${busy ? 'aria-busy="true"' : ''}>${b(false)}${b(true)}</span>`;
}
A['pl-step'] = async (el) => {
  const d = pl.detail, stat = el.dataset.stat, up = el.dataset.up === '1';
  if (!d || pl.busy[stat]) return;
  pl.busy[stat] = true; repaintProfile();
  try { await invoke('player_adjust', { id: state.params.id, name: d.name, stat, up }); }
  catch (e) { fail(e); }
  finally { pl.busy[stat] = false; }
  await fetchDetail(true);
};
function killRow(d) {
  const a = pl.access;
  if (!a?.allowed || !d || d.source === 'none') return '';
  const live = d.source === 'live';
  return `<div class="danger-row"><div class="grow"><span class="b sm">Matar al jugador</span><span class="xs t3">${live ? 'Muere de verdad en el juego, como con /kill.' : 'Solo se puede con el jugador conectado.'}</span></div>
    <button class="btn s danger-ghost" data-a="pl-kill" ${live && !pl.busy.kill ? '' : 'disabled'} ${pl.busy.kill ? 'aria-busy="true"' : ''}>${icon('alert', 14)}Matar</button></div>`;
}
A['pl-kill'] = async () => {
  const d = pl.detail;
  if (!d || d.source !== 'live' || pl.busy.kill) return;
  const ok = await dialog({ title: `¿Matar a ${d.name}?`, body: `<p class="sm t2">Muere de verdad en el servidor: según las reglas del mundo puede soltar su inventario y su experiencia. No se puede deshacer.</p>`, confirm: 'Matar', danger: true });
  if (!ok) return;
  pl.busy.kill = true; repaintProfile();
  try { toast(await invoke('player_kill', { id: state.params.id, name: d.name })); } catch (e) { fail(e); }
  finally { pl.busy.kill = false; }
  await fetchDetail(true);
};

const onFor = (iso) => { const m = Math.max(0, Math.floor((Date.now() - Date.parse(iso)) / 60000)); return m < 1 ? 'menos de 1 min' : m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`; };
const fmtClock = (s) => { s = Math.floor(s); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };

function slotHTML(item, opts = {}) {
  const ph = opts.ph ? sprite(`slot:${opts.ph}`) : null;
  const dk = opts.slot != null ? `${opts.group === 'ender' ? 'ender' : 'inv'}:${opts.slot}` : item ? `${opts.group === 'ender' ? 'ender' : 'inv'}:${item.slot}` : '';
  const dnd = canDrag() && dk ? `data-drop="${dk}"` : '';
  if (!item) return `<span class="slot ${opts.sel ? 'sel' : ''}" ${dnd} title="${esc(opts.label || 'Vacío')}" aria-label="${esc(opts.label || 'Vacío')}">${ph ? `<img class="ghost" src="${ph}" alt="">` : ''}</span>`;
  const img = itemImg(item.id);
  const mi = isModded(item.id) ? media.mods[item.id] : null;
  const unknown = mi && !mi.known;
  const key = `${opts.group || 'inv'}:${item.slot}`;
  const dur = item.maxDamage && item.damage != null ? Math.max(0, 1 - item.damage / item.maxDamage) : null;
  const name = item.customName || itemName(item.id);
  const label = unknown ? `${name} (mod no instalado: ${item.id.split(':')[0]})` : mi?.modName ? `${name} · ${mi.modName}` : name;
  return `<button class="slot has ${unknown ? 'unk' : ''} ${opts.sel ? 'sel' : ''} ${pl.picked === key ? 'picked' : ''}" data-a="pick-item" data-key="${esc(key)}" ${dnd}${dnd ? ' draggable="true"' : ''} title="${esc(label)}${item.count > 1 ? ` ×${item.count}` : ''}" aria-label="${esc(label)}${item.count > 1 ? `, ${item.count}` : ''}">
    ${img ? `<img src="${img}" alt="">` : unknown ? '<span class="nm unk-q" aria-hidden="true">?</span>' : `<span class="nm full" lang="es">${esc(name)}</span>`}
    ${item.count > 1 ? `<span class="ct">${item.count}</span>` : ''}${dur != null && dur < 1 ? `<span class="dur"><i style="width:${Math.round(dur * 100)}%"></i></span>` : ''}</button>`;
}
/* Arrastrar y soltar: mueve de verdad en el servidor (op «move», que verifica el resultado
   leyendo el inventario de nuevo). Equipo e inventario comparten grupo; el cofre de ender es aparte. */
function canDrag() { return !!(pl.access?.allowed && pl.detail?.source === 'live' && !pl.busy.inv); }
function dropOk(from, to) {
  if (!from || !to || from === to) return false;
  const [gf] = from.split(':'), [gt] = to.split(':');
  return gf === gt && !findItem(pl.detail, to.replace(/^inv:(10[0-3]|-106)$/, 'eq:$1'));
}
if (!window.__plDnd) {
  window.__plDnd = true;
  const slotOf = (e) => e.target.closest?.('#pl-profile [data-drop]');
  document.addEventListener('dragstart', (e) => {
    const el = slotOf(e); if (!el || !canDrag()) return;
    pl.dragging = el.dataset.drop; el.classList.add('dragging');
    e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/plain', el.dataset.drop);
  });
  document.addEventListener('dragover', (e) => {
    const el = slotOf(e); if (!el || !pl.dragging) return;
    if (dropOk(pl.dragging, el.dataset.drop)) { e.preventDefault(); e.dataTransfer.dropEffect = 'move'; el.classList.add('drop-on'); }
  });
  document.addEventListener('dragleave', (e) => { slotOf(e)?.classList.remove('drop-on'); });
  document.addEventListener('drop', (e) => {
    const el = slotOf(e); if (!el || !pl.dragging) return;
    e.preventDefault();
    const from = pl.dragging, to = el.dataset.drop; pl.dragging = null;
    if (!dropOk(from, to)) { repaintProfile(); return; }
    const [g, f] = from.split(':'), t = +to.split(':')[1];
    invRun({ op: 'move', group: g, from: +f, to: t }, () => { pl.picked = `${t >= 100 || t === -106 ? 'eq' : g}:${t}`; });
  });
  document.addEventListener('dragend', () => { if (pl.dragging) { pl.dragging = null; repaintProfile(); } });
}
function findItem(d, key) {
  const [g, s] = key.split(':');
  const slot = +s;
  if (g === 'eq') return { 103: d.equipment.head, 102: d.equipment.chest, 101: d.equipment.legs, 100: d.equipment.feet, '-106': d.equipment.offhand }[slot] || null;
  return (g === 'ender' ? d.ender : d.inventory).find((i) => i.slot === slot) || null;
}
A['pick-item'] = (el) => { pl.picked = pl.picked === el.dataset.key ? null : el.dataset.key; paintProfile(); };
function itemDetail(d) {
  const it = pl.picked ? findItem(d, pl.picked) : null;
  if (!it) return `<div class="card pad item-detail sm t2">Tocá un casillero para ver el detalle del ítem.</div>`;
  const img = itemImg(it.id);
  const mi = isModded(it.id) ? media.mods[it.id] : null;
  const left = it.maxDamage && it.damage != null ? it.maxDamage - it.damage : null;
  return `<div class="card pad item-detail stack">
    <div class="row">${img ? `<span class="slot" style="width:48px;height:48px"><img src="${img}" alt=""></span>` : ''}<div class="grow"><b>${esc(it.customName || itemName(it.id))}</b><div class="xs t3 mono ellipsis">${esc(it.id)}</div>${mi ? `<div class="xs ${mi.known ? 't2' : ''}">${mi.known ? `Del mod ${esc(mi.modName || it.id.split(':')[0])}` : `Ítem de «${esc(it.id.split(':')[0])}», un mod que no está instalado en el servidor: Minecraft lo guarda pero no lo puede usar hasta que vuelva el mod.`}</div>` : ''}</div></div>
    <div><div class="kv"><span class="k">Cantidad</span><span class="mono">${it.count}</span></div>
    ${left != null ? `<div class="kv"><span class="k">Durabilidad</span><span class="mono">${left} / ${it.maxDamage}</span></div><div class="bar"><i style="width:${Math.round(left / it.maxDamage * 100)}%"></i></div>` : ''}</div>
    ${it.enchantments.length ? `<div><div class="sec" style="margin-bottom:4px">Encantamientos</div>${it.enchantments.map(([e, l]) => `<div class="kv"><span>${esc(enchName(e))}</span><span class="mono">${roman(l)}</span></div>`).join('')}</div>` : ''}
    ${it.customName ? `<div class="xs t3">Nombre puesto en un yunque. Original: ${esc(itemName(it.id))}</div>` : ''}
    ${invActions(d, it)}</div>`;
}
/* Acciones reales sobre el inventario (con el jugador conectado y permiso). */
function invActions(d, it) {
  if (!pl.access?.allowed) return '';
  if (d.source !== 'live') return '<div class="xs t3">Para cambiar el inventario, el jugador tiene que estar conectado.</div>';
  const busy = pl.busy.inv;
  const dis = busy ? 'disabled aria-busy="true"' : '';
  return `<div class="row wrap" style="gap:6px">
    <button class="btn s icon" data-a="inv-count" data-d="-1" ${dis} aria-label="Uno menos" title="Uno menos">−</button>
    <button class="btn s icon" data-a="inv-count" data-d="1" ${dis} aria-label="Uno más" title="Uno más">+</button>
    <button class="btn s" data-a="inv-setcount" ${dis}>Cantidad…</button>
    ${pl.picked.startsWith('ender:') || pl.picked.startsWith('inv:') ? `<button class="btn s" data-a="inv-move" ${dis}>Mover…</button>` : ''}
    <button class="btn s danger-ghost" data-a="inv-remove" ${dis}>Quitar</button></div>`;
}
const pickedRef = () => { const [g, s] = (pl.picked || '').split(':'); return { group: g, slot: +s }; };
async function invRun(op, done) {
  const d = pl.detail;
  if (!d || pl.busy.inv) return;
  pl.busy.inv = true; paintProfile();
  try { toast(await invoke('player_inventory', { id: state.params.id, name: d.name, op })); if (done) done(); } catch (e) { fail(e); }
  finally { pl.busy.inv = false; }
  await fetchDetail(true);
}
A['inv-count'] = (el) => { const it = findItem(pl.detail, pl.picked); if (!it) return; const n = it.count + +el.dataset.d; invRun(n <= 0 ? { op: 'remove', ...pickedRef() } : { op: 'setCount', ...pickedRef(), count: n }); };
A['inv-setcount'] = async () => {
  const it = findItem(pl.detail, pl.picked); if (!it) return;
  let v = String(it.count);
  const r = await dialog({ title: 'Cambiar la cantidad', confirm: 'Cambiar', body: `<label class="field">Cantidad en este casillero<input class="input mono" id="ic-n" type="number" min="0" max="99" value="${it.count}"></label><p class="xs t3">0 lo vacía. El máximo depende del objeto (64, 16 o 1).</p>`,
    onMount: (dl) => { const i = dl.querySelector('#ic-n'); i.addEventListener('input', () => { v = i.value; }); i.select(); } });
  if (!r) return;
  const n = Math.max(0, Math.min(99, parseInt(v, 10) || 0));
  if (n === it.count) return;
  invRun(n === 0 ? { op: 'remove', ...pickedRef() } : { op: 'setCount', ...pickedRef(), count: n });
};
A['inv-remove'] = async () => {
  const it = findItem(pl.detail, pl.picked); if (!it) return;
  if (!(await dialog({ title: 'Quitar del inventario', body: `<p class="sm t2">Se borran ${it.count} × ${esc(it.customName || itemName(it.id))} del inventario de ${esc(pl.detail.name)}. No se puede deshacer.</p>`, confirm: 'Quitar', danger: true }))) return;
  invRun({ op: 'remove', ...pickedRef() }, () => { pl.picked = null; });
};
A['inv-move'] = async () => {
  const d = pl.detail, { group, slot } = pickedRef();
  const used = new Set((group === 'ender' ? d.ender : d.inventory).map((i) => i.slot));
  const max = group === 'ender' ? 27 : 36;
  const free = []; for (let s = 0; s < max; s++) if (!used.has(s)) free.push(s);
  if (!free.length) { toast('No hay casilleros vacíos a donde moverlo.', 'error'); return; }
  const label = (s) => group === 'ender' ? `Cofre de ender · ${s + 1}` : s < 9 ? `Barra rápida · ${s + 1}` : `Inventario · ${s - 8}`;
  let to = free[0];
  const r = await dialog({ title: 'Mover a otro casillero', confirm: 'Mover', body: `<label class="field">Casillero vacío<select class="input" id="im-to">${free.map((s) => `<option value="${s}">${label(s)}</option>`).join('')}</select></label><p class="xs t3">Se mueve tal cual: encantamientos, nombre y desgaste incluidos.</p>`,
    onMount: (dl) => { const i = dl.querySelector('#im-to'); i.addEventListener('change', () => { to = +i.value; }); } });
  if (!r) return;
  invRun({ op: 'move', group, from: slot, to }, () => { pl.picked = `${group}:${to}`; });
};
A['inv-give'] = async () => {
  const d = pl.detail; if (!d) return;
  let item = '', count = 1;
  const r = await dialog({ title: `Dar un objeto a ${esc(d.name)}`, confirm: 'Dar', body: `<label class="field">Objeto<input class="input mono" id="ig-id" placeholder="minecraft:diamond" spellcheck="false"></label><label class="field">Cantidad<input class="input mono" id="ig-n" type="number" min="1" max="2304" value="1"></label><p class="xs t3">Id del objeto como en Minecraft (también de mods, como «create:cogwheel»). Si no tiene lugar, lo que sobra cae a sus pies.</p>`,
    onMount: (dl) => { const a = dl.querySelector('#ig-id'), b = dl.querySelector('#ig-n'); a.addEventListener('input', () => { item = a.value.trim(); }); b.addEventListener('input', () => { count = parseInt(b.value, 10) || 1; }); a.focus(); } });
  if (!r || !item) return;
  invRun({ op: 'give', item, count });
};
/* Vista de lista: cada objeto con su nombre completo, cantidad, durabilidad y encantamientos. */
pl.invView = 'grid';
A['inv-view'] = (el) => { pl.invView = el.dataset.v; paintProfile(); };
function viewToggle() {
  return `<div class="seg" role="group" aria-label="Vista del inventario">${[['grid', 'Casilleros'], ['list', 'Lista']].map(([k, l]) => `<button class="${pl.invView === k ? 'on' : ''}" data-a="inv-view" data-v="${k}" aria-pressed="${pl.invView === k}">${l}</button>`).join('')}</div>`;
}
function itemRows(groups) {
  const rows = groups.flatMap(([label, list]) => list.filter((x) => x.item).length ? [`<div class="inv-group">${esc(label)}</div>`, ...list.filter((x) => x.item).map(({ item, key, note }) => {
    const img = itemImg(item.id);
    const mi = isModded(item.id) ? media.mods[item.id] : null;
    const unknown = mi && !mi.known;
    const name = item.customName || itemName(item.id);
    const dur = item.maxDamage && item.damage != null ? Math.max(0, 1 - item.damage / item.maxDamage) : null;
    const sub = [note, unknown ? `mod no instalado (${item.id.split(':')[0]})` : mi?.modName ? mi.modName : null, item.enchantments?.length ? item.enchantments.map(([e, l]) => `${enchName(e)}${l > 1 ? ` ${roman(l)}` : ''}`).join(', ') : null].filter(Boolean).join(' · ');
    return `<button class="inv-row ${pl.picked === key ? 'on' : ''} ${unknown ? 'unk' : ''}" data-a="pick-item" data-key="${esc(key)}">
      <span class="ic">${img ? `<img src="${img}" alt="">` : unknown ? '?' : ''}</span>
      <span class="grow" style="min-width:0"><span class="b">${esc(name)}</span>${sub ? `<span class="xs t3">${esc(sub)}</span>` : ''}</span>
      ${dur != null && dur < 1 ? `<span class="dur-l" title="Durabilidad ${Math.round(dur * 100)} %"><i style="width:${Math.round(dur * 100)}%"></i></span>` : ''}
      <span class="mono sm">${item.count > 1 ? `×${item.count}` : ''}</span></button>`;
  })] : []);
  return rows.length ? `<div class="inv-list">${rows.join('')}</div>` : '<span class="sm t3">Vacío.</span>';
}
function invBody(d) {
  const at = (s) => d.inventory.find((i) => i.slot === s) || null;
  const eq = d.equipment;
  const main = []; for (let s = 9; s < 36; s++) main.push(slotHTML(at(s), { slot: s }));
  const hot = []; for (let s = 0; s < 9; s++) hot.push(slotHTML(at(s), { sel: d.selectedSlot === s, slot: s }));
  const e = (it, slot, ph, label) => slotHTML(it ? { ...it, slot } : null, { group: 'eq', ph, label, slot });
  if (pl.invView === 'list') {
    const eqs = [[eq.head, 103, 'Casco'], [eq.chest, 102, 'Pechera'], [eq.legs, 101, 'Pantalones'], [eq.feet, 100, 'Botas'], [eq.offhand, -106, 'Mano secundaria']].map(([it, sl, note]) => ({ item: it ? { ...it, slot: sl } : null, key: `eq:${sl}`, note }));
    const hotL = []; for (let sl = 0; sl < 9; sl++) hotL.push({ item: at(sl), key: `inv:${sl}`, note: d.selectedSlot === sl ? 'en la mano' : `casillero ${sl + 1}` });
    const mainL = []; for (let sl = 9; sl < 36; sl++) mainL.push({ item: at(sl), key: `inv:${sl}` });
    return `<div class="inv-wrap"><div class="stack">${viewToggle()}${itemRows([['Equipo', eqs], ['Barra rápida', hotL], ['Inventario', mainL]])}</div>${itemDetail(d)}</div>`;
  }
  return `<div class="inv-wrap">
    <div class="stack">
      <div class="spread">${viewToggle()}${pl.access?.allowed && d.source === 'live' ? `<button class="btn s" data-a="inv-give" ${pl.busy.inv ? 'disabled' : ''}>${icon('plus', 14)}Dar un objeto</button>` : ''}</div>
      <div class="body3"><div class="figure" id="pf-figure"><span class="avatar lg" style="border:0;background:none">${esc(initials(d.name))}</span></div>
        <div class="slotcol">${e(eq.head, 103, 'helmet', 'Casco')}${e(eq.chest, 102, 'chestplate', 'Pechera')}${e(eq.legs, 101, 'leggings', 'Pantalones')}${e(eq.feet, 100, 'boots', 'Botas')}</div>
        <div class="stack" style="gap:6px;justify-content:flex-end;align-self:flex-end"><span class="xs t3">Mano secundaria</span>${e(eq.offhand, -106, 'shield', 'Mano secundaria')}</div></div>
      <div class="grid9">${main.join('')}</div>
      <div class="grid9" style="padding-top:10px;border-top:1px solid var(--line)">${hot.join('')}</div>
      <span class="xs t3">El casillero marcado es el que tiene en la mano.${canDrag() ? ' Arrastrá un objeto a un casillero vacío para moverlo.' : ''}</span>
    </div>${itemDetail(d)}</div>`;
}
function enderBody(d) {
  const cells = []; for (let s = 0; s < 27; s++) cells.push(slotHTML(d.ender.find((i) => i.slot === s) || null, { group: 'ender', slot: s }));
  if (pl.invView === 'list') {
    const L = []; for (let sl = 0; sl < 27; sl++) L.push({ item: d.ender.find((i) => i.slot === sl) || null, key: `ender:${sl}` });
    return `<div class="inv-wrap"><div class="stack">${viewToggle()}${itemRows([['Cofre de ender', L]])}</div>${itemDetail(d)}</div>`;
  }
  return `<div class="inv-wrap"><div class="stack">${viewToggle()}<div class="grid9">${cells.join('')}</div><span class="xs t3">${d.ender.length ? `${d.ender.length} de 27 casilleros ocupados.` : 'Vacío.'}</span></div>${itemDetail(d)}</div>`;
}
function statsBody(d) {
  const s = d.stats;
  const n = (v) => (v == null ? '—' : v.toLocaleString('es-AR'));
  const cells = [
    ['Tiempo jugado', s.playTimeTicks != null ? fmtDuration(s.playTimeTicks / 20) : '—'],
    ['Muertes', n(s.deaths)], ['Desde la última muerte', s.timeSinceDeathTicks != null ? fmtDuration(s.timeSinceDeathTicks / 20) : '—'],
    ['Mobs eliminados', n(s.mobKills)], ['Jugadores eliminados', n(s.playerKills)], ['Logros', n(s.advancements)],
    ['Distancia recorrida', s.distanceCm != null ? `${(s.distanceCm / 100000).toFixed(1).replace('.', ',')} km` : '—'],
    ['Saltos', n(s.jumps)], ['Daño causado', s.damageDealt != null ? `${n(Math.round(s.damageDealt / 20))} ❤` : '—'],
  ];
  const h = d.history || { sessions: 0, recent: [], otherNames: [] };
  const hist = h.sessions ? `<div class="stack" style="gap:8px"><div class="sec">Sesiones registradas</div>
      <div class="statgrid">${[['Sesiones', n(h.sessions)], ['Tiempo en esas sesiones', fmtDuration(h.totalSecs)]].map(([k, v]) => `<div><span class="xs t3">${k}</span><span class="v">${esc(v)}</span></div>`).join('')}</div>
      <div><div class="kv"><span class="k">Primera vez</span><span>${h.firstSeen ? esc(fmtDate(h.firstSeen)) : '—'}</span></div><div class="kv"><span class="k">Última salida</span><span>${h.lastSeen ? esc(fmtDate(h.lastSeen)) : '—'}</span></div></div>
      <div class="sec" style="margin-top:4px">Últimas sesiones</div>
      ${h.otherNames.length ? `<span class="sm t2">También entró como ${h.otherNames.map((x) => `<b>${esc(x)}</b>`).join(', ')} (Minecraft avisó el cambio de nombre).</span>` : ''}
      <div class="list sessions">${h.recent.map((r) => `<div class="kv"><span>${esc(fmtDate(r.join))}</span><span class="mono sm">${esc(fmtDuration(r.secs))}${r.cut ? ' · <span class="t3">cortada por cierre del servidor</span>' : ''}</span></div>`).join('')}</div>
      <span class="xs t3">Entradas y salidas que 939 Host vio en la consola de este servidor. Lo anterior a esta versión no está registrado.</span></div>` : '';
  return `<div class="statgrid">${cells.map(([k, v]) => `<div><span class="xs t3">${k}</span><span class="v">${v}</span></div>`).join('')}</div>
    <span class="xs t3">Estadísticas que guarda Minecraft para este mundo. Se actualizan cuando el servidor guarda.</span>${hist}`;
}
function permBody(p) {
  return `<div class="stack">
    <div class="card" style="padding:0 20px">
      <div class="switch" style="cursor:default"><div class="grow"><b>Lista blanca</b><span class="sm t2">${state.server.config.whitelist ? 'Puede entrar aunque la lista blanca esté activa.' : 'La lista blanca está desactivada: entra cualquiera con la dirección.'}</span></div><button class="btn s" data-a="pl-act" data-act="${p.whitelisted ? 'whitelist_remove' : 'whitelist_add'}">${p.whitelisted ? 'Quitar' : 'Agregar'}</button></div>
      <div class="switch" style="cursor:default"><div class="grow"><b>Operador</b><span class="sm t2">Puede usar comandos y administrar el servidor.</span></div><button class="btn s" data-a="pl-act" data-act="${p.opLevel ? 'deop' : 'op'}">${p.opLevel ? 'Quitar operador' : 'Hacer operador'}</button></div>
      <div class="switch" style="cursor:default"><div class="grow"><b>${p.banned ? 'Baneado' : 'Banear'}</b><span class="sm t2">${p.banned ? esc(p.banReason || 'Sin motivo.') : 'No puede volver a entrar hasta que lo desbanees.'}</span></div><button class="btn s" data-a="pl-act" data-act="${p.banned ? 'pardon' : 'ban'}">${p.banned ? 'Desbanear' : 'Banear…'}</button></div>
    </div>
    ${p.uuid ? `<div class="kv"><span class="k">UUID</span><span class="mono xs">${esc(p.uuid)}</span></div>` : ''}
    <span class="xs t3">${state.server.status === 'running' ? 'El servidor está encendido: los cambios se aplican al instante.' : 'El servidor está detenido: los cambios se guardan y se aplican al iniciar.'}</span></div>`;
}
A['pl-act'] = async (el) => {
  const action = el.dataset.act, name = pl.sel;
  let reason = null;
  if (action === 'ban' || action === 'kick') {
    let r = '';
    const ok = await dialog({ title: action === 'ban' ? `Banear a ${name}` : `Expulsar a ${name}`, body: '<label class="field">Motivo <span class="hint">Opcional; lo ve el jugador</span><input class="input" id="pl-reason" maxlength="120"></label>', confirm: action === 'ban' ? 'Banear' : 'Expulsar', onMount: (d) => d.querySelector('#pl-reason').addEventListener('input', (e) => { r = e.target.value; }) });
    if (!ok) return;
    reason = r.trim() || null;
  }
  try { toast(await invoke('player_action', { id: state.params.id, name, action, reason })); } catch (e) { fail(e); }
  setTimeout(() => loadPlayers(true), 600);
};
