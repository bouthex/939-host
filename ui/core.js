'use strict';
/* 939 Host — núcleo de la interfaz: utilidades, estado, navegación, diálogos y tema.
   Sin frameworks: liviana y rápida de abrir. */

const TAURI = window.__TAURI__;
const invoke = (cmd, args) => TAURI.core.invoke(cmd, args);
const listen = (ev, fn) => TAURI.event.listen(ev, fn);
const $ = (sel, root = document) => root.querySelector(sel);
const main = document.getElementById('main');

const state = {
  route: 'servers', params: {}, info: null, servers: [], server: null, tab: 'summary', hw: null,
  progress: {}, timers: [], wizard: null, versions: null, snapshots: null,
};

/* ───────── íconos «Bloque» ───────── */
// Set de íconos «Bloque» de 939 Host: grilla de 20, trazo 1.6, geometría ortogonal con
// escalones (como el logo Cinta); curvas solo donde la forma lo exige para leerse en 16 px.
const ICONS = {
  servers: '<path d="M3 3.5h14v5H3zM3 11.5h14v5H3z"/><path d="M5.5 6h2M5.5 14h2"/><path d="M14 6h.01M14 14h.01" stroke-width="2.2"/>',
  friends: '<path d="M4.5 3.5h4v4h-4z"/><path d="M2.5 16.5v-3l1.5-1.5h5l1.5 1.5v3"/><path d="M12.5 4.5h3.5v3.5h-3.5z"/><path d="M13 11.5h3.5l1 1v4"/>',
  user: '<path d="M7.5 3h5v5h-5z"/><path d="M3.5 17v-2.5l2-2h9l2 2V17"/>',
  settings: '<path d="M8.5 2.5h3l.4 2.1 1.5.9 2-.8 1.5 2.6-1.6 1.4v1.6l1.6 1.4-1.5 2.6-2-.8-1.5.9-.4 2.1h-3l-.4-2.1-1.5-.9-2 .8-1.5-2.6 1.6-1.4V9.7L2.6 8.3l1.5-2.6 2 .8 1.5-.9z"/><path d="M8.5 8.5h3v3h-3z"/>',
  plus: '<path d="M10 4v12M4 10h12"/>',
  copy: '<path d="M7 7h9.5v9.5H7z"/><path d="M13 7V3.5H3.5V13H7"/>',
  play: '<path d="M6.5 4.5v11l9-5.5z"/>',
  stop: '<path d="M5 5h10v10H5z"/>',
  pause: '<path d="M5.5 4.5h3v11h-3zM11.5 4.5h3v11h-3z"/>',
  restart: '<path d="M15.6 10a5.6 5.6 0 1 1-1.7-4"/><path d="M15.5 3v3.5H12"/>',
  refresh: '<path d="M16 9.5A6 6 0 0 0 5 6.2M4 10.5a6 6 0 0 0 11 3.3"/><path d="M5 2.5v3.7h3.7M15 17.5v-3.7h-3.7"/>',
  search: '<path d="M8.5 3.5h0a5 5 0 0 1 5 5v0a5 5 0 0 1-5 5h0a5 5 0 0 1-5-5v0a5 5 0 0 1 5-5z"/><path d="m12.5 12.5 4 4"/>',
  chev: '<path d="m7.5 4.5 5.5 5.5-5.5 5.5"/>',
  back: '<path d="m12.5 4.5-5.5 5.5 5.5 5.5"/>',
  x: '<path d="m5 5 10 10M15 5 5 15"/>',
  check: '<path d="m4.5 10.5 3.5 3.5 7.5-8"/>',
  more: '<path d="M4.5 9.5h1v1h-1zM9.5 9.5h1v1h-1zM14.5 9.5h1v1h-1z" stroke-width="1.8"/>',
  box: '<path d="M10 2.5 16.5 6v8L10 17.5 3.5 14V6z"/><path d="M3.5 6 10 9.5 16.5 6M10 9.5v8"/>',
  cube: '<path d="M10 2.5 16.5 6v8L10 17.5 3.5 14V6z"/><path d="M3.5 6 10 9.5 16.5 6M10 9.5v8"/>',
  clock: '<path d="M10 3a7 7 0 1 1 0 14 7 7 0 0 1 0-14z"/><path d="M10 6v4.5h3"/>',
  terminal: '<path d="M2.5 3.5h15v13h-15z"/><path d="m6 8 2.5 2L6 12M10.5 12.5H14"/>',
  shield: '<path d="M10 2.5 16 5v5c0 3.6-2.5 6.2-6 7.5-3.5-1.3-6-3.9-6-7.5V5z"/><path d="m7.5 10 2 2 3.5-4"/>',
  folder: '<path d="M2.5 4.5h5l1.5 2h8.5v10h-15z"/><path d="M2.5 8.5h15"/>',
  link: '<path d="M8.5 6.5H5.5a3.5 3.5 0 0 0 0 7h3M11.5 6.5h3a3.5 3.5 0 0 1 0 7h-3M7 10h6"/>',
  alert: '<path d="M10 3 17.5 16.5h-15z"/><path d="M10 8v4M10 14.2h.01"/>',
  info: '<path d="M3 3h14v14H3z"/><path d="M10 9v5M10 6.2h.01"/>',
  pc: '<path d="M2.5 3.5h15v10h-15z"/><path d="M6.5 16.5h7M10 13.5v3"/>',
  gauge: '<path d="M3.5 14.5a6.5 6.5 0 1 1 13 0"/><path d="m10 14.5 3-4.5"/><path d="M3.5 14.5h13"/>',
  download: '<path d="M10 3v9.5M6 8.5l4 4 4-4"/><path d="M3.5 13v3.5h13V13"/>',
  upload: '<path d="M10 13V3.5M6 7.5l4-4 4 4"/><path d="M3.5 13v3.5h13V13"/>',
  trash: '<path d="M3.5 5.5h13M8 5.5v-2h4v2"/><path d="M5 5.5 6 16.5h8l1-11"/><path d="M8.5 8.5v5M11.5 8.5v5"/>',
  logout: '<path d="M8.5 3.5h-5v13h5"/><path d="M12 6.5 15.5 10 12 13.5M15.5 10H7.5"/>',
  wrench: '<path d="M12.5 3.2a4 4 0 0 0-4.9 5.2l-4.4 4.4a1.6 1.6 0 0 0 2.3 2.3l4.4-4.4a4 4 0 0 0 5.2-4.9l-2.4 2.4-2.3-.5-.5-2.3z"/>',
  sun: '<path d="M7.5 7.5h5v5h-5z"/><path d="M10 2.5v2M10 15.5v2M2.5 10h2M15.5 10h2M4.7 4.7l1.4 1.4M13.9 13.9l1.4 1.4M4.7 15.3l1.4-1.4M13.9 6.1l1.4-1.4"/>',
  moon: '<path d="M15.5 12.2A6.5 6.5 0 0 1 7.8 4.5a6.5 6.5 0 1 0 7.7 7.7z"/>',
  monitor: '<path d="M2.5 3.5h15v10h-15z"/><path d="M6.5 16.5h7M10 3.5v10"/><path d="M10 3.5h7.5v10H10z" fill="currentColor" stroke="none"/>',
  external: '<path d="M11 3.5h5.5V9M16.5 3.5 9 11"/><path d="M14 12v4.5H3.5V6H8"/>',
  globe: '<path d="M10 3a7 7 0 1 1 0 14 7 7 0 0 1 0-14z"/><path d="M3 10h14M10 3c2 2 2.8 4.5 2.8 7S12 15 10 17c-2-2-2.8-4.5-2.8-7S8 5 10 3z"/>',
  eye: '<path d="M2.5 10s2.7-5 7.5-5 7.5 5 7.5 5-2.7 5-7.5 5-7.5-5-7.5-5z"/><path d="M8.5 8.5h3v3h-3z"/>',
  map: '<path d="M7.5 4 3 5.5V16l4.5-1.5 5 1.5 4.5-1.5V4l-4.5 1.5z"/><path d="M7.5 4v10.5M12.5 5.5V16"/>',
  key: '<path d="M3.5 9.5h6v6h-6z"/><path d="M9.5 9.5 15.5 3.5M13 6l2 2"/>',
  layers: '<path d="m10 3 7 4-7 4-7-4z"/><path d="m3 11 7 4 7-4"/>',
  heart: '<path d="M10 16.5 3.5 10V6l2-2h2.5l2 2 2-2h2.5l2 2v4z"/>',
  sync: '<path d="M15.5 8A6 6 0 0 0 4.6 6.4M4.5 12a6 6 0 0 0 10.9 1.6"/><path d="M15.5 3.5V8H11M4.5 16.5V12H9"/>',
  host: '<path d="M3.5 10.5h13v6h-13z"/><path d="M10 10.5V3h5l-1.2 1.75L15 6.5h-5"/><path d="M6.5 13.5h2"/>',
  hand: '<path d="M7 10V4.5h2V9M9 9V3.5h2V9M11 9V4.5h2V10M13 7.5h2V13l-2.5 3.5h-4L5 12.5 6 11l2 1.5"/>',
  dice: '<path d="M3.5 3.5h13v13h-13z"/><path d="M6.8 6.8h.01M13.2 13.2h.01M10 10h.01M13.2 6.8h.01M6.8 13.2h.01" stroke-width="2.4"/>',
  edit: '<path d="M12.5 3.5 16.5 7.5 7.5 16.5h-4v-4z"/><path d="M10.5 5.5 14.5 9.5"/>',
  bell: '<path d="M5 14.5V9a5 5 0 0 1 10 0v5.5l1.5 1.5h-13z"/><path d="M8.5 16.5h3"/>',
  // Símbolos de PC
  'sym-bloque': '<path d="M10 2.5 16.5 6v8L10 17.5 3.5 14V6z"/><path d="M3.5 6 10 9.5 16.5 6M10 9.5v8"/>',
  'sym-pico': '<path d="M3.5 7 7 3.5h6L16.5 7"/><path d="M10 3.5v14"/>',
  'sym-antorcha': '<path d="M9 10h2v7.5H9z"/><path d="M8.5 2.5h3v2h1v3.5h-5V4.5h1z"/>',
  'sym-hoja': '<path d="M4 16.5c0-7 4.5-12 12.5-12.5C16 12 11 16.5 4 16.5z"/><path d="M4 16.5 11 9.5"/>',
  'sym-cofre': '<path d="M3 5.5h14v11H3z"/><path d="M3 9.5h14"/><path d="M9 8.5h2v3H9z"/>',
  'sym-gema': '<path d="M6 3.5h8l3 4.5-7 9-7-9z"/><path d="M3 8h14M8 3.5 7 8l3 9 3-9-1-4.5"/>',
  'sym-faro': '<path d="M8 6.5h4l1 10H7z"/><path d="M7.5 3.5h5v3h-5z"/><path d="M4.5 17h11M7.5 11h5"/>',
  'sym-montana': '<path d="M2.5 16.5h15l-4-6-2 2-3.5-6z"/><path d="m6.5 10 1.5 1.5L10 9.5"/>',
  'sym-luna': '<path d="M14.5 13.5A6 6 0 0 1 7 5a6.5 6.5 0 1 0 7.5 8.5z"/>',
  'sym-rayo': '<path d="M11 2.5 4.5 11H9.5L8.5 17.5 15.5 9H10.5z"/>',
  'sym-ola': '<path d="M2.5 10h3V7h3v3h3V7h3v3h3"/><path d="M2.5 14.5h15"/>',
  'sym-estrella': '<path d="m10 2.8 2.2 4.6 5 .7-3.6 3.5.9 5-4.5-2.4-4.5 2.4.9-5-3.6-3.5 5-.7z"/>',
};

const icon = (n, size = 18, sw = 1.6) => `<svg class="ic" width="${size}" height="${size}" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="${sw}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[n] || ''}</svg>`;
/* Símbolo de una PC (avatar cuadrado). */
const PC_SYMBOLS = ['bloque', 'pico', 'antorcha', 'hoja', 'cofre', 'gema', 'faro', 'montana', 'luna', 'rayo', 'ola', 'estrella'];
const SYMBOL_NAMES = { bloque: 'Bloque', pico: 'Pico', antorcha: 'Antorcha', hoja: 'Hoja', cofre: 'Cofre', gema: 'Gema', faro: 'Faro', montana: 'Montaña', luna: 'Luna', rayo: 'Rayo', ola: 'Ola', estrella: 'Estrella' };
const pcAvatar = (symbol, size = 32, cls = '') => `<span class="pcav ${cls}" style="width:${size}px;height:${size}px" aria-hidden="true">${icon(PC_SYMBOLS.includes(symbol) ? `sym-${symbol}` : 'pc', Math.round(size * 0.55))}</span>`;
const CINTA = (w = 40) => `<svg width="${w}" height="${Math.round(w * 19.4 / 40.9)}" viewBox="-0.2 -0.2 40.9 19.4" aria-hidden="true"><path d="M11.2 9.5H1.35V1.35H11.2V17.65H1.35M11.2 1.35H25.2V17.65H15.4M19.2 9.5H25.2M25.2 17.65H39.15V1.35H29.3V9.5H39.15" fill="none" stroke="currentColor" stroke-width="2.7" stroke-linejoin="round" stroke-linecap="round"/></svg>`;

/* ───────── utilidades ───────── */
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const fmtMB = (mb) => (mb == null ? '—' : mb >= 1024 ? `${(mb / 1024).toFixed(1).replace('.', ',')} GB` : `${Math.round(mb)} MB`);
const fmtNum = (n) => (n == null ? '—' : n >= 1e6 ? `${(n / 1e6).toFixed(1).replace('.', ',')} M` : n >= 1e3 ? `${Math.round(n / 1e3)} k` : String(n));
const fmtDate = (iso) => { try { return new Date(iso).toLocaleString('es-AR', { dateStyle: 'medium', timeStyle: 'short' }); } catch { return iso || ''; } };
const clone = (o) => JSON.parse(JSON.stringify(o));
function fmtDuration(sec) {
  sec = Math.max(0, Math.floor(sec));
  const d = Math.floor(sec / 86400), h = Math.floor((sec % 86400) / 3600), m = Math.floor((sec % 3600) / 60);
  if (d) return `${d} d ${h} h`;
  if (h) return `${h} h ${m} min`;
  return m ? `${m} min` : `${sec} s`;
}
const fmtUptime = (iso) => (iso ? fmtDuration((Date.now() - new Date(iso).getTime()) / 1000) : '—');
function ago(iso) {
  if (!iso) return 'nunca';
  const m = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 60000));
  return m < 1 ? 'recién' : m < 60 ? `hace ${m} min` : m < 1440 ? `hace ${Math.round(m / 60)} h` : `hace ${Math.round(m / 1440)} d`;
}
const loaderName = (l) => ({ fabric: 'Fabric', forge: 'Forge', neoforge: 'NeoForge' }[l] || 'Vanilla');
const DIFF = { peaceful: 'Pacífico', easy: 'Fácil', normal: 'Normal', hard: 'Difícil' };
const MODE = { survival: 'Supervivencia', creative: 'Creativo', adventure: 'Aventura', spectator: 'Espectador' };
const initials = (name) => (name || '?').replace(/[^A-Za-z0-9]/g, '').slice(0, 2).toUpperCase() || '?';
const debounce = (fn, ms) => { let t; return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); }; };

/* ───────── avisos y errores ───────── */
function toast(message, kind = 'ok', detail = '') {
  const el = document.createElement('div');
  el.className = `toast ${kind === 'error' ? 'error' : ''}`;
  el.setAttribute('role', kind === 'error' ? 'alert' : 'status');
  el.innerHTML = `<span>${esc(message)}</span>${detail ? `<span class="detail">${esc(detail)}</span>` : ''}`;
  document.getElementById('toasts').appendChild(el);
  setTimeout(() => el.remove(), kind === 'error' ? 8000 : 3500);
}
function errMsg(e) { return typeof e === 'string' ? e : e?.message || 'Ocurrió un error inesperado.'; }
function fail(e) { console.error(e); toast(errMsg(e), 'error', e?.detail || ''); }
async function call(cmd, args, okMsg) {
  try { const r = await invoke(cmd, args); if (okMsg) toast(okMsg); return r; } catch (e) { fail(e); throw e; }
}

/* ───────── diálogos ─────────
   buttons: [{ label, value, kind: 'pri'|'' }]. Devuelve el value elegido (o false al cancelar).
   requireText: hay que escribirlo para habilitar el botón principal. */
let dialogOpen = null;
function dialog({ title, body = '', confirm = 'Aceptar', cancel = 'Cancelar', buttons = null, requireText = null, wide = false, onMount = null }) {
  return new Promise((resolve) => {
    const root = document.getElementById('modal-root');
    const btns = buttons || [{ label: confirm, value: true, kind: 'pri' }];
    const prev = document.activeElement;
    root.innerHTML = `<div class="overlay" role="dialog" aria-modal="true" aria-labelledby="dlg-t"><div class="dialog ${wide ? 'wide' : ''}">
      <h2 id="dlg-t">${esc(title)}</h2><div class="stack" style="gap:10px">${body}</div>
      ${requireText ? `<label class="field">Escribí <span class="mono">${esc(requireText)}</span> para confirmar<input class="input" id="dlg-in" autocomplete="off" spellcheck="false"></label>` : ''}
      <div class="btns end">${cancel ? `<button class="btn" data-dv="__cancel">${esc(cancel)}</button>` : ''}${btns.map((b, i) => `<button class="btn ${b.kind || ''}" data-dv="${i}" ${requireText && b.kind === 'pri' ? 'disabled' : ''}>${esc(b.label)}</button>`).join('')}</div>
    </div></div>`;
    const done = (v) => { root.innerHTML = ''; dialogOpen = null; prev?.focus?.(); resolve(v); };
    dialogOpen = done;
    root.querySelectorAll('[data-dv]').forEach((b) => b.addEventListener('click', () => done(b.dataset.dv === '__cancel' ? false : btns[+b.dataset.dv].value)));
    const inp = root.querySelector('#dlg-in');
    if (inp) { inp.addEventListener('input', () => root.querySelectorAll('.btn.pri').forEach((b) => { b.disabled = inp.value.trim() !== requireText; })); inp.focus(); }
    else (root.querySelector('.btn.pri') || root.querySelector('.btn'))?.focus();
    root.querySelector('.overlay').addEventListener('keydown', (e) => { if (e.key === 'Escape' && cancel) done(false); });
    if (onMount) onMount(root.querySelector('.dialog'));
  });
}

/* ───────── tema ───────── */
const darkQuery = window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null;
function applyTheme() {
  const s = state.info?.settings || {};
  const t = s.theme || 'system';
  const dark = t === 'dark' || (t === 'system' && darkQuery?.matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  if (s.reduceMotion) document.documentElement.dataset.motion = 'reduce'; else delete document.documentElement.dataset.motion;
  document.documentElement.style.setProperty('--console-size', `${Math.min(18, Math.max(11, s.consoleFontSize || 13))}px`);
  try { localStorage.setItem('939-theme', t); localStorage.setItem('939-motion', s.reduceMotion ? 'reduce' : ''); } catch { /* sin almacenamiento */ }
}
darkQuery?.addEventListener?.('change', applyTheme);

async function saveSetting(patch, msg = 'Ajuste guardado') {
  const next = { ...state.info.settings, ...patch };
  try {
    state.info.settings = await invoke('save_settings', { settings: next });
    applyTheme();
    paintFoot();
    if (msg) toast(msg);
    return true;
  } catch (e) { fail(e); return false; }
}

/* ───────── temporizadores por pantalla ───────── */
function every(ms, fn) { const t = setInterval(fn, ms); state.timers.push(t); return t; }
function clearTimers() { state.timers.forEach(clearInterval); state.timers = []; }
const leaveHooks = [];
function onLeave(fn) { leaveHooks.push(fn); }

/* ───────── navegación ───────── */
const NAV = [['servers', 'Servidores', 'servers'], ['friends', 'Servidores de amigos', 'friends'], ['settings', 'Configuración', 'settings']];
function navKey() { return ['server', 'create', 'repair'].includes(state.route) ? 'servers' : state.route; }
function paintNav() {
  const k = navKey();
  $('#nav').innerHTML = NAV.map(([r, label, ic]) => `<button class="navi ${k === r ? 'on' : ''}" data-a="nav" data-route="${r}" ${k === r ? 'aria-current="page"' : ''}>${icon(ic)}<span>${label}</span>${r === 'servers' && state.servers.length ? `<span class="count">${state.servers.length}</span>` : ''}</button>`).join('');
}
const guards = [];
async function canLeave() { for (const g of guards) { if (!(await g())) return false; } return true; }
async function go(route, params = {}) {
  if (!(await canLeave())) return;
  clearTimers();
  while (leaveHooks.length) { try { leaveHooks.pop()(); } catch { /* nada */ } }
  guards.length = 0;
  state.route = route; state.params = params;
  paintNav();
  main.scrollTop = 0;
  await render();
  main.firstElementChild?.classList.add('enter');
}
const routes = {};
async function render() {
  try { await (routes[state.route] || routes.servers)(); } catch (e) { fail(e); }
  paintNav();
}

/* ───────── acciones (delegación) ───────── */
const A = {};
document.addEventListener('click', (e) => {
  const el = e.target.closest('[data-a]');
  if (!el || el.disabled || el.closest('#modal-root') && !el.closest('.dialog')) return;
  const fn = A[el.dataset.a];
  if (fn) { e.preventDefault(); Promise.resolve(fn(el, e)).catch(fail); }
});
A.nav = (el) => { state.tab = 'summary'; go(el.dataset.route); };
A.copy = async (el) => {
  try { await navigator.clipboard.writeText(el.dataset.text); } catch { toast('No se pudo copiar', 'error'); return; }
  toast(el.dataset.msg || 'Copiado');
  const ic = el.querySelector('svg.ic');
  if (ic && !el.dataset.busy) {
    el.dataset.busy = '1';
    const prev = ic.outerHTML;
    ic.outerHTML = icon('check', +ic.getAttribute('width') || 14).replace('class="ic"', 'class="ic copied"');
    setTimeout(() => { const now = el.querySelector('svg.ic'); if (now) now.outerHTML = prev; delete el.dataset.busy; }, 1300);
  }
};
A.link = (el) => call('open_link', { url: el.dataset.url }).catch(() => {});
A.folder = (el) => call('open_folder', { id: el.dataset.global ? null : state.params.id, which: el.dataset.which }).catch(() => {});

/* ───────── pie de la barra lateral ───────── */
async function paintFoot() {
  if (!state.info) return;
  $('#foot-node').textContent = state.info.settings.nodeName;
  $('#foot-ic').innerHTML = pcAvatar(state.info.settings.nodeSymbol, 30);
}

/* ───────── nombre visible y símbolo de esta PC (solo visual) ───────── */
async function identityDialog() {
  const s = state.info.settings;
  let name = s.nodeName, symbol = PC_SYMBOLS.includes(s.nodeSymbol) ? s.nodeSymbol : 'bloque';
  const paintPick = (d) => { d.querySelectorAll('.sympick button').forEach((b) => { const on = b.dataset.sym === symbol; b.classList.toggle('on', on); b.setAttribute('aria-pressed', on); }); d.querySelector('#id-prev').innerHTML = pcAvatar(symbol, 48, 'inv lg'); };
  const ok = await dialog({
    title: 'Nombre de esta PC',
    body: `<div class="row" style="gap:14px"><span id="id-prev"></span><div class="grow stack" style="gap:6px">
        <label class="field">Nombre<div class="row" style="gap:8px"><input class="input" id="id-name" maxlength="24" value="${esc(name)}" autocomplete="off" spellcheck="false"><button class="btn icon" id="id-dice" title="Otro nombre al azar" aria-label="Otro nombre al azar">${icon('dice', 18)}</button></div></label></div></div>
      <div class="field">Símbolo<div class="sympick" role="group" aria-label="Símbolo">${PC_SYMBOLS.map((k) => `<button type="button" data-sym="${k}" title="${SYMBOL_NAMES[k]}" aria-label="${SYMBOL_NAMES[k]}">${icon(`sym-${k}`, 22)}</button>`).join('')}</div></div>
      <p class="sm">Es solo para reconocer esta PC en el grupo. No cambia su identidad, sus claves, sus permisos ni los servidores.</p>`,
    confirm: 'Guardar',
    onMount: (d) => {
      paintPick(d);
      const inp = d.querySelector('#id-name');
      inp.addEventListener('input', () => { name = inp.value; });
      d.querySelector('#id-dice').addEventListener('click', async () => { name = await invoke('pc_name_random', { current: name }).catch(() => name); inp.value = name; inp.classList.remove('copied'); void inp.offsetWidth; inp.classList.add('copied'); });
      d.querySelectorAll('.sympick button').forEach((b) => b.addEventListener('click', () => { symbol = b.dataset.sym; paintPick(d); }));
      inp.select();
    },
  });
  if (!ok) return false;
  try {
    state.info.settings = await invoke('pc_identity_set', { name, symbol });
    paintFoot();
    toast(`Esta PC ahora se llama «${state.info.settings.nodeName}»`);
    return true;
  } catch (e) { fail(e); return false; }
}
A.identity = async () => { if (await identityDialog()) { if (state.route === 'settings') routes.settings(); else if (state.route === 'server' && state.tab === 'group') drawTab(); } };

/* ───────── solicitudes para alojar un servidor (llegan a la anfitriona) ───────── */
let pendingReqs = [];
async function refreshRequests() {
  try { pendingReqs = await invoke('host_requests_pending'); } catch { pendingReqs = []; }
  const root = $('#reqbar-root');
  if (!root) return;
  if (!pendingReqs.length) { root.innerHTML = ''; return; }
  const r = pendingReqs[0];
  const sig = `${r.id}|${r.request.nodeId}`;
  if (root.dataset.sig === sig && root.firstElementChild) return;
  root.dataset.sig = sig;
  root.innerHTML = `<div class="reqbar" role="alert">${pcAvatar(r.request.symbol, 32)}<div class="grow"><b>${esc(r.request.name)}</b> quiere alojar «${esc(r.serverName)}»${pendingReqs.length > 1 ? ` <span class="sm" style="opacity:.7">· y ${pendingReqs.length - 1} más</span>` : ''}<div class="xs" style="opacity:.7">El servidor sigue en esta PC hasta que aceptes.</div></div>
    <div class="btns"><button class="btn s" data-a="req-no" data-id="${esc(r.id)}">Rechazar</button><button class="btn s pri" data-a="req-review" data-id="${esc(r.id)}">Revisar</button></div></div>`;
}
const profileRows = (a, b) => {
  const ram = (p) => (p.ramTotalMb ? `${fmtMB(p.ramTotalMb)}` : '—');
  const free = (p) => (p.ramTotalMb ? `${fmtMB(p.ramFreeMb)}` : '—');
  return [['Procesador', a.cpu || '—', b.cpu || '—'], ['Núcleos', a.cores || '—', b.cores || '—'], ['Memoria total', ram(a), ram(b)], ['Memoria libre ahora', free(a), free(b)],
    ['Rendimiento por núcleo', a.cpuScore ?? '—', b.cpuScore ?? '—'], ['Disco libre', a.diskFreeGb != null ? `${Math.round(a.diskFreeGb)} GB` : '—', b.diskFreeGb != null ? `${Math.round(b.diskFreeGb)} GB` : '—']];
};
async function reviewRequest(id) {
  await refreshRequests();
  const p = pendingReqs.find((x) => x.id === id);
  if (!p) { toast('Esa solicitud ya no está pendiente.'); return; }
  const r = p.request;
  const conn = r.path ? `${r.path === 'relay' ? 'por relay' : 'directa'}${r.rttMs != null ? ` · ${r.rttMs} ms` : ''}` : (r.online ? 'conectada' : 'sin conexión ahora');
  const choice = await dialog({
    title: `${r.name} quiere alojar «${p.serverName}»`, wide: true,
    body: `<div class="row" style="gap:14px">${pcAvatar(r.symbol, 48, 'lg')}<div class="grow"><b>${esc(r.name)}</b><div class="sm t3">Pidió hace ${fmtDuration(r.ageSecs)} · vence en ${fmtDuration(r.expiresInSecs)} · conexión ${esc(conn)}</div></div></div>
      ${r.warnings.length ? `<div class="note strong">${icon('alert')}<div class="grow"><b>Tené en cuenta</b>${r.warnings.map((w) => `<span class="sm t2">${esc(w)}</span>`).join('')}</div></div>` : `<div class="note">${icon('check')}<div class="grow"><b>No vemos limitaciones importantes</b></div></div>`}
      ${r.strengths.length ? `<div class="stack" style="gap:4px">${r.strengths.map((g) => `<div class="row sm" style="gap:8px">${icon('check', 14)}<span>${esc(g)}</span></div>`).join('')}</div>` : ''}
      <div class="cmp"><div class="h"></div><div class="h">Esta PC</div><div class="h">${esc(r.name)}</div>${profileRows(r.mine, r.profile).map(([k, a, b]) => `<div class="k">${k}</div><div>${esc(a)}</div><div>${esc(b)}</div>`).join('')}</div>
      <p class="sm">Si aceptás, 939 Host copia el mundo a ${esc(r.name)} sin cortar el juego, guarda y recién ahí cambia de PC. Los jugadores se reconectan solos con la misma dirección. ${r.needsCopy ? 'Le falta copiar parte del mundo, así que puede tardar un poco más. ' : ''}Si algo falla, el servidor sigue acá con el mundo intacto.</p>`,
    cancel: 'Decidir después',
    buttons: [{ label: 'Rechazar', value: 'no' }, { label: `Pasar el servidor a ${r.name}`, value: 'yes', kind: 'pri' }],
  });
  if (choice === 'no') return answerRequest(id, false);
  if (choice === 'yes') return answerRequest(id, true, r.name);
}
async function answerRequest(id, accept, name = '') {
  $('#reqbar-root').innerHTML = ''; $('#reqbar-root').dataset.sig = '';
  if (accept) toast(`Pasando el servidor a ${name}…`);
  try {
    const r = await invoke('host_request_answer', { id, accept });
    toast(accept ? `Listo: ${r} es la nueva anfitriona` : 'Solicitud rechazada');
  } catch (e) { fail(e); }
  refreshRequests();
  if (state.route === 'server' && state.params.id === id) refreshServer();
}
A['req-review'] = (el) => reviewRequest(el.dataset.id);
A['req-no'] = (el) => answerRequest(el.dataset.id, false);
async function refreshFootState() {
  try {
    const l = await invoke('hardware_live', {});
    $('#foot-state').textContent = l.hostReady ? 'Lista para hostear' : 'Ocupada: no conviene hostear';
  } catch { /* sin datos */ }
}

/* ───────── estados del servidor (8) ───────── */
function serverState(v) {
  if (!v) return { key: 'stopped', label: 'Detenido', shape: 'off' };
  if (v.installing) return { key: 'starting', label: 'Instalando', shape: 'ring' };
  if (!v.config.installed) return v.lastError ? { key: 'error', label: 'Instalación fallida', shape: 'tri' } : { key: 'starting', label: 'Preparando', shape: 'ring' };
  if (v.restarting) return { key: 'restarting', label: 'Reiniciando', shape: 'ring' };
  switch (v.status) {
    case 'running':
      if (v.paused) return { key: 'paused', label: 'Pausado', shape: 'pause' };
      if (v.saving) return { key: 'saving', label: 'Guardando', shape: 'ring' };
      return { key: 'running', label: 'Funcionando', shape: 'dot' };
    case 'starting': return { key: 'starting', label: 'Iniciando', shape: 'ring', detail: v.phase };
    case 'stopping': return { key: 'stopping', label: 'Deteniéndose', shape: 'ring' };
    case 'crashed': return { key: 'error', label: 'Error', shape: 'tri' };
    default: return { key: 'stopped', label: 'Detenido', shape: 'off' };
  }
}
const SHAPES = { dot: '<span class="dot"></span>', off: '<span class="dot off"></span>', ring: '<span class="vload" aria-hidden="true"><i></i><i></i><i></i></span>', pause: '<span class="pausei"><i></i><i></i></span>', tri: '<span class="tri"></span>', sq: '<span class="sq"></span>' };
const stateHTML = (v, pill = false) => { const s = serverState(v); return `<span class="${pill ? 'statepill' : 'st'}">${SHAPES[s.shape]}${esc(s.label)}</span>`; };
const isActive = (v) => v && ['running', 'starting', 'stopping'].includes(v.status);


/* ───────── movimiento: indicadores que se deslizan ─────────
   Un solo observador actualiza el indicador de la barra lateral, la línea de las pestañas y el
   fondo de los segmentados cuando cambia el elemento activo. Solo usa transform y ancho. */
const reduceMotion = () => document.documentElement.dataset.motion === 'reduce' || window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
const scrollBehavior = () => (reduceMotion() ? 'auto' : 'smooth');
function place(ind, x, y, w, h, first) {
  if (first) ind.classList.add('no-anim');
  ind.style.transform = `translate(${x}px, ${y}px)`;
  if (w != null) ind.style.width = `${w}px`;
  if (h != null) ind.style.height = `${h}px`;
  if (first) requestAnimationFrame(() => requestAnimationFrame(() => ind.classList.remove('no-anim')));
}
function syncIndicators() {
  const navInd = document.getElementById('nav-ind');
  const on = document.querySelector('#nav .navi.on');
  if (navInd) {
    if (on) { const first = !navInd.classList.contains('show'); place(navInd, 0, on.offsetTop, null, on.offsetHeight, first); navInd.classList.add('show'); }
    else navInd.classList.remove('show');
  }
  document.querySelectorAll('.tabs').forEach((t) => {
    const a = t.querySelector('.tab.on');
    let ind = t.querySelector(':scope > .tab-ind');
    if (!a) { ind?.remove(); return; }
    const first = !ind;
    if (!ind) { ind = document.createElement('span'); ind.className = 'tab-ind'; t.appendChild(ind); t.classList.add('has-ind'); }
    place(ind, a.offsetLeft, 0, a.offsetWidth, null, first);
  });
  document.querySelectorAll('.seg').forEach((sg) => {
    const a = sg.querySelector(':scope > button.on');
    let th = sg.querySelector(':scope > .seg-thumb');
    if (!a) { th?.remove(); sg.classList.remove('has-thumb'); return; }
    const first = !th;
    if (!th) { th = document.createElement('i'); th.className = 'seg-thumb'; sg.prepend(th); sg.classList.add('has-thumb'); }
    place(th, a.offsetLeft, a.offsetTop, a.offsetWidth, a.offsetHeight, first);
  });
}
let indFrame = 0;
const queueIndicators = () => { if (!indFrame) indFrame = requestAnimationFrame(() => { indFrame = 0; syncIndicators(); }); };
new MutationObserver((list) => {
  // Ignora los cambios que hacen los propios indicadores.
  if (list.every((m) => m.target.classList?.contains('tab-ind') || m.target.classList?.contains('seg-thumb') || m.target.id === 'nav-ind')) return;
  queueIndicators();
}).observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['class'] });
window.addEventListener('resize', queueIndicators);

/* Indicador circular de progreso (fracción 0–1, o indeterminado). */
function pring(frac, size = 22, indet = false) {
  const r = 9, c = 2 * Math.PI * r;
  const off = indet ? c * 0.7 : c * (1 - Math.max(0, Math.min(1, frac || 0)));
  return `<svg class="pring ${indet ? 'indet' : ''}" width="${size}" height="${size}" viewBox="0 0 22 22" aria-hidden="true"><circle class="track" cx="11" cy="11" r="${r}" fill="none" stroke-width="2.2"/><circle class="val" cx="11" cy="11" r="${r}" fill="none" stroke-width="2.2" stroke-linecap="round" stroke-dasharray="${c.toFixed(2)}" stroke-dashoffset="${off.toFixed(2)}"/></svg>`;
}
function setPring(el, frac) { const v = el?.querySelector('.val'); if (!v) return; const c = 2 * Math.PI * 9; v.style.strokeDashoffset = (c * (1 - Math.max(0, Math.min(1, frac)))).toFixed(2); }

/* Índice que sigue la sección visible (Configuración y Ajustes del servidor). */
function scrollSpy(tocSel, prefix) {
  const toc = document.querySelector(tocSel);
  if (!toc) return;
  const btns = [...toc.querySelectorAll('button[data-v]')];
  const secs = btns.map((b) => [b, document.getElementById(`${prefix}-${b.dataset.v}`)]).filter(([, el]) => el);
  let pinned = null, pinUntil = 0, raf = 0;
  const mark = (btn) => btns.forEach((b) => b.classList.toggle('on', b === btn));
  const update = () => {
    raf = 0;
    if (pinned && Date.now() < pinUntil) return mark(pinned);
    const top = main.getBoundingClientRect().top + 96;
    let cur = secs[0] && secs[0][0];
    for (const [b, el] of secs) if (el.getBoundingClientRect().top <= top) cur = b;
    // Al llegar al final, una sección corta no puede subir hasta arriba: queda la que se eligió
    if (pinned && main.scrollTop + main.clientHeight >= main.scrollHeight - 4) cur = pinned;
    if (cur) mark(cur);
  };
  const onScroll = () => { if (!raf) raf = requestAnimationFrame(update); };
  const onClick = (e) => { const b = e.target.closest('button[data-v]'); if (b && btns.includes(b)) { pinned = b; pinUntil = Date.now() + 900; mark(b); } };
  main.addEventListener('scroll', onScroll, { passive: true });
  toc.addEventListener('click', onClick);
  update();
  onLeave(() => { main.removeEventListener('scroll', onScroll); toc.removeEventListener('click', onClick); if (raf) cancelAnimationFrame(raf); });
}
