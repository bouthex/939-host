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

/* ───────── íconos (20 px, trazo 1.5) ───────── */
const ICONS = {
  servers: '<rect x="3" y="3.5" width="14" height="5" rx="1.5"/><rect x="3" y="11.5" width="14" height="5" rx="1.5"/><path d="M6 6h.01M6 14h.01"/>',
  friends: '<circle cx="7.5" cy="7" r="2.75"/><path d="M2.5 16.5c.6-2.6 2.6-4 5-4s4.4 1.4 5 4"/><circle cx="14" cy="7.5" r="2.25"/><path d="M13.5 12.6c2 .1 3.4 1.4 4 3.9"/>',
  settings: '<circle cx="10" cy="10" r="2.5"/><path d="M10 2.5v2M10 15.5v2M2.5 10h2M15.5 10h2M4.7 4.7l1.4 1.4M13.9 13.9l1.4 1.4M4.7 15.3l1.4-1.4M13.9 6.1l1.4-1.4"/>',
  plus: '<path d="M10 4v12M4 10h12"/>',
  copy: '<rect x="7" y="7" width="9.5" height="9.5" rx="1.5"/><path d="M13 7V4.5A1.5 1.5 0 0 0 11.5 3h-7A1.5 1.5 0 0 0 3 4.5v7A1.5 1.5 0 0 0 4.5 13H7"/>',
  play: '<path d="M6.5 4.5v11l9-5.5z"/>',
  stop: '<rect x="5" y="5" width="10" height="10" rx="1.5"/>',
  pause: '<path d="M7 4.5v11M13 4.5v11"/>',
  restart: '<path d="M15.5 10A5.5 5.5 0 1 1 13.9 6.1M16 3.5v3h-3"/>',
  search: '<circle cx="9" cy="9" r="5.5"/><path d="m13.2 13.2 3.8 3.8"/>',
  chev: '<path d="m7.5 4.5 5.5 5.5-5.5 5.5"/>',
  back: '<path d="m12.5 4.5-5.5 5.5 5.5 5.5"/>',
  user: '<circle cx="10" cy="7" r="3"/><path d="M4 16.5c.8-3 3.1-4.5 6-4.5s5.2 1.5 6 4.5"/>',
  box: '<path d="M10 2.5 16.5 6v8L10 17.5 3.5 14V6z"/><path d="M3.5 6 10 9.5 16.5 6M10 9.5v8"/>',
  clock: '<circle cx="10" cy="10" r="7"/><path d="M10 6v4l2.5 2"/>',
  terminal: '<rect x="2.5" y="3.5" width="15" height="13" rx="1.5"/><path d="m6 8 2.5 2L6 12M10.5 12.5H14"/>',
  shield: '<path d="M10 2.5 16 5v4.5c0 3.8-2.6 6.6-6 8-3.4-1.4-6-4.2-6-8V5z"/>',
  folder: '<path d="M2.5 6A1.5 1.5 0 0 1 4 4.5h3.5l2 2H16A1.5 1.5 0 0 1 17.5 8v6.5A1.5 1.5 0 0 1 16 16H4a1.5 1.5 0 0 1-1.5-1.5z"/>',
  link: '<path d="M8.5 11.5a3 3 0 0 0 4.2 0l2.8-2.8a3 3 0 0 0-4.2-4.2L10 5.8M11.5 8.5a3 3 0 0 0-4.2 0l-2.8 2.8a3 3 0 0 0 4.2 4.2L10 14.2"/>',
  alert: '<path d="M10 3 17.5 16h-15z"/><path d="M10 8v3.5M10 13.8h.01"/>',
  check: '<path d="m4.5 10.5 3.5 3.5 7.5-8"/>',
  pc: '<rect x="2.5" y="3.5" width="15" height="10" rx="1.5"/><path d="M7 17h6M10 13.5V17"/>',
  gauge: '<path d="M3.5 14a6.5 6.5 0 1 1 13 0"/><path d="m10 14 3-4"/>',
  download: '<path d="M10 3v9.5M6 9l4 4 4-4M3.5 16.5h13"/>',
  upload: '<path d="M10 13.5V4M6 7.5l4-4 4 4M3.5 16.5h13"/>',
  trash: '<path d="M4 6h12M8 6V4h4v2M5.5 6l.8 10h7.4l.8-10"/>',
  logout: '<path d="M8 3.5H4.5v13H8M12 6.5 15.5 10 12 13.5M15.5 10H7.5"/>',
  cube: '<path d="M10 2.5 16.5 6v8L10 17.5 3.5 14V6z"/>',
  wrench: '<path d="M12.5 3.2a4 4 0 0 0-4.9 5.2l-4.4 4.4a1.6 1.6 0 0 0 2.3 2.3l4.4-4.4a4 4 0 0 0 5.2-4.9l-2.4 2.4-2.3-.5-.5-2.3z"/>',
  refresh: '<path d="M16 10a6 6 0 0 1-10.4 4.1M4 10a6 6 0 0 1 10.4-4.1M14.8 2.8v3.4h-3.4M5.2 17.2v-3.4h3.4"/>',
  sun: '<circle cx="10" cy="10" r="3.2"/><path d="M10 2.5v1.8M10 15.7v1.8M2.5 10h1.8M15.7 10h1.8M4.7 4.7 6 6M14 14l1.3 1.3M4.7 15.3 6 14M14 6l1.3-1.3"/>',
  moon: '<path d="M15.5 12.2A6.5 6.5 0 0 1 7.8 4.5a6.5 6.5 0 1 0 7.7 7.7z"/>',
  monitor: '<rect x="2.5" y="3.5" width="15" height="10" rx="1.5"/><path d="M7 17h6"/>',
  x: '<path d="M5 5l10 10M15 5 5 15"/>',
  external: '<path d="M11 3.5h5.5V9M16.5 3.5 9 11M14 12v3.5a1 1 0 0 1-1 1H4.5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1H8"/>',
  info: '<circle cx="10" cy="10" r="7"/><path d="M10 9v4.5M10 6.6h.01"/>',
  globe: '<circle cx="10" cy="10" r="7"/><path d="M3 10h14M10 3c2 2 2.8 4.5 2.8 7S12 15 10 17c-2-2-2.8-4.5-2.8-7S8 5 10 3z"/>',
  eye: '<path d="M2.5 10s2.7-5 7.5-5 7.5 5 7.5 5-2.7 5-7.5 5-7.5-5-7.5-5z"/><circle cx="10" cy="10" r="2.2"/>',
  map: '<path d="M7.5 4 3 5.5v10.5l4.5-1.5 5 1.5 4.5-1.5V4l-4.5 1.5z"/><path d="M7.5 4v10.5M12.5 5.5V16"/>',
  key: '<circle cx="7" cy="12" r="3.5"/><path d="m9.5 9.5 6-6M13.5 5.5l2 2"/>',
  layers: '<path d="m10 3 7 4-7 4-7-4z"/><path d="m3 11 7 4 7-4"/>',
  heart: '<path d="M10 16.5s-6-3.6-6-8A3.5 3.5 0 0 1 10 6a3.5 3.5 0 0 1 6 2.5c0 4.4-6 8-6 8z"/>',
  more: '<circle cx="5" cy="10" r=".9"/><circle cx="10" cy="10" r=".9"/><circle cx="15" cy="10" r=".9"/>',
};
const icon = (n, size = 18, sw = 1.5) => `<svg class="ic" width="${size}" height="${size}" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="${sw}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[n] || ''}</svg>`;
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
A.copy = async (el) => { try { await navigator.clipboard.writeText(el.dataset.text); toast(el.dataset.msg || 'Copiado'); } catch { toast('No se pudo copiar', 'error'); } };
A.link = (el) => call('open_link', { url: el.dataset.url }).catch(() => {});
A.folder = (el) => call('open_folder', { id: el.dataset.global ? null : state.params.id, which: el.dataset.which }).catch(() => {});

/* ───────── pie de la barra lateral ───────── */
async function paintFoot() {
  if (!state.info) return;
  $('#foot-node').textContent = state.info.settings.nodeName;
}
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
const SHAPES = { dot: '<span class="dot"></span>', off: '<span class="dot off"></span>', ring: '<span class="ring"></span>', pause: '<span class="pausei"><i></i><i></i></span>', tri: '<span class="tri"></span>', sq: '<span class="sq"></span>' };
const stateHTML = (v, pill = false) => { const s = serverState(v); return `<span class="${pill ? 'statepill' : 'st'}">${SHAPES[s.shape]}${esc(s.label)}</span>`; };
const isActive = (v) => v && ['running', 'starting', 'stopping'].includes(v.status);
