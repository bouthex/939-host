'use strict';
/* 939 Host — interfaz. Sin frameworks: liviana y rápida de abrir. */

const TAURI = window.__TAURI__;
const invoke = (cmd, args) => TAURI.core.invoke(cmd, args);
const listen = (ev, fn) => TAURI.event.listen(ev, fn);

const main = document.getElementById('main');
const state = {
  route: 'home', params: {}, info: null, servers: [], server: null, tab: 'summary',
  versions: null, snapshots: null, hw: null, progress: {}, edit: null, extraProps: '', extraPropsOrig: '',
  open: new Set(), timers: [], wizard: null, backups: [], consoleFollow: true, history: [], histIdx: -1,
};

/* ───────── iconos (trazo 1.5, 16 px, consistentes) ───────── */
const ICONS = {
  plus: '<path d="M8 3v10M3 8h10"/>',
  play: '<path d="M5 3.5v9l7.5-4.5z"/>',
  stop: '<rect x="4" y="4" width="8" height="8" rx="1"/>',
  restart: '<path d="M12.5 6.5A4.6 4.6 0 1 0 13 9.5M12.8 3v3.6H9.2"/>',
  folder: '<path d="M2 4.5c0-.6.4-1 1-1h3l1.4 1.5H13c.6 0 1 .4 1 1V12c0 .6-.4 1-1 1H3c-.6 0-1-.4-1-1z"/>',
  copy: '<rect x="5.5" y="5.5" width="8" height="8" rx="1"/><path d="M10.5 5.5V3.5c0-.6-.4-1-1-1h-6c-.6 0-1 .4-1 1v6c0 .6.4 1 1 1h2"/>',
  search: '<circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5 14 14"/>',
  download: '<path d="M8 2.5v8M4.5 7 8 10.5 11.5 7M3 13.5h10"/>',
  trash: '<path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5"/>',
  bolt: '<path d="M9 1.5 3.5 9H8l-1 5.5L12.5 7H8z"/>',
  refresh: '<path d="M13 8a5 5 0 0 1-8.7 3.4M3 8a5 5 0 0 1 8.7-3.4M11.8 2v2.8H9M4.2 14v-2.8H7"/>',
  share: '<circle cx="4" cy="8" r="1.8"/><circle cx="12" cy="3.8" r="1.8"/><circle cx="12" cy="12.2" r="1.8"/><path d="M5.6 7.1 10.4 4.7M5.6 8.9l4.8 2.4"/>',
  plug: '<path d="M6 2v3M10 2v3M4.5 5h7v2.5a3.5 3.5 0 0 1-7 0zM8 11v3"/>',
};
const icon = (n) => `<svg class="ico" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[n] || ''}</svg>`;

/* ───────── utilidades ───────── */
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const fmtMB = (mb) => (mb == null ? '—' : mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`);
const fmtDate = (iso) => { try { return new Date(iso).toLocaleString('es-AR', { dateStyle: 'medium', timeStyle: 'short' }); } catch { return iso; } };
function fmtUptime(iso) {
  if (!iso) return '—';
  const s = Math.max(0, Math.floor((Date.now() - new Date(iso).getTime()) / 1000));
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60);
  return h ? `${h} h ${m} min` : `${m} min`;
}
const loaderName = (l) => ({ fabric: 'Fabric', forge: 'Forge', neoforge: 'NeoForge' }[l] || 'Vanilla');
const DIFF = { peaceful: 'Pacífico', easy: 'Fácil', normal: 'Normal', hard: 'Difícil' };
const MODE = { survival: 'Supervivencia', creative: 'Creativo', adventure: 'Aventura', spectator: 'Espectador' };
const clone = (o) => JSON.parse(JSON.stringify(o));

function statusOf(v) {
  if (v.installing) return { cls: 's-busy', text: 'Instalando' };
  if (!v.config.installed) return v.lastError ? { cls: 's-error', text: 'Instalación fallida' } : { cls: 's-busy', text: 'Preparando' };
  switch (v.status) {
    case 'running': return { cls: 's-running', text: 'En línea' };
    case 'starting': return { cls: 's-busy', text: 'Iniciando' };
    case 'stopping': return { cls: 's-busy', text: 'Deteniendo' };
    case 'crashed': return { cls: 's-error', text: 'Error' };
    default: return { cls: 's-stopped', text: 'Apagado' };
  }
}
const statusHTML = (v) => { const s = statusOf(v); return `<span class="status ${s.cls}"><span class="shape" aria-hidden="true"></span>${s.text}</span>`; };
const isActive = (v) => ['running', 'starting', 'stopping'].includes(v.status);

function toast(message, kind = 'ok', detail = '') {
  const el = document.createElement('div');
  el.className = `toast ${kind === 'error' ? 'error' : ''}`;
  el.innerHTML = `<div>${esc(message)}${detail ? `<span class="detail">${esc(detail)}</span>` : ''}</div>`;
  document.getElementById('toasts').appendChild(el);
  setTimeout(() => el.remove(), kind === 'error' ? 8000 : 3500);
}
function fail(e) {
  console.error(e);
  const msg = typeof e === 'string' ? e : e?.message || 'Ocurrió un error inesperado.';
  toast(msg, 'error', e?.detail || '');
}
async function call(cmd, args, okMsg) {
  try { const r = await invoke(cmd, args); if (okMsg) toast(okMsg); return r; } catch (e) { fail(e); throw e; }
}

/* Diálogo propio (monocromático). requireText: hay que escribirlo para confirmar. */
function dialog({ title, body, confirm = 'Aceptar', cancel = 'Cancelar', requireText = null }) {
  return new Promise((resolve) => {
    const root = document.getElementById('modal-root');
    root.innerHTML = `<div class="overlay" role="dialog" aria-modal="true" aria-labelledby="dlg-t"><div class="dialog">
      <h2 id="dlg-t">${esc(title)}</h2><div class="muted">${body}</div>
      ${requireText ? `<label class="field">Escribí <span class="mono">${esc(requireText)}</span> para confirmar<input class="input" id="dlg-in" autocomplete="off"></label>` : ''}
      <div class="btn-row" style="justify-content:flex-end">${cancel ? `<button class="btn" id="dlg-no">${esc(cancel)}</button>` : ''}<button class="btn primary" id="dlg-ok" ${requireText ? 'disabled' : ''}>${esc(confirm)}</button></div>
    </div></div>`;
    const done = (v) => { root.innerHTML = ''; resolve(v); };
    const ok = root.querySelector('#dlg-ok');
    root.querySelector('#dlg-no')?.addEventListener('click', () => done(false));
    ok.addEventListener('click', () => done(true));
    const inp = root.querySelector('#dlg-in');
    if (inp) { inp.addEventListener('input', () => { ok.disabled = inp.value.trim() !== requireText; }); inp.focus(); } else ok.focus();
    root.querySelector('.overlay').addEventListener('keydown', (e) => { if (e.key === 'Escape' && cancel) done(false); });
  });
}

function every(ms, fn) { const t = setInterval(fn, ms); state.timers.push(t); }
function clearTimers() { state.timers.forEach(clearInterval); state.timers = []; }

/* ───────── navegación ───────── */
function go(route, params = {}) {
  clearTimers();
  state.route = route; state.params = params;
  document.querySelectorAll('.nav[data-route]').forEach((n) => n.classList.toggle('active', n.dataset.route === (route === 'server' || route === 'create' ? (route === 'server' && state.tab === 'mods' ? 'mods' : 'servers') : route)));
  main.scrollTop = 0;
  render();
}
async function render() {
  try {
    switch (state.route) {
      case 'home': return await renderHome();
      case 'servers': return await renderServers();
      case 'create': return await renderCreate();
      case 'server': return await renderServer();
      case 'node': return await renderNode();
      case 'mods': return await renderModsPage();
      case 'network': return await renderNetwork();
      case 'settings': return await renderSettings();
    }
  } catch (e) { fail(e); }
}

/* ───────── Inicio ───────── */
async function renderHome() {
  const [servers, hw] = await Promise.all([invoke('list_servers'), state.hw ? Promise.resolve(state.hw) : invoke('hardware_snapshot')]);
  state.servers = servers; state.hw = hw;
  const live = await invoke('hardware_live', {});
  const online = servers.filter((s) => s.status === 'running').length;
  const players = servers.reduce((n, s) => n + s.players.length, 0);
  main.innerHTML = `<div class="page">
    <div class="page-head"><div><h1>Inicio</h1><div class="sub">${esc(state.info.settings.nodeName)} · ${servers.length} ${servers.length === 1 ? 'servidor' : 'servidores'}</div></div>
      <button class="btn primary" data-action="new">${icon('plus')}Crear servidor</button></div>
    <div class="kpis">
      <div class="card kpi"><span class="label">Servidores en línea</span><span class="value">${online} <small>de ${servers.length}</small></span></div>
      <div class="card kpi"><span class="label">Jugadores conectados</span><span class="value">${players}</span></div>
      <div class="card kpi"><span class="label">RAM libre ahora</span><span class="value" id="k-ram">${fmtMB(live.ramAvailableMb)} <small>de ${fmtMB(live.ramTotalMb)}</small></span></div>
      <div class="card kpi"><span class="label">CPU por núcleo</span><span class="value">${hw.benchmark ? hw.benchmark.cpuSingleScore : '—'} <small>${hw.benchmark ? 'puntos' : 'sin medir'}</small></span></div>
    </div>
    ${servers.length ? serversTable(servers) : emptyServers()}
    <div class="grid-2">
      <div class="card pad stack"><div class="spread"><h2>Este equipo</h2><a class="small muted" data-action="route" data-route="node" style="cursor:pointer;text-decoration:underline">Ver detalle</a></div>
        <div class="kv"><span class="k">Evaluación</span><span class="strong">${esc(hw.assessment.rating)}</span></div>
        <div class="kv"><span class="k">CPU</span><span>${esc(hw.cpuModel)}</span></div>
        <div class="kv"><span class="k">RAM recomendada por servidor</span><span class="mono">${fmtMB(hw.assessment.recommendedServerRamMb)}</span></div></div>
      <div class="card pad stack"><div class="spread"><h2>Jugar con amigos</h2><a class="small muted" data-action="route" data-route="network" style="cursor:pointer;text-decoration:underline">Red</a></div>
        <div class="small muted">Compartí un servidor desde su Resumen e invitá a tus amigos como coanfitriones: guardan una copia sincronizada y pueden hostearlo cuando tu PC no esté. Todos entran siempre por la misma dirección.</div></div>
    </div></div>`;
  if (servers.some(isActive)) every(10000, () => state.route === 'home' && refreshList());
}
function emptyServers() {
  return `<div class="empty"><h2>Todavía no hay servidores</h2><div class="muted">Creá el primero: elegís versión, loader y RAM, y 939 Host descarga y prepara todo.</div><button class="btn primary" data-action="new">Crear servidor</button></div>`;
}
function serversTable(servers) {
  return `<div class="card"><div class="table-wrap"><table>
    <thead><tr><th>Nombre</th><th>Versión</th><th>Estado</th><th>Jugadores</th><th>MSPT</th><th>RAM</th><th>Puerto</th><th><span class="hidden">Acción</span></th></tr></thead>
    <tbody>${servers.map((s) => `<tr class="clickable" data-action="open" data-id="${esc(s.config.id)}">
      <td><div class="cell-title">${esc(s.config.name)}</div><div class="cell-sub">${esc(s.config.description || s.config.identity.accessCode || '')}</div></td>
      <td class="mono small">${loaderName(s.config.loader)} ${esc(s.config.mcVersion)}</td>
      <td>${statusHTML(s)}</td>
      <td class="mono">${s.status === 'running' ? `${s.players.length}/${s.config.maxPlayers}` : '—'}</td>
      <td class="mono">${s.mspt != null ? `${s.mspt.toFixed(1)} ms` : '—'}</td>
      <td class="mono">${s.memoryUsedMb != null ? fmtMB(s.memoryUsedMb) : fmtMB(s.config.memoryMb)}</td>
      <td class="mono">${s.config.port}</td>
      <td style="text-align:right">${quickAction(s)}</td></tr>`).join('')}</tbody></table></div></div>`;
}
function quickAction(s) {
  if (!s.config.installed || s.installing) return '';
  if (s.status === 'running' || s.status === 'starting') return `<button class="btn sm" data-action="stop" data-id="${esc(s.config.id)}">Detener</button>`;
  if (s.status === 'stopping') return `<button class="btn sm" disabled>Deteniendo…</button>`;
  return `<button class="btn sm" data-action="start" data-id="${esc(s.config.id)}">Iniciar</button>`;
}
async function refreshList() { if (state.route === 'home') await renderHome(); else if (state.route === 'servers') await renderServers(); }

/* ───────── Servidores ───────── */
async function renderServers() {
  const servers = await invoke('list_servers');
  state.servers = servers;
  main.innerHTML = `<div class="page">
    <div class="page-head"><div><h1>Servidores</h1><div class="sub">Cada servidor tiene su propia carpeta, mundo y configuración.</div></div>
      <button class="btn primary" data-action="new">${icon('plus')}Crear servidor</button></div>
    ${servers.length ? serversTable(servers) : emptyServers()}</div>`;
  if (servers.some(isActive)) every(10000, () => state.route === 'servers' && refreshList());
}

/* ───────── Crear servidor ───────── */
async function renderCreate() {
  if (!state.wizard) {
    const hw = state.hw || (state.hw = await invoke('hardware_snapshot'));
    state.wizard = {
      step: 1, name: '', description: '', mcVersion: '', loader: 'vanilla', support: null,
      worldMode: 'new', seed: '', difficulty: 'normal', gamemode: 'survival', importPath: '', importOk: false,
      memoryMb: hw.assessment.recommendedServerRamMb, maxMem: hw.assessment.maxServerRamMb, maxPlayers: 8,
      viewDistance: 10, simulationDistance: 8, port: await invoke('suggest_port'), onlineMode: true, whitelist: true,
      eula: false, creating: false, snapshots: false,
    };
  }
  if (!state.versions) state.versions = await call('list_versions', { includeSnapshots: false });
  const w = state.wizard;
  if (!w.mcVersion && state.versions.length) w.mcVersion = state.versions[0].id;
  drawCreate();
}
function drawCreate() {
  const w = state.wizard;
  const steps = ['Básico', 'Mundo', 'Rendimiento y red', 'Revisar'];
  const versions = (w.snapshots ? state.snapshots : state.versions) || [];
  let body = '';
  if (w.step === 1) body = `
    <div class="form-grid">
      <label class="field full">Nombre del servidor<input class="input" data-w="name" maxlength="40" value="${esc(w.name)}" placeholder="Ej.: 939-ZOMBIES" autofocus></label>
      <label class="field full">Descripción <span class="hint">Opcional</span><input class="input" data-w="description" maxlength="120" value="${esc(w.description)}"></label>
      <label class="field">Versión de Minecraft<select class="input" data-w="mcVersion">${versions.map((v) => `<option value="${esc(v.id)}" ${v.id === w.mcVersion ? 'selected' : ''}>${esc(v.id)}${v.kind === 'snapshot' ? ' (snapshot)' : ''}</option>`).join('')}</select></label>
      <div class="field">&nbsp;<label class="check"><input type="checkbox" data-w="snapshots" ${w.snapshots ? 'checked' : ''}>Mostrar snapshots<span class="hint"></span></label></div>
    </div>
    <div class="stack"><h3>Loader</h3><div class="choice-grid">
      <button class="choice ${w.loader === 'vanilla' ? 'on' : ''}" data-action="loader" data-v="vanilla"><span class="t">Vanilla</span><span class="d">Minecraft oficial, sin mods.</span></button>
      <button class="choice ${w.loader === 'fabric' ? 'on' : ''}" data-action="loader" data-v="fabric"><span class="t">Fabric</span><span class="d">${loaderText('fabric', 'Loader liviano y rápido para mods.')}</span></button>
      <button class="choice ${w.loader === 'neoforge' ? 'on' : ''}" data-action="loader" data-v="neoforge"><span class="t">NeoForge</span><span class="d">${loaderText('neoforge', 'Para modpacks grandes basados en NeoForge.')}</span></button>
      <button class="choice ${w.loader === 'forge' ? 'on' : ''}" data-action="loader" data-v="forge"><span class="t">Forge</span><span class="d">${loaderText('forge', 'Para mods y modpacks clásicos de Forge.')}</span></button>
    </div></div>`;
  if (w.step === 2) body = `
    <div class="seg" role="group" aria-label="Origen del mundo"><button class="${w.worldMode === 'new' ? 'on' : ''}" data-action="worldmode" data-v="new">Mundo nuevo</button><button class="${w.worldMode === 'import' ? 'on' : ''}" data-action="worldmode" data-v="import">Importar una copia</button></div>
    ${w.worldMode === 'new' ? `<div class="form-grid">
      <label class="field">Dificultad<select class="input" data-w="difficulty">${Object.entries(DIFF).map(([k, v]) => `<option value="${k}" ${k === w.difficulty ? 'selected' : ''}>${v}</option>`).join('')}</select></label>
      <label class="field">Modo de juego<select class="input" data-w="gamemode">${Object.entries(MODE).map(([k, v]) => `<option value="${k}" ${k === w.gamemode ? 'selected' : ''}>${v}</option>`).join('')}</select></label>
      <label class="field full">Semilla <span class="hint">Vacío = aleatoria</span><input class="input mono" data-w="seed" value="${esc(w.seed)}"></label></div>`
    : `<div class="stack"><div class="banner info"><span class="mark"></span><div class="body"><strong>Se copia, nunca se modifica el original</strong><span class="small muted">Elegí la carpeta de un mundo (la que tiene level.dat). Si está abierto en Minecraft, salí al menú antes de copiarlo.</span></div></div>
      <div class="row"><button class="btn" data-action="pick-world">Elegir carpeta…</button><span class="small ${w.importOk ? '' : 'muted'} mono">${esc(w.importPath || 'Ninguna carpeta elegida')}</span></div>
      <div class="form-grid"><label class="field">Dificultad<select class="input" data-w="difficulty">${Object.entries(DIFF).map(([k, v]) => `<option value="${k}" ${k === w.difficulty ? 'selected' : ''}>${v}</option>`).join('')}</select></label>
      <label class="field">Modo de juego<select class="input" data-w="gamemode">${Object.entries(MODE).map(([k, v]) => `<option value="${k}" ${k === w.gamemode ? 'selected' : ''}>${v}</option>`).join('')}</select></label></div></div>`}`;
  if (w.step === 3) body = `
    <div class="field"><div class="range-head"><span>RAM asignada</span><span class="mono">${fmtMB(w.memoryMb)}</span></div>
      <input type="range" min="1024" max="${w.maxMem}" step="512" value="${w.memoryMb}" data-w="memoryMb" data-type="int">
      <span class="hint">Recomendado para esta PC: ${fmtMB(state.hw.assessment.recommendedServerRamMb)}. Más RAM no es más rendimiento: alcanza con que no falte.</span></div>
    <div class="form-grid">
      <div class="field"><div class="range-head"><span>Distancia de visión</span><span class="mono">${w.viewDistance} chunks</span></div><input type="range" min="2" max="32" value="${w.viewDistance}" data-w="viewDistance" data-type="int"></div>
      <div class="field"><div class="range-head"><span>Distancia de simulación</span><span class="mono">${w.simulationDistance} chunks</span></div><input type="range" min="2" max="32" value="${w.simulationDistance}" data-w="simulationDistance" data-type="int"><span class="hint">Simular menos que lo que se ve es lo que más ayuda con muchos mobs.</span></div>
      <label class="field">Jugadores máximos<input class="input mono" type="number" min="1" max="500" data-w="maxPlayers" data-type="int" value="${w.maxPlayers}"></label>
      <label class="field">Puerto<input class="input mono" type="number" min="1024" max="65535" data-w="port" data-type="int" value="${w.port}"><span class="hint">Sugerido: libre en esta PC.</span></label>
      <label class="check full"><input type="checkbox" data-w="onlineMode" ${w.onlineMode ? 'checked' : ''}><span>Verificar cuentas con Mojang (online-mode)<span class="hint">Recomendado. Solo cuentas oficiales de Minecraft pueden entrar.</span></span></label>
      <label class="check full"><input type="checkbox" data-w="whitelist" ${w.whitelist ? 'checked' : ''}><span>Lista blanca activa<span class="hint">Solo entran los jugadores que agregues.</span></span></label>
    </div>`;
  if (w.step === 4) body = `
    <div class="card pad stack">${[
      ['Nombre', esc(w.name)], ['Versión', `${loaderName(w.loader)} ${esc(w.mcVersion)}`],
      ['Mundo', w.worldMode === 'import' ? `Copia de <span class="mono small">${esc(w.importPath)}</span>` : `Nuevo · ${DIFF[w.difficulty]} · ${MODE[w.gamemode]}`],
      ['RAM', fmtMB(w.memoryMb)], ['Distancias', `${w.viewDistance} visión · ${w.simulationDistance} simulación`],
      ['Red', `Puerto ${w.port} · ${w.maxPlayers} jugadores · ${w.onlineMode ? 'cuentas verificadas' : 'sin verificar cuentas'}`],
      ['Guardado', 'Relevo y puntos de restauración cada 15 min (configurable)'],
    ].map(([k, v]) => `<div class="kv"><span class="k">${k}</span><span>${v}</span></div>`).join('')}</div>
    <label class="check"><input type="checkbox" data-w="eula" ${w.eula ? 'checked' : ''}><span>Acepto el <a data-action="eula" style="text-decoration:underline;cursor:pointer">EULA de Minecraft</a><span class="hint">Mojang exige aceptarlo para ejecutar un servidor. 939 Host lo registra con fecha.</span></span></label>
    <div class="small muted">Al crear, 939 Host descarga el servidor oficial y el Java que corresponde, y verifica cada archivo antes de usarlo.</div>`;

  main.innerHTML = `<div class="page">
    <div class="crumbs"><a data-action="route" data-route="servers">Servidores</a> / Crear servidor</div>
    <div class="page-head"><h1>Crear servidor</h1></div>
    <div class="steps">${steps.map((s, i) => `${i ? '<span class="step-sep"></span>' : ''}<span class="step-pill ${w.step === i + 1 ? 'on' : w.step > i + 1 ? 'done' : ''}"><span class="n">${i + 1}</span>${s}</span>`).join('')}</div>
    <div class="wizard"><div class="stack" style="gap:20px">${body}
      <div class="btn-row" style="justify-content:space-between;border-top:1px solid var(--light);padding-top:18px">
        <button class="btn" data-action="${w.step === 1 ? 'route' : 'wback'}" data-route="servers">${w.step === 1 ? 'Cancelar' : 'Atrás'}</button>
        ${w.step < 4 ? `<button class="btn primary" data-action="wnext" ${stepValid() ? '' : 'disabled'}>Continuar</button>` : `<button class="btn primary" data-action="wcreate" ${w.eula && !w.creating ? '' : 'disabled'}>${w.creating ? 'Creando…' : 'Crear servidor'}</button>`}
      </div></div>
      <aside class="card pad summary stack"><h3>Resumen</h3>
        <div class="kv"><span class="k">Nombre</span><span>${esc(w.name) || '—'}</span></div>
        <div class="kv"><span class="k">Versión</span><span class="mono small">${loaderName(w.loader)} ${esc(w.mcVersion)}</span></div>
        <div class="kv"><span class="k">Mundo</span><span>${w.worldMode === 'import' ? 'Importado' : 'Nuevo'}</span></div>
        <div class="kv"><span class="k">RAM</span><span class="mono">${fmtMB(w.memoryMb)}</span></div>
        <div class="kv"><span class="k">Puerto</span><span class="mono">${w.port}</span></div>
      </aside></div></div>`;
}
function loaderText(loader, idle) {
  const w = state.wizard;
  const f = w.support && w.support.loader === loader ? w.support : null;
  if (!f) return idle;
  if (f.loading) return 'Comprobando compatibilidad…';
  if (f.error) return `No se pudo comprobar ${loaderName(loader)} (sin conexión).`;
  return f.supported ? `Compatible · ${esc(f.loaderVersion)}${f.unverified ? ' · instalación no probada por 939 Host todavía' : ''}` : `Todavía no hay ${loaderName(loader)} para ${esc(w.mcVersion)}.`;
}
function stepValid() {
  const w = state.wizard;
  if (w.step === 1) return w.name.trim().length > 0 && !!w.mcVersion && (w.loader === 'vanilla' || (w.support && w.support.loader === w.loader && w.support.supported));
  if (w.step === 2) return w.worldMode === 'new' || w.importOk;
  if (w.step === 3) return w.port >= 1024 && w.port <= 65535 && w.maxPlayers >= 1 && w.maxPlayers <= 500;
  return true;
}
async function checkLoader() {
  const w = state.wizard;
  const loader = w.loader, mc = w.mcVersion;
  if (loader === 'vanilla') { w.support = null; drawCreate(); return; }
  w.support = { loader, loading: true };
  drawCreate();
  let r;
  try { r = { loader, ...(await invoke('loader_support', { loader, mcVersion: mc })) }; } catch (e) { r = { loader, error: true }; }
  if (w.loader === loader && w.mcVersion === mc) w.support = r;
  if (state.route === 'create') drawCreate();
}

/* ───────── Detalle de servidor ───────── */
async function renderServer() {
  const id = state.params.id;
  state.server = await invoke('get_server', { id });
  if (!state.edit || state.edit.id !== id) { state.edit = null; }
  main.innerHTML = `<div class="page">
    <div class="crumbs"><a data-action="route" data-route="servers">Servidores</a> / ${esc(state.server.config.name)}</div>
    <div id="srv-header"></div>
    <div class="tabs" role="tablist">${[['summary', 'Resumen'], ['console', 'Consola'], ['players', 'Jugadores'], ['mods', 'Mods'], ['settings', 'Ajustes'], ['backups', 'Restauración']].map(([k, v]) => `<button class="tab ${state.tab === k ? 'active' : ''}" role="tab" data-action="tab" data-v="${k}">${v}</button>`).join('')}</div>
    <div id="srv-tab"></div></div>`;
  drawServerHeader();
  await drawTab();
  every(5000, () => { if (state.route === 'server' && state.server && (isActive(state.server) || state.server.installing)) refreshServer(); });
}
function drawServerHeader() {
  const v = state.server, c = v.config;
  const prog = state.progress[c.id];
  const active = isActive(v);
  const primary = !c.installed || v.installing ? `<button class="btn primary" disabled>Iniciar</button>`
    : active ? `<button class="btn primary" data-action="stop" data-id="${esc(c.id)}" ${v.status === 'stopping' ? 'disabled' : ''}>${icon('stop')}${v.status === 'stopping' ? 'Deteniendo…' : 'Detener'}</button>`
      : `<button class="btn primary" data-action="start" data-id="${esc(c.id)}">${icon('play')}Iniciar</button>`;
  document.getElementById('srv-header').innerHTML = `
    <div class="page-head"><div class="stack" style="gap:8px"><h1>${esc(c.name)}</h1>
      <div class="row" style="flex-wrap:wrap;gap:18px;color:var(--dark)">${statusHTML(v)}<span class="mono small">${loaderName(c.loader)} ${esc(c.mcVersion)}${c.javaMajor ? ` · Java ${c.javaMajor}` : ''}</span><span class="small">localhost:${c.port}</span>${v.status === 'running' ? `<span class="small">Encendido hace ${fmtUptime(v.startedAt)}</span>` : ''}</div></div>
      <div class="btn-row">${active ? `<button class="btn" data-action="restart" data-id="${esc(c.id)}" ${v.status !== 'running' ? 'disabled' : ''}>${icon('restart')}Reiniciar</button>` : ''}
        <button class="btn" data-action="folder" data-which="server">${icon('folder')}Carpeta</button>${primary}</div></div>
    ${v.installing || (prog && !prog.done) ? `<div class="card pad stack"><div class="spread"><strong>${esc(prog?.message || 'Preparando instalación')}</strong><span class="mono small">${Math.round((prog?.fraction || 0) * 100)}%</span></div><div class="progress"><div style="width:${Math.round((prog?.fraction || 0) * 100)}%"></div></div></div>` : ''}
    ${v.lastError && !v.installing ? `<div class="banner"><span class="mark"></span><div class="body"><strong>${!c.installed ? 'La instalación no se completó' : 'El servidor se detuvo con un error'}</strong><span class="small">${esc(v.lastError)}</span></div>${!c.installed ? `<button class="btn sm" data-action="reinstall">Reintentar</button>` : `<button class="btn sm" data-action="tab" data-v="console">Ver consola</button>`}</div>` : ''}
    ${v.needsRestart ? `<div class="banner info"><span class="mark"></span><div class="body"><strong>Hay cambios guardados que se aplican al reiniciar</strong></div><button class="btn sm" data-action="restart" data-id="${esc(c.id)}">Reiniciar ahora</button></div>` : ''}`;
}
async function refreshServer() {
  try {
    state.server = await invoke('get_server', { id: state.params.id });
    drawServerHeader();
    if (state.tab === 'summary') drawSummary();
  } catch (e) { /* el servidor pudo haberse borrado */ }
}
async function drawTab() {
  if (state.tab === 'summary') return drawSummary();
  if (state.tab === 'console') return drawConsole();
  if (state.tab === 'settings') return drawSettings();
  if (state.tab === 'backups') return drawBackups();
  if (state.tab === 'mods') return drawMods();
  if (state.tab === 'players') return drawPlayers();
}

async function drawPlayers() {
  const el = document.getElementById('srv-tab');
  if (!el) return;
  let list = [];
  try { list = await invoke('players_list', { id: state.params.id }); } catch (e) { el.innerHTML = `<div class="small">${esc(e.message || e)}</div>`; return; }
  const c = state.server.config;
  const running = state.server.status === 'running';
  el.innerHTML = `<div class="stack">
    <div class="card pad stack"><h2>Agregar o administrar un jugador</h2>
      <div class="small muted">${running ? 'El servidor está encendido: los cambios se aplican al instante.' : 'El servidor está apagado: los cambios se guardan y se aplican al encender.'} ${c.whitelist ? 'La lista blanca está activa: solo entran los jugadores de la lista.' : 'La lista blanca está desactivada (se cambia en Ajustes → Jugadores y red).'}</div>
      <div class="row"><input class="input mono" id="pl-name" placeholder="Nombre de Minecraft" maxlength="16" autocomplete="off" spellcheck="false">
        <button class="btn primary" data-action="pl" data-a="whitelist_add">Agregar a la lista blanca</button><button class="btn" data-action="pl" data-a="op">Hacer operador</button></div></div>
    <div class="card"><table class="table"><thead><tr><th>Jugador</th><th>Estado</th><th>Permisos</th><th><span class="hidden">Acciones</span></th></tr></thead><tbody>
    ${list.length ? list.map((p) => `<tr><td><strong>${esc(p.name)}</strong>${p.uuid ? `<div class="small muted mono">${esc(p.uuid)}</div>` : ''}</td>
      <td>${p.online ? '<span class="badge issue">Conectado</span>' : p.banned ? `<span class="badge outline">Baneado</span>${p.banReason ? ` <span class="small muted">${esc(p.banReason)}</span>` : ''}` : '<span class="small muted">Desconectado</span>'}</td>
      <td class="small">${[p.whitelisted ? 'Lista blanca' : '', p.opLevel ? `Operador (nivel ${p.opLevel})` : ''].filter(Boolean).join(' · ') || '—'}</td>
      <td><div class="btn-row" style="justify-content:flex-end">
        <button class="btn sm ghost" data-action="pl" data-a="${p.whitelisted ? 'whitelist_remove' : 'whitelist_add'}" data-name="${esc(p.name)}">${p.whitelisted ? 'Quitar de la lista' : 'A la lista blanca'}</button>
        <button class="btn sm ghost" data-action="pl" data-a="${p.opLevel ? 'deop' : 'op'}" data-name="${esc(p.name)}">${p.opLevel ? 'Quitar operador' : 'Operador'}</button>
        ${p.online ? `<button class="btn sm ghost" data-action="pl" data-a="kick" data-name="${esc(p.name)}">Expulsar</button>` : ''}
        <button class="btn sm ghost" data-action="pl" data-a="${p.banned ? 'pardon' : 'ban'}" data-name="${esc(p.name)}">${p.banned ? 'Desbanear' : 'Banear'}</button></div></td></tr>`).join('')
      : '<tr><td colspan="4" class="small muted">Todavía no entró nadie y no hay jugadores en las listas.</td></tr>'}
    </tbody></table></div></div>`;
}
const ROLE = { propietario: 'Propietaria', coanfitrion: 'Coanfitriona', jugador: 'Jugador' };
function ago(iso) {
  if (!iso) return 'nunca';
  const m = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 60000));
  return m < 1 ? 'recién' : m < 60 ? `hace ${m} min` : m < 1440 ? `hace ${Math.round(m / 60)} h` : `hace ${Math.round(m / 1440)} d`;
}
async function drawShare() {
  const el = document.getElementById('share-card');
  if (!el) return;
  let st;
  try { st = await invoke('share_status', { id: state.params.id }); } catch (e) { el.innerHTML = `<h2>Grupo</h2><div class="small">${esc(e.message || e)}</div>`; return; }
  state.share = st;
  const g = st.group;
  const head = `<div class="spread"><div class="row">${icon('share')}<h2>Grupo y anfitriona</h2></div>
      <label class="check" style="align-items:center"><input type="checkbox" id="share-toggle" ${st.enabled ? 'checked' : ''}><span>${st.enabled ? 'Compartido' : 'Solo esta PC'}</span></label></div>`;
  if (!st.enabled || !g) {
    el.innerHTML = `${head}<div class="small muted">Compartilo para que tus amigos jueguen desde sus casas sin abrir puertos y para que el servidor pueda pasar de una PC a otra sin perder progreso. Conexión directa cifrada${st.relays ? '; relays públicos gratuitos solo como último recurso' : ' (relays públicos desactivados en Configuración)'}.</div>
      <div class="btn-row"><button class="btn primary" data-action="invite" data-role="coanfitrion">${icon('plus')}Invitar coanfitriona</button><button class="btn" data-action="invite" data-role="jugador">${icon('plus')}Invitar jugador</button></div>`;
  } else {
    const l = g.lease;
    const hostName = l ? (l.host === g.me ? 'Esta PC' : (l.hostName || 'otra PC')) : '—';
    const hostState = !l ? '' : l.state === 'released' ? 'libre: nadie lo está hosteando' : g.hostOnline ? 'en línea' : 'no responde';
    const online = g.members.filter((m) => m.online).length;
    const eligible = g.members.filter((m) => !m.isMe && m.online && m.canHost);
    el.innerHTML = `${head}
      <div class="code-box"><span>127.0.0.1:${st.routePort}</span><button class="btn sm" data-action="copy" data-text="127.0.0.1:${st.routePort}">${icon('copy')}Copiar</button></div>
      <div class="small muted">Dirección para jugar desde cualquier PC del grupo. No cambia aunque cambie la PC anfitriona.</div>
      ${g.busy ? `<div class="banner info"><span class="mark"></span><div class="body"><strong>${esc(g.busy)}…</strong></div></div>` : ''}
      ${g.fenced ? `<div class="banner"><span class="mark"></span><div class="body"><strong>Servidor detenido en esta PC por seguridad</strong><span class="small">Se perdió el contacto con el grupo o otra PC tomó el servidor. Tu copia quedó como punto de seguridad.</span></div></div>` : ''}
      <div class="kv"><span class="k">Anfitriona</span><span><strong>${esc(hostName)}</strong>${l ? ` <span class="small muted">· ${esc(hostState)} · época ${l.epoch}</span>` : ''}</span></div>
      ${g.myLatest ? `<div class="kv"><span class="k">Último estado en esta PC</span><span>${esc(ago(g.myLatest.createdAt))}</span></div>` : ''}
      <div class="btn-row">
        ${g.iAmHost ? `<button class="btn" data-action="handover" ${eligible.length ? '' : 'disabled title="No hay otra PC coanfitriona conectada"'}>${icon('share')}Pasar a…</button><button class="btn" data-action="release">Guardar y soltar</button>` : ''}
        ${!g.iAmHost && g.hasCopy && l && l.state === 'hosting' && !g.hostOnline ? `<button class="btn" data-action="take-here">Recuperar aquí</button>` : ''}
        ${!g.iAmHost && g.hasCopy ? `<button class="btn ghost" data-action="group-sync">Sincronizar ahora</button>` : ''}
      </div>
      ${g.canStart && g.canStart.Err && !g.iAmHost ? `<div class="small muted">${esc(g.canStart.Err)}</div>` : ''}
      <h3>PCs del grupo · ${online}/${g.members.length} conectadas</h3>
      ${g.members.map((m) => `<div class="kv member"><span class="row" style="gap:8px"><span class="dot ${m.online ? 'on' : ''}" aria-label="${m.online ? 'conectada' : 'desconectada'}"></span><span>${esc(m.name)}${m.isMe ? ' <span class="small muted">(vos)</span>' : ''}</span>${m.isHost ? '<span class="badge issue">Anfitriona</span>' : ''}<span class="badge outline">${ROLE[m.role] || m.role}</span></span>
        <span class="row small muted" style="gap:10px">${m.path ? `<span>${m.path === 'relay' ? 'por relay' : 'directa'}${m.rttMs != null ? ` · ${m.rttMs} ms` : ''}</span>` : ''}${m.canHost ? `<span>estado ${esc(ago(m.latestAt))}</span>` : ''}${!m.isMe && m.role !== 'propietario' && !m.isHost ? `<button class="btn sm ghost" data-action="member-remove" data-node="${esc(m.nodeId)}">Quitar</button>` : ''}</span></div>`).join('')}
      <details class="adv"><summary>Opciones del grupo</summary><div class="stack" style="padding-top:10px">
        <label class="check"><input type="checkbox" id="opt-failover" ${g.autoFailover ? 'checked' : ''}><span>Recuperación automática: si la anfitriona deja de responder, las PCs conectadas eligen otra por mayoría (hacen falta ${g.majority} de ${g.members.length}).${g.members.length < 3 ? ' Con dos PCs no hay mayoría posible: queda «Recuperar aquí».' : ''}</span></label>
        <label class="check"><input type="checkbox" id="opt-handover" ${g.handoverOnQuit ? 'checked' : ''}><span>Al salir de 939 Host, pasar el servidor a otra PC conectada en vez de apagarlo.</span></label>
      </div></details>
      <div class="btn-row"><button class="btn primary" data-action="invite" data-role="coanfitrion">${icon('plus')}Invitar coanfitriona</button><button class="btn" data-action="invite" data-role="jugador">${icon('plus')}Invitar jugador</button></div>
      ${st.invites.length ? `<h3>Invitaciones sin usar</h3>${st.invites.map((i) => `<div class="kv"><span>${ROLE[i.role] || 'Jugador'} <span class="small muted">${esc(fmtDate(i.createdAt))}</span></span><button class="btn sm ghost" data-action="invite-revoke" data-secret="${esc(i.secret)}">Revocar</button></div>`).join('')}` : ''}`;
    const fo = el.querySelector('#opt-failover'), ho = el.querySelector('#opt-handover');
    const opts = () => invoke('group_options', { id: state.params.id, autoFailover: fo.checked, handoverOnQuit: ho.checked }).then(() => toast('Opciones del grupo guardadas')).catch(fail);
    fo.addEventListener('change', opts);
    ho.addEventListener('change', opts);
  }
  el.querySelector('#share-toggle').addEventListener('change', async (e) => {
    try { await invoke('share_set', { id: state.params.id, enabled: e.target.checked }); toast(e.target.checked ? 'Servidor compartido con el grupo' : 'Dejaste de compartir'); } catch (err) { fail(err); }
    drawShare();
  });
}

function drawSummary() {
  setTimeout(drawShare, 0);
  const v = state.server, c = v.config;
  const el = document.getElementById('srv-tab');
  if (!el) return;
  el.innerHTML = `<div class="stack" style="gap:20px">
    <div class="kpis five">
      <div class="card kpi"><span class="label">Jugadores</span><span class="value">${v.status === 'running' ? v.players.length : '—'} <small>/ ${c.maxPlayers}</small></span></div>
      <div class="card kpi"><span class="label">MSPT promedio · p95</span><span class="value">${v.mspt != null ? v.mspt.toFixed(1) : '—'} <small>${v.msptP95 != null ? `/ ${v.msptP95.toFixed(1)} ms` : 'ms'}</small></span></div>
      <div class="card kpi"><span class="label">Memoria</span><span class="value">${v.memoryUsedMb != null ? fmtMB(v.memoryUsedMb) : '—'} <small>/ ${fmtMB(c.memoryMb)}</small></span></div>
      <div class="card kpi"><span class="label">Tamaño del mundo</span><span class="value">${fmtMB(v.worldSizeMb)}</span></div>
      <div class="card kpi"><span class="label">Encendido</span><span class="value" style="font-size:18px">${v.status === 'running' ? fmtUptime(v.startedAt) : '—'}</span></div>
    </div>
    <div class="grid-2">
      <div class="stack">
        <div class="card pad stack" id="share-card"><div class="row">${icon('share')}<h2>Grupo y anfitriona</h2></div><div class="small muted">Cargando…</div></div>
        <div class="card pad stack"><h2>Cómo entrar</h2>
          <div class="kv"><span class="k">Desde esta PC</span><span class="mono">localhost:${c.port}</span></div>
          <div class="kv"><span class="k">Desde tu red local</span><span>IP de esta PC + puerto ${c.port}</span></div>
          <div class="kv"><span class="k">Desde otras casas</span><span>Invitación de 939 Host → dirección fija del grupo</span></div></div>
        <div class="card pad stack"><h2>Identidad permanente</h2>
          <div class="small muted">Identifica a este servidor lógico en todas las PCs del grupo. No depende de la IP de ninguna PC: el servidor sigue siendo el mismo aunque cambie la anfitriona.</div>
          <div class="code-box"><span>${esc(c.identity.accessCode)}</span><button class="btn sm" data-action="copy" data-text="${esc(c.identity.accessCode)}">${icon('copy')}Copiar</button></div></div>
      </div>
      <div class="stack">
        <div class="card pad stack"><h2>Jugadores conectados</h2>${v.players.length ? v.players.map((p) => `<div class="kv"><span>${esc(p)}</span></div>`).join('') : `<div class="small muted">${v.status === 'running' ? 'Nadie conectado.' : 'El servidor está apagado.'}</div>`}</div>
        <div class="card pad stack"><h2>Configuración</h2>
          <div class="kv"><span class="k">Dificultad</span><span>${DIFF[c.difficulty] || c.difficulty}</span></div>
          <div class="kv"><span class="k">Modo</span><span>${MODE[c.gamemode] || c.gamemode}</span></div>
          <div class="kv"><span class="k">Distancias</span><span class="mono">${c.viewDistance} / ${c.simulationDistance}</span></div>
          <div class="kv"><span class="k">Perfil de Java</span><span>${{ balanced: 'Equilibrado (G1)', lowlatency: 'Baja latencia (ZGC)', custom: 'Personalizado' }[c.jvmProfile]}</span></div></div>
      </div></div></div>`;
}

async function drawConsole() {
  const id = state.params.id;
  const lines = await invoke('console_history', { id });
  const el = document.getElementById('srv-tab');
  el.innerHTML = `<div class="console"><div class="console-head"><span>Consola · ${esc(state.server.config.name)}</span>
      <div class="btn-row"><label class="small" style="display:flex;gap:6px;align-items:center;color:var(--light)"><input type="checkbox" id="follow" ${state.consoleFollow ? 'checked' : ''} style="accent-color:#fff">Seguir</label><button class="btn sm" data-action="folder" data-which="logs">Logs</button></div></div>
    <div class="console-log" id="console-log" tabindex="0" aria-live="off"></div>
    <div class="console-input"><span class="mono">&gt;</span><input id="console-in" placeholder="${state.server.status === 'running' ? 'Escribí un comando (sin /) y Enter' : 'El servidor está apagado'}" ${state.server.status === 'running' || state.server.status === 'starting' ? '' : 'disabled'} autocomplete="off" spellcheck="false" aria-label="Comando"></div></div>`;
  const log = document.getElementById('console-log');
  log.innerHTML = lines.map(lineHTML).join('');
  log.scrollTop = log.scrollHeight;
  document.getElementById('follow').addEventListener('change', (e) => { state.consoleFollow = e.target.checked; });
  const inp = document.getElementById('console-in');
  inp.addEventListener('keydown', async (e) => {
    if (e.key === 'Enter' && inp.value.trim()) {
      const cmd = inp.value; state.history.unshift(cmd); state.histIdx = -1; inp.value = '';
      appendConsole(`> ${cmd}`);
      try { await invoke('send_command', { id, command: cmd }); } catch (err) { fail(err); }
    } else if (e.key === 'ArrowUp') { if (state.histIdx < state.history.length - 1) inp.value = state.history[++state.histIdx]; e.preventDefault(); }
    else if (e.key === 'ArrowDown') { inp.value = state.histIdx > 0 ? state.history[--state.histIdx] : (state.histIdx = -1, ''); e.preventDefault(); }
  });
  if (!inp.disabled) inp.focus();
}
function lineHTML(l) {
  const cls = l.startsWith('──') ? 'sys' : (l.startsWith('[stderr]') || /\/(ERROR|FATAL)\]/.test(l)) ? 'err' : '';
  return `<div class="${cls}">${esc(l)}</div>`;
}
function appendConsole(line) {
  const log = document.getElementById('console-log');
  if (!log) return;
  log.insertAdjacentHTML('beforeend', lineHTML(line));
  while (log.childElementCount > 2000) log.firstElementChild.remove();
  if (state.consoleFollow) log.scrollTop = log.scrollHeight;
}

/* Ajustes del servidor: copia de trabajo + barra de guardado */
async function drawSettings() {
  const c = state.server.config;
  if (!state.edit || state.edit.id !== c.id) {
    state.edit = clone(c);
    const extra = await invoke('server_properties_extra', { id: c.id }).catch(() => ({}));
    state.extraProps = Object.entries(extra).map(([k, v]) => `${k}=${v}`).join('\n');
    state.extraPropsOrig = state.extraProps;
  }
  const e = state.edit;
  const opt = (map, cur) => Object.entries(map).map(([k, v]) => `<option value="${k}" ${k === cur ? 'selected' : ''}>${v}</option>`).join('');
  const sec = (id, title, basic, adv, badge = '') => `<section class="section" id="sec-${id}"><div class="section-head"><h2>${title}</h2>${badge}</div>${basic}${adv ? `<button class="disclosure" data-action="disclose" data-v="${id}" aria-expanded="${state.open.has(id)}"><span class="chev">▸</span>Opciones avanzadas</button>${state.open.has(id) ? `<div class="adv">${adv}</div>` : ''}` : ''}</section>`;
  const maxMem = state.hw ? state.hw.assessment.maxServerRamMb : 16384;
  document.getElementById('srv-tab').innerHTML = `<div class="settings-layout">
    <nav class="toc" aria-label="Secciones">${[['general', 'General'], ['perf', 'Rendimiento'], ['world', 'Mundo'], ['dist', 'Distancias'], ['net', 'Jugadores y red'], ['save', 'Guardado'], ['expert', 'Experto'], ['danger', 'Zona de peligro']].map(([k, v]) => `<a data-action="toc" data-v="${k}">${v}</a>`).join('')}</nav>
    <div class="stack" style="gap:16px">
      ${sec('general', 'General', `<div class="form-grid">
        <label class="field">Nombre<input class="input" data-bind="name" maxlength="40" value="${esc(e.name)}"></label>
        <label class="field">Versión<input class="input mono" readonly value="${loaderName(e.loader)} ${esc(e.mcVersion)}"><span class="hint">Para cambiar de versión, creá un servidor nuevo importando una copia de este mundo; este queda intacto.</span></label>
        <label class="field full">Descripción<input class="input" data-bind="description" maxlength="120" value="${esc(e.description)}"></label>
        <label class="field full">Mensaje del servidor (MOTD)<input class="input" data-bind="motd" maxlength="120" value="${esc(e.motd)}"></label></div>`)}
      ${sec('perf', 'Rendimiento', `<div class="field"><div class="range-head"><span>RAM asignada</span><span class="mono" id="mem-label">${fmtMB(e.memoryMb)}</span></div>
          <input type="range" min="1024" max="${Math.max(maxMem, e.memoryMb)}" step="512" value="${e.memoryMb}" data-bind="memoryMb" data-type="int" data-label="mem-label"></div>
        <label class="field">Perfil de Java<select class="input" data-bind="jvmProfile">${opt({ balanced: 'Equilibrado (G1) — recomendado', lowlatency: 'Baja latencia (ZGC) — para mucha RAM', custom: 'Personalizado — solo mis argumentos' }, e.jvmProfile)}</select></label>`,
        `<label class="field">Argumentos extra de Java<input class="input mono" data-bind="extraJvmArgs" value="${esc(e.extraJvmArgs)}" placeholder="-Dejemplo=1"><span class="hint">Se agregan al final. Con el perfil Personalizado son los únicos (además de la RAM).</span></label>
        <div class="field">Línea de Java resultante<div class="code-box" style="align-items:flex-start"><span id="jvm-preview" class="small" style="word-break:break-all">…</span></div></div>`, '<span class="badge">Requiere reinicio</span>')}
      ${sec('world', 'Mundo', `<div class="form-grid">
        <label class="field">Dificultad<select class="input" data-bind="difficulty">${opt(DIFF, e.difficulty)}</select></label>
        <label class="field">Modo de juego<select class="input" data-bind="gamemode">${opt(MODE, e.gamemode)}</select></label>
        <label class="check"><input type="checkbox" data-bind="pvp" ${e.pvp ? 'checked' : ''}><span>PvP entre jugadores</span></label>
        <label class="check"><input type="checkbox" data-bind="hardcore" ${e.hardcore ? 'checked' : ''}><span>Hardcore<span class="hint">Una sola vida. Cambia la jugabilidad.</span></span></label></div>`,
        `<div class="form-grid">
        <label class="field">Semilla<input class="input mono" data-bind="levelSeed" value="${esc(e.levelSeed)}"><span class="hint">Solo afecta a mundos nuevos.</span></label>
        <label class="field">Tipo de mundo<select class="input" data-bind="levelType">${opt({ 'minecraft:normal': 'Normal', 'minecraft:flat': 'Plano', 'minecraft:large_biomes': 'Biomas grandes', 'minecraft:amplified': 'Amplificado' }, e.levelType)}</select></label>
        <label class="field">Protección del spawn (bloques)<input class="input mono" type="number" min="0" max="1000" data-bind="spawnProtection" data-type="int" value="${e.spawnProtection}"></label>
        <div class="stack"><label class="check"><input type="checkbox" data-bind="allowFlight" ${e.allowFlight ? 'checked' : ''}><span>Permitir vuelo<span class="hint">Evita expulsiones falsas con mods de movimiento.</span></span></label>
        <label class="check"><input type="checkbox" data-bind="enableCommandBlock" ${e.enableCommandBlock ? 'checked' : ''}><span>Bloques de comandos</span></label></div></div>
        <div class="row"><button class="btn" data-action="import-world">Reemplazar mundo por una copia…</button><span class="small muted">El mundo actual queda en la Papelera de 939 Host.</span></div>`)}
      ${sec('dist', 'Distancias', `<div class="form-grid">
        <div class="field"><div class="range-head"><span>Visión</span><span class="mono" id="vd-label">${e.viewDistance} chunks</span></div><input type="range" min="2" max="32" value="${e.viewDistance}" data-bind="viewDistance" data-type="int" data-label="vd-label" data-suffix=" chunks"></div>
        <div class="field"><div class="range-head"><span>Simulación</span><span class="mono" id="sd-label">${e.simulationDistance} chunks</span></div><input type="range" min="2" max="32" value="${e.simulationDistance}" data-bind="simulationDistance" data-type="int" data-label="sd-label" data-suffix=" chunks"></div></div>
        <div class="small muted">Se ve lejos, pero solo se simula cerca: es lo que más ayuda con hordas grandes.</div>`)}
      ${sec('net', 'Jugadores y red', `<div class="form-grid">
        <label class="field">Puerto<input class="input mono" type="number" min="1024" max="65535" data-bind="port" data-type="int" value="${e.port}"></label>
        <label class="field">Jugadores máximos<input class="input mono" type="number" min="1" max="500" data-bind="maxPlayers" data-type="int" value="${e.maxPlayers}"></label>
        <label class="check"><input type="checkbox" data-bind="onlineMode" ${e.onlineMode ? 'checked' : ''}><span>Verificar cuentas con Mojang<span class="hint">Recomendado.</span></span></label>
        <label class="check"><input type="checkbox" data-bind="whitelist" ${e.whitelist ? 'checked' : ''}><span>Lista blanca<span class="hint">Agregá jugadores con el comando <span class="mono">whitelist add NOMBRE</span> en la consola.</span></span></label></div>`)}
      ${sec('save', 'Guardado y puntos de restauración', `<div class="form-grid">
        <label class="field">Sincronización entre PCs<select class="input" data-bind="syncIntervalMin" data-type="int">${[5, 10, 15, 30, 60].map((m) => `<option value="${m}" ${m === e.syncIntervalMin ? 'selected' : ''}>Cada ${m} minutos</option>`).join('')}</select><span class="hint">Pérdida máxima si el anfitrión se apaga de golpe.</span></label>
        <label class="field">Puntos de restauración<select class="input" data-bind="restoreIntervalMin" data-type="int">${[15, 30, 60, 120].map((m) => `<option value="${m}" ${m === e.restoreIntervalMin ? 'selected' : ''}>Cada ${m} minutos</option>`).join('')}</select><span class="hint">Momentos a los que se puede volver.</span></label></div>
        <div class="banner info"><span class="mark"></span><div class="body"><span class="small">El guardado normal de Minecraft funciona siempre, sin depender de esto. Los puntos de restauración ya están activos: con el servidor encendido se crean solos, guardando solo lo que cambió. La sincronización entre PCs usa esos mismos puntos cuando llegue la red del grupo.</span></div></div>`)}
      ${sec('expert', 'Experto', `<label class="field">Otras claves de server.properties<textarea class="input" id="extra-props" spellcheck="false">${esc(state.extraProps)}</textarea><span class="hint">Una por línea: clave=valor. Las claves que administran las secciones de arriba se ignoran acá.</span></label>
        <div class="row"><button class="btn" data-action="reinstall">Reinstalar archivos del servidor</button><span class="small muted">Vuelve a descargar y verificar el servidor y Java. No toca el mundo.</span></div>`)}
      <section class="section" id="sec-danger"><div class="section-head"><h2>Zona de peligro</h2></div>
        <div class="spread"><div class="stack" style="gap:2px"><strong>Mundo nuevo</strong><span class="small muted">El mundo actual queda como punto de restauración y en la Papelera; al encender se genera uno nuevo.</span></div><button class="btn" data-action="world-new">Mundo nuevo…</button></div>
        <div class="spread"><div class="stack" style="gap:2px"><strong>Informe de diagnóstico</strong><span class="small muted">Un .zip con registros, errores, mods y configuración (sin el mundo ni claves) para pedir ayuda.</span></div><button class="btn" data-action="diag">Generar informe</button></div>
        <div class="spread"><div class="stack" style="gap:2px"><strong>Borrar servidor</strong><span class="small muted">Se mueve a la Papelera de 939 Host (no se elimina del disco).</span></div><button class="btn" data-action="delete">Borrar…</button></div></section>
      <div class="savebar" id="savebar"></div>
    </div></div>`;
  updateSavebar();
  updateJvmPreview();
}
const isDirty = () => state.edit && (JSON.stringify(state.edit) !== JSON.stringify(state.server.config) || state.extraProps !== state.extraPropsOrig);
function updateSavebar() {
  const bar = document.getElementById('savebar');
  if (!bar) return;
  bar.innerHTML = isDirty()
    ? `<span class="strong">Cambios sin guardar</span><div class="btn-row"><button class="btn" data-action="discard">Descartar</button><button class="btn primary" data-action="save">Guardar cambios</button></div>`
    : `<span class="small muted">Todo guardado.</span><span></span>`;
}
let jvmTimer = null;
function updateJvmPreview() {
  clearTimeout(jvmTimer);
  jvmTimer = setTimeout(async () => {
    const el = document.getElementById('jvm-preview');
    if (!el || !state.edit) return;
    try { el.textContent = `java ${await invoke('jvm_preview', { config: state.edit })} -jar … nogui`; } catch { /* nada */ }
  }, 150);
}
async function saveSettings() {
  const e = clone(state.edit);
  const extra = {};
  for (const line of state.extraProps.split('\n')) {
    const t = line.trim();
    if (!t || t.startsWith('#')) continue;
    const i = t.indexOf('=');
    if (i <= 0) { toast(`Línea inválida en propiedades: «${t}»`, 'error'); return; }
    extra[t.slice(0, i).trim()] = t.slice(i + 1).trim();
  }
  e.extraProperties = extra;
  try {
    const saved = await invoke('update_server', { config: e });
    state.server.config = saved; state.edit = clone(saved); state.extraPropsOrig = state.extraProps;
    toast(isActive(state.server) ? 'Guardado. Se aplica al reiniciar el servidor.' : 'Cambios guardados.');
    state.server = await invoke('get_server', { id: saved.id });
    drawServerHeader(); updateSavebar();
  } catch (err) { fail(err); }
}

async function drawBackups() {
  const id = state.params.id, c = state.server.config;
  const [snaps, zips] = await Promise.all([invoke('snapshots_list', { id }), invoke('list_backups', { id })]);
  state.backups = zips;
  const running = state.server.status === 'running';
  const st = snaps.stats;
  const KIND = { auto: 'Automático', manual: 'Manual', seguridad: 'Seguridad' };
  const groups = {};
  for (const s of snaps.items) { const d = new Date(s.createdAt).toLocaleDateString('es-AR', { weekday: 'long', day: 'numeric', month: 'long' }); (groups[d] = groups[d] || []).push(s); }
  document.getElementById('srv-tab').innerHTML = `<div class="stack" style="gap:20px">
    <div class="kpis">
      <div class="card kpi"><span class="label">Puntos de restauración</span><span class="value">${st.snapshots}</span></div>
      <div class="card kpi"><span class="label">Espacio usado</span><span class="value">${fmtMB(st.storeBytes / 1048576)}</span></div>
      <div class="card kpi"><span class="label">Equivale a copias de</span><span class="value">${fmtMB(st.logicalBytes / 1048576)}</span></div>
      <div class="card kpi"><span class="label">Automático cada</span><span class="value">${c.restoreIntervalMin} <small>min</small></span></div>
    </div>
    <div class="card pad stack"><div class="spread"><h2>Crear punto ahora</h2><span class="small muted">${running ? 'Con el servidor encendido: pausa el guardado unos milisegundos, nadie se desconecta.' : 'Guarda solo lo que cambió desde el último punto.'}</span></div>
      <div class="row"><input class="input" id="snap-label" placeholder="Etiqueta (opcional), ej.: antes de la horda" maxlength="80" style="max-width:420px"><button class="btn primary" data-action="snap-take">${icon('plus')}Crear punto</button></div></div>
    <div class="card">${snaps.items.length ? Object.entries(groups).map(([day, items]) => `<div style="padding:12px 16px;border-bottom:1px solid var(--light)" class="small strong">${esc(day.charAt(0).toUpperCase() + day.slice(1))}</div>
      ${items.map((s) => `<div class="mod-row"><span class="mono" style="width:56px">${new Date(s.createdAt).toLocaleTimeString('es-AR', { hour: '2-digit', minute: '2-digit' })}</span>
        <div class="mod-main"><span class="t">${esc(s.label)} <span class="badge outline">${KIND[s.kind] || s.kind}</span></span><span class="d">+${fmtMB(s.newBytes / 1048576)} nuevos · mundo de ${fmtMB(s.totalBytes / 1048576)} · ${s.fileCount} archivos${s.pauseMs ? ` · pausa ${s.pauseMs} ms` : ''}</span></div>
        <div class="btn-row"><button class="btn sm" data-action="snap-restore" data-snap="${esc(s.id)}">Restaurar</button><button class="btn sm ghost" data-action="snap-delete" data-snap="${esc(s.id)}">${icon('trash')}</button></div></div>`).join('')}`).join('')
      : `<div class="empty" style="border:none"><h2>Sin puntos todavía</h2><div class="muted small">Con el servidor encendido se crea uno automático cada ${c.restoreIntervalMin} minutos. También podés crear uno ahora.</div></div>`}</div>
    <div class="small muted">Retención automática: todos los de las últimas 6 h, uno por hora durante 2 días, uno por día durante 14 días y uno por semana durante 8 semanas. Los manuales no se borran solos.</div>
    <details class="card pad"><summary class="strong" style="cursor:pointer">Copias completas (.zip) para exportar</summary>
      <div class="stack" style="margin-top:14px"><div class="small muted">Un archivo .zip independiente con el mundo y la configuración, para guardarlo afuera o compartirlo.</div>
      <div class="row"><button class="btn" data-action="backup">${icon('download')}Crear copia .zip</button><button class="btn ghost" data-action="folder" data-which="backups">${icon('folder')}Abrir carpeta</button></div>
      ${zips.map((b) => `<div class="kv"><span>${esc(fmtDate(b.createdAt))} · ${esc(b.label)} <span class="mono small muted">${fmtMB(b.sizeMb)}</span></span><span class="btn-row"><button class="btn sm" data-action="restore" data-file="${esc(b.file)}">Restaurar</button><button class="btn sm ghost" data-action="del-backup" data-file="${esc(b.file)}">${icon('trash')}</button></span></div>`).join('')}</div></details>
  </div>`;
}

/* ───────── Mods ───────── */
async function renderModsPage() {
  const servers = await invoke('list_servers');
  const fabric = servers.filter((s) => s.config.loader !== 'vanilla');
  main.innerHTML = `<div class="page">
    <div class="page-head"><div><h1>Mods</h1><div class="sub">Buscá e instalá mods de Modrinth con sus dependencias. Solo se ofrecen versiones compatibles con cada servidor.</div></div></div>
    ${fabric.length ? `<div class="server-cards">${fabric.map((s) => `<button class="card hover server-card" data-action="open-mods" data-id="${esc(s.config.id)}">
        <div class="spread"><strong>${esc(s.config.name)}</strong>${statusHTML(s)}</div>
        <span class="mono small muted">${loaderName(s.config.loader)} ${esc(s.config.mcVersion)}</span><span class="small muted">Administrar mods →</span></button>`).join('')}</div>`
      : `<div class="empty"><h2>No hay servidores con mods</h2><div class="muted">Los mods necesitan un servidor Fabric, NeoForge o Forge.</div><button class="btn primary" data-action="new">${icon('plus')}Crear servidor</button></div>`}
    ${servers.length > fabric.length ? `<div class="small muted">${servers.length - fabric.length} servidor(es) Vanilla no usan mods.</div>` : ''}
  </div>`;
}

const SIDE = { server: 'Solo servidor', both: 'Servidor y jugadores', client: 'Solo cliente', unknown: 'Lado desconocido' };
const ISSUE = { duplicate: 'Duplicado', incompatible: 'Otra versión', client_only: 'Solo cliente', unknown: 'No está en Modrinth' };
state.mods = { list: [], updates: {}, results: null, query: '', offline: false, busy: null };

async function drawMods() {
  const v = state.server, c = v.config;
  const el = document.getElementById('srv-tab');
  if (c.loader === 'vanilla') {
    el.innerHTML = `<div class="empty"><h2>Este servidor es Vanilla</h2><div class="muted">Los mods necesitan Fabric, NeoForge o Forge. Creá un servidor con loader (podés importar este mismo mundo) para instalar mods.</div></div>`;
    return;
  }
  el.innerHTML = `<div class="stack" style="gap:20px"><div class="card pad"><span class="muted">Analizando la carpeta de mods…</span></div></div>`;
  try { state.mods.list = await invoke('mods_list', { id: c.id, online: true }); state.mods.offline = false; }
  catch (e) { state.mods.list = await invoke('mods_list', { id: c.id, online: false }).catch(() => []); state.mods.offline = true; }
  const perf = await invoke('mods_performance_list');
  paintMods(perf);
}
function paintMods(perf) {
  const c = state.server.config, m = state.mods;
  const installedIds = new Set(m.list.map((x) => x.projectId).filter(Boolean));
  const issues = m.list.filter((x) => x.issues.length).length;
  const el = document.getElementById('srv-tab');
  if (!el) return;
  el.innerHTML = `<div class="stack" style="gap:20px">
    ${m.offline ? `<div class="banner"><span class="mark"></span><div class="body"><strong>Sin conexión con Modrinth</strong><span class="small">Se muestran los archivos locales. Para buscar, instalar o identificar mods hace falta internet.</span></div><button class="btn sm" data-action="tab" data-v="mods">${icon('refresh')}Reintentar</button></div>` : ''}
    <div class="stack"><div class="search-box">${icon('search')}<input id="mod-q" placeholder="Buscar mods en Modrinth (ej.: voice chat, zombies, lithium)" value="${esc(m.query)}" aria-label="Buscar mods" ${m.offline ? 'disabled' : ''}></div>
      <div class="pills"><span class="pill fixed">${esc(c.mcVersion)}</span><span class="pill fixed">Fabric</span><span class="pill">Compatibles con servidor</span><span class="small muted" style="align-self:center">La versión y el loader los fija el servidor: no se puede instalar algo incompatible por error.</span></div></div>
    <div id="mods-progress"></div>
    ${m.results ? `<div class="card"><div class="spread" style="padding:14px 16px;border-bottom:1px solid var(--light)"><h2>Resultados</h2><span class="small muted">${m.results.totalHits} encontrados</span></div>
      ${m.results.hits.length ? m.results.hits.map((h) => `<div class="mod-row"><div class="mod-icon">${h.iconUrl ? `<img src="${esc(h.iconUrl)}" alt="" loading="lazy">` : esc(h.title[0] || '?')}</div>
        <div class="mod-main"><span class="t">${esc(h.title)} <span class="badge">${SIDE[sideOf(h.clientSide, h.serverSide)]}</span></span><span class="d">${esc(h.description)}</span><span class="small muted">por ${esc(h.author)} · ${fmtNum(h.downloads)} descargas</span></div>
        ${installedIds.has(h.projectId) ? '<button class="btn sm" disabled>Instalado</button>' : `<button class="btn sm" data-action="mod-install" data-project="${esc(h.projectId)}" data-title="${esc(h.title)}">${icon('download')}Instalar</button>`}</div>`).join('') : '<div class="mod-row muted">Sin resultados para esa búsqueda.</div>'}</div>` : ''}
    <div class="grid-2">
      <div class="card"><div class="spread" style="padding:14px 16px;border-bottom:1px solid var(--light)"><h2>Instalados · ${m.list.length}</h2>
        <div class="btn-row">${issues ? `<span class="badge issue">${issues} con avisos</span>` : ''}<button class="btn sm" data-action="mod-updates" ${m.offline ? 'disabled' : ''}>${icon('refresh')}Buscar actualizaciones</button><button class="btn sm" data-action="mod-import">Importar de una carpeta…</button><button class="btn sm ghost" data-action="folder" data-which="mods">${icon('folder')}</button></div></div>
        ${m.list.length ? m.list.map((x) => `<div class="mod-row"><div class="mod-main"><span class="t">${esc(x.title)} ${x.versionNumber ? `<span class="mono small muted">${esc(x.versionNumber)}</span>` : ''}<span class="badge outline">${SIDE[x.side] || x.side}</span>${x.issues.map((i) => `<span class="badge issue">${ISSUE[i] || i}</span>`).join('')}</span>
          <span class="d">${x.managed ? (x.explicit ? 'Instalado por vos' : 'Dependencia') : 'Agregado a mano'}${x.requiredBy.length ? ` · lo necesitan: ${esc(x.requiredBy.join(', '))}` : ''} · <span class="mono">${esc(x.fileName)}</span></span></div>
          <div class="btn-row">${m.updates[x.projectId] ? `<button class="btn sm" data-action="mod-update" data-project="${esc(x.projectId)}" data-title="${esc(x.title)}">${icon('download')}${esc(m.updates[x.projectId])}</button>` : ''}
          ${x.managed ? `<button class="btn sm ghost" data-action="mod-remove" data-project="${esc(x.projectId)}" data-title="${esc(x.title)}" ${x.requiredBy.length ? 'disabled title="Otro mod lo necesita"' : ''}>${icon('trash')}Quitar</button>` : `<button class="btn sm ghost" data-action="mod-aside" data-file="${esc(x.fileName)}">${icon('trash')}Apartar</button>`}</div></div>`).join('') : '<div class="mod-row muted">Todavía no hay mods. Buscá arriba o instalá el set de optimización.</div>'}
      </div>
      <div class="card pad stack"><div class="row">${icon('bolt')}<h2>Optimización del servidor</h2></div>
        <div class="small muted">Mods de rendimiento que solo corren en el servidor. Se instala la versión exacta para ${esc(c.mcVersion)}.</div>
        ${perf.map((p) => `<div class="kv"><span><strong>${esc(p.title)}</strong>${p.recommended ? ' <span class="badge">Recomendado</span>' : ''}<br><span class="small muted">${esc(p.description)}</span></span>
          ${m.list.some((x) => (x.title || '').toLowerCase().startsWith(p.title.toLowerCase())) ? '<span class="small muted">Instalado</span>' : `<button class="btn sm" data-action="mod-install" data-project="${esc(p.slug)}" data-title="${esc(p.title)}" ${m.offline ? 'disabled' : ''}>Instalar</button>`}</div>`).join('')}
      </div>
    </div></div>`;
  const q = document.getElementById('mod-q');
  q?.addEventListener('keydown', async (e) => { if (e.key === 'Enter') searchMods(q.value); });
  state.mods.perf = perf;
}
const sideOf = (cl, sv) => (sv === 'unsupported' ? 'client' : cl === 'unsupported' ? 'server' : 'both');
const fmtNum = (n) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)} M` : n >= 1e3 ? `${Math.round(n / 1e3)} k` : String(n));
async function searchMods(query) {
  state.mods.query = query;
  try { state.mods.results = await invoke('mods_search', { id: state.params.id, query, offset: 0 }); } catch (e) { fail(e); return; }
  paintMods(state.mods.perf);
}
async function installMod(projectId, title, versionId = null) {
  let plan;
  try { plan = await invoke('mods_plan', { id: state.params.id, projectId, versionId }); } catch (e) { fail(e); return; }
  const rows = plan.items.map((i) => `<div class="kv"><span><strong>${esc(i.title)}</strong> <span class="mono small">${esc(i.version.versionNumber)}</span><br><span class="small muted">${esc(i.reason)}${i.replaces ? ` · reemplaza ${esc(i.replaces)}` : ''}</span></span><span class="badge outline">${SIDE[i.side] || i.side}</span></div>`).join('');
  const warn = plan.warnings.length ? `<ul class="list-plain small" style="margin-top:8px">${plan.warnings.map((w) => `<li>${esc(w)}</li>`).join('')}</ul>` : '';
  const ok = await dialog({ title: plan.items.length > 1 ? `Instalar ${title} y ${plan.items.length - 1} dependencia(s)` : `Instalar ${title}`, body: `${rows}${warn}<div class="small muted" style="margin-top:8px">Descarga: ${fmtMB(plan.totalBytes / 1048576)} · cada archivo se verifica por hash.</div>`, confirm: 'Instalar' });
  if (!ok) return;
  document.getElementById('mods-progress').innerHTML = `<div class="card pad stack"><strong id="mp-msg">Preparando…</strong><div class="progress"><div id="mp-bar" style="width:0%"></div></div></div>`;
  try { await invoke('mods_install', { id: state.params.id, projectId, versionId }); toast(isActive(state.server) ? 'Mods instalados. Reiniciá el servidor para cargarlos.' : 'Mods instalados'); } catch (e) { fail(e); }
  state.mods.results = state.mods.results; drawMods();
}

/* ───────── Red ───────── */
async function renderNetwork() {
  const remotes = await invoke('remote_list');
  main.innerHTML = `<div class="page">
    <div class="page-head"><div><h1>Red</h1><div class="sub">Servidores de otras PCs del grupo. Esta PC: <span class="mono">${esc(state.info.node.shortId)}</span></div></div></div>
    <div class="card pad stack"><h2>Unirse con una invitación</h2>
      <div class="small muted">Pegá el código que te mandaron (empieza con <span class="mono">939-</span>). Cada código sirve para una sola PC.</div>
      <div class="row"><input class="input mono" id="join-code" placeholder="939-…" autocomplete="off" spellcheck="false"><button class="btn primary" data-action="join">${icon('plug')}Unirme</button></div></div>
    ${remotes.length ? `<div class="server-cards">${remotes.map((r) => `<div class="card server-card" style="cursor:default">
        <div class="spread"><strong>${esc(r.name || 'Servidor')}</strong><span class="status ${r.hostOnline ? 's-running' : 's-stopped'}"><span class="shape"></span>${r.iAmHost ? 'En esta PC' : r.hostOnline ? 'En línea' : 'Sin anfitriona'}</span></div>
        <span class="mono small muted">${esc(loaderName(r.loader))} ${esc(r.mcVersion)}${r.hasCopy ? ' · coanfitriona' : ' · jugador'}</span>
        <div class="code-box"><span>127.0.0.1:${r.routePort}</span><button class="btn sm" data-action="copy" data-text="127.0.0.1:${r.routePort}">${icon('copy')}</button></div>
        <div class="small muted">En Minecraft: Multijugador → Agregar servidor → esta dirección. Siempre lleva a la PC que lo esté hosteando${r.hostName ? ` (ahora: ${esc(r.hostName)})` : ''}.</div>
        <div class="btn-row">${r.hasCopy ? `<button class="btn" data-action="open-server" data-id="${esc(r.id)}">Abrir</button>` : ''}<button class="btn ghost" data-action="remote-remove" data-id="${esc(r.id)}">${icon('trash')}Salir del grupo</button></div>
      </div>`).join('')}</div>`
      : `<div class="empty"><h2>Todavía no te uniste a ningún servidor</h2><div class="muted">Pedile a un amigo que te invite desde su servidor en 939 Host.</div></div>`}
  </div>`;
}

/* ───────── Este equipo ───────── */
async function renderNode() {
  const hw = state.hw = await invoke('hardware_snapshot');
  const s = state.info.settings;
  const a = hw.assessment;
  main.innerHTML = `<div class="page">
    <div class="page-head"><div><h1>Este equipo</h1><div class="sub">${esc(s.nodeName)} · nodo <span class="mono">${esc(state.info.node.shortId)}</span></div></div>
      <button class="btn" data-action="bench" id="bench-btn">Ejecutar prueba rápida (~2 s)</button></div>
    <div class="kpis">
      <div class="card kpi"><span class="label">Uso de CPU</span><span class="value" id="live-cpu">—</span></div>
      <div class="card kpi"><span class="label">RAM libre</span><span class="value" id="live-ram">—</span></div>
      <div class="card kpi"><span class="label">CPU por núcleo</span><span class="value">${hw.benchmark ? hw.benchmark.cpuSingleScore : '—'} <small>${hw.benchmark ? 'puntos' : 'sin medir'}</small></span></div>
      <div class="card kpi"><span class="label">Escritura en disco</span><span class="value">${hw.benchmark?.diskWriteMbS ? hw.benchmark.diskWriteMbS : '—'} <small>MB/s</small></span></div>
    </div>
    <div class="banner ${a.rating.startsWith('Buena') ? 'info' : ''}" id="live-banner"><span class="mark"></span><div class="body"><strong>${esc(a.rating)}</strong><ul class="list-plain small">${a.reasons.map((r) => `<li>${esc(r)}</li>`).join('')}</ul><span class="small muted" id="live-reasons"></span></div></div>
    <div class="grid-2">
      <div class="card pad stack"><h2>Hardware</h2>
        <div class="kv"><span class="k">Procesador</span><span>${esc(hw.cpuModel)}</span></div>
        <div class="kv"><span class="k">Núcleos</span><span class="mono">${hw.physicalCores ?? '—'} físicos · ${hw.logicalCores} hilos</span></div>
        <div class="kv"><span class="k">RAM total</span><span class="mono">${fmtMB(hw.ramTotalMb)}</span></div>
        <div class="kv"><span class="k">Sistema</span><span>${esc(hw.os)}</span></div>
        <div class="kv"><span class="k">RAM recomendada por servidor</span><span class="mono">${fmtMB(a.recommendedServerRamMb)} (máx. ${fmtMB(a.maxServerRamMb)})</span></div>
        <h3 style="margin-top:8px">Discos</h3>
        ${hw.disks.map((d) => `<div class="kv"><span class="k">${esc(d.mount)} ${d.isDataDisk ? '<span class="badge">datos de 939 Host</span>' : ''}</span><span class="mono">${esc(d.kind)} · ${d.freeGb.toFixed(0)} de ${d.totalGb.toFixed(0)} GB libres</span></div>`).join('')}
        <div class="small muted">Las métricas en vivo se actualizan cada 5 s solo mientras esta pantalla está abierta. La prueba rápida se corre una vez o cuando la pidas; nunca en segundo plano.</div>
      </div>
      <div class="card pad stack"><h2>Participación en el grupo</h2>
        <label class="check"><input type="checkbox" data-s="allowHosting" ${s.allowHosting ? 'checked' : ''}><span>Esta PC puede ser anfitriona<span class="hint">Si lo desactivás, nunca va a recibir un servidor por traspaso ni por recuperación automática (igual podés jugar).</span></span></label>
        <label class="check"><input type="checkbox" data-s="allowRelay" ${s.allowRelay ? 'checked' : ''}><span>Puede hacer de puente para otras PCs del grupo<span class="hint">Si un amigo no llega directo a la anfitriona pero sí a esta PC, el tráfico pasa por acá.</span></span></label>
        <h3 style="margin-top:6px">Orden de conexión</h3>
        <ol class="list-plain small"><li>Directa: misma red, IPv6 o puertos abiertos solos por UPnP/NAT-PMP</li><li>Hole punching entre routers (el relay público solo coordina el primer contacto; los datos van directo si se logra)</li><li>Otra PC del grupo como puente</li><li>Relay público gratuito para los datos, solo si nada de lo anterior funciona</li></ol>
        <label class="check"><input type="checkbox" data-s="allowPublicRelays" ${s.allowPublicRelays ? 'checked' : ''}><span>Permitir relays públicos gratuitos (pasos 2 y 4)<span class="hint">Sin esto solo hay conexión directa o por otra PC del grupo. Se aplica al reiniciar 939 Host.</span></span></label>
      </div></div></div>`;
  const tick = async () => {
    if (state.route !== 'node') return;
    try {
      const l = await invoke('hardware_live', { needMb: a.recommendedServerRamMb });
      document.getElementById('live-cpu').innerHTML = `${l.cpuUsage.toFixed(0)}<small>%</small>`;
      document.getElementById('live-ram').innerHTML = `${fmtMB(l.ramAvailableMb)} <small>de ${fmtMB(l.ramTotalMb)}</small>`;
      document.getElementById('live-reasons').textContent = l.hostReady ? 'Ahora mismo: disponible para hostear.' : `Ahora mismo: ${l.reasons.join(' ')}`;
    } catch { /* ignorar */ }
  };
  setTimeout(tick, 400);
  every(5000, tick);
}

/* ───────── Configuración ───────── */
async function renderSettings() {
  const s = state.info.settings;
  const runtimes = await invoke('list_runtimes');
  let profiles = [];
  try { profiles = await invoke('client_profiles'); } catch (e) { profiles = []; }
  state.clientProfiles = profiles;
  main.innerHTML = `<div class="page">
    <div class="page-head"><div><h1>Configuración</h1><div class="sub">939 Host ${esc(state.info.version)}</div></div></div>
    <section class="section"><h2>Aplicación</h2>
      <label class="field" style="max-width:420px">Nombre de este equipo<input class="input" data-s="nodeName" maxlength="40" value="${esc(s.nodeName)}"></label>
      <label class="check"><input type="checkbox" data-s="startWithWindows" ${s.startWithWindows ? 'checked' : ''}><span>Iniciar con Windows<span class="hint">Arranca minimizado en la bandeja, sin abrir la ventana. Los servidores compartidos vuelven a estar disponibles para tus amigos.</span></span></label>
      <label class="check"><input type="checkbox" data-s="closeToTray" ${s.closeToTray ? 'checked' : ''}><span>Al cerrar la ventana, seguir en la bandeja del sistema<span class="hint">Los servidores siguen andando y la interfaz se cierra por completo para liberar memoria. Para cerrar todo, usá Salir.</span></span></label>
    </section>
    <section class="section"><h2>Reconexión automática al jugar</h2>
      <div class="small muted">Cuando el servidor cambia de PC, los jugadores se desconectan unos segundos. Con esto activado, tu Minecraft vuelve a entrar solo. 939 Host agrega el mod AutoReconnect (Reforged) y lo que necesita al perfil que elijas: solo agrega archivos, nunca borra ni reemplaza los tuyos, y lo vuelve a poner si se borra por error.</div>
      ${profiles.length ? profiles.map((p, i) => `<div class="kv"><span>${esc(p.name)} <span class="small muted mono">${esc(loaderName(p.loader))} ${esc(p.mcVersion)}</span></span>
        ${p.enabled ? `<span class="row" style="gap:8px"><span class="badge outline">Activada</span><button class="btn sm ghost" data-action="reconnect-off" data-i="${i}">Desactivar</button></span>` : `<button class="btn sm" data-action="reconnect-on" data-i="${i}">Activar</button>`}</div>`).join('')
        : '<div class="small muted">No se encontraron perfiles Fabric o NeoForge en el launcher oficial de Minecraft de esta PC.</div>'}
    </section>
    <section class="section"><h2>Datos</h2>
      <div class="kv"><span class="k">Carpeta de datos</span><span class="mono small">${esc(state.info.dataDir)}</span></div>
      <div class="btn-row"><button class="btn" data-action="folder-global" data-which="data">Abrir carpeta de datos</button><button class="btn" data-action="folder-global" data-which="trash">Abrir Papelera</button></div>
      <div class="small muted">939 Host no modifica tu instalación de Minecraft: los mundos importados se copian. La única excepción es la reconexión automática, y solo en el perfil que vos actives.</div>
    </section>
    <section class="section"><h2>Java</h2>
      ${runtimes.length ? runtimes.map((r) => `<div class="kv"><span class="k">Java ${r.major}</span><span class="mono small">${esc(r.version)} · ${fmtMB(r.sizeMb)}</span></div>`).join('') : '<div class="small muted">Todavía no se descargó ningún Java. Se instala solo al crear el primer servidor.</div>'}
    </section>
    <section class="section"><h2>Salir</h2><div class="spread"><span class="small muted">Detiene todos los servidores de forma ordenada, libera la RAM y la CPU, y deja de participar en el grupo.</span><button class="btn primary" data-action="quit">Salir de 939 Host</button></div></section>
  </div>`;
}

/* ───────── acciones (delegación de eventos) ───────── */
const actions = {
  route: (el) => go(el.dataset.route),
  'reconnect-on': async (el) => {
    const p = state.clientProfiles[+el.dataset.i]; if (!p) return;
    el.disabled = true; toast('Preparando la reconexión automática…');
    try { const { name, gameDir, mcVersion, loader } = p; const r = await invoke('client_reconnect_enable', { profile: { name, gameDir, mcVersion, loader } }); toast(r.message); } catch (e) { fail(e); }
    renderSettings();
  },
  'reconnect-off': async (el) => {
    const p = state.clientProfiles[+el.dataset.i]; if (!p) return;
    if (!(await dialog({ title: 'Desactivar la reconexión automática', body: 'Se quitan solo los archivos que agregó 939 Host (van a la Papelera de 939 Host). Lo demás de tu perfil no se toca.', confirm: 'Desactivar' }))) return;
    try { const { name, gameDir, mcVersion, loader } = p; await invoke('client_reconnect_disable', { profile: { name, gameDir, mcVersion, loader } }); toast('Reconexión automática desactivada'); } catch (e) { fail(e); }
    renderSettings();
  },
  new: () => { state.wizard = null; go('create'); },
  open: (el) => { state.tab = 'summary'; state.edit = null; go('server', { id: el.dataset.id }); },
  tab: async (el) => {
    if (state.tab === 'settings' && el.dataset.v !== 'settings' && isDirty() && !(await dialog({ title: 'Hay cambios sin guardar', body: 'Si cambiás de pestaña se descartan.', confirm: 'Descartar cambios' }))) return;
    if (el.dataset.v !== 'settings') state.edit = null;
    state.tab = el.dataset.v;
    document.querySelectorAll('.tab').forEach((t) => t.classList.toggle('active', t.dataset.v === state.tab));
    drawTab();
  },
  start: async (el) => { el.disabled = true; try { await invoke('start_server', { id: el.dataset.id }); toast('Iniciando servidor…'); } catch (e) { fail(e); el.disabled = false; } },
  stop: async (el) => { el.disabled = true; try { await invoke('stop_server', { id: el.dataset.id }); toast('Guardando y deteniendo…'); } catch (e) { fail(e); el.disabled = false; } },
  restart: async (el) => { el.disabled = true; toast('Reiniciando…'); try { await invoke('restart_server', { id: el.dataset.id }); } catch (e) { fail(e); } },
  reinstall: async () => {
    if (!(await dialog({ title: 'Reinstalar archivos del servidor', body: 'Se vuelven a descargar y verificar el servidor y Java. El mundo, la configuración y los backups no se tocan.', confirm: 'Reinstalar' }))) return;
    try { await invoke('reinstall_server', { id: state.params.id }); toast('Reinstalando…'); refreshServer(); } catch (e) { fail(e); }
  },
  folder: (el) => call('open_folder', { id: state.params.id, which: el.dataset.which }).catch(() => {}),
  'folder-global': (el) => call('open_folder', { id: null, which: el.dataset.which }).catch(() => {}),
  copy: async (el) => { try { await navigator.clipboard.writeText(el.dataset.text); toast('Código copiado'); } catch { toast('No se pudo copiar', 'error'); } },
  eula: () => call('open_link', { url: 'https://aka.ms/MinecraftEULA' }).catch(() => {}),
  disclose: (el) => { const k = el.dataset.v; state.open.has(k) ? state.open.delete(k) : state.open.add(k); const y = main.scrollTop; drawSettings().then(() => { main.scrollTop = y; }); },
  toc: (el) => document.getElementById(`sec-${el.dataset.v}`)?.scrollIntoView({ behavior: 'smooth' }),
  discard: () => { state.edit = clone(state.server.config); state.extraProps = state.extraPropsOrig; drawSettings(); },
  save: () => saveSettings(),
  pl: async (el) => {
    const a = el.dataset.a;
    const name = el.dataset.name || (document.getElementById('pl-name')?.value || '').trim();
    if (!name) { toast('Escribí el nombre del jugador.', 'error'); return; }
    let reason = null;
    if (a === 'ban' || a === 'kick') {
      const ok = await dialog({ title: a === 'ban' ? `Banear a ${name}` : `Expulsar a ${name}`, body: `<label class="field">Motivo (opcional)<input class="input" id="pl-reason" maxlength="120"></label>`, confirm: a === 'ban' ? 'Banear' : 'Expulsar' });
      if (!ok) return;
      reason = state.plReason || null; state.plReason = null;
    }
    try { const r = await invoke('player_action', { id: state.params.id, name, action: a, reason }); toast(r); } catch (e) { fail(e); }
    setTimeout(drawPlayers, 600);
  },
  'mod-import': async () => {
    const path = await TAURI.dialog.open({ directory: true, title: 'Elegí una carpeta de mods (por ejemplo, la de tu perfil de Minecraft)' });
    if (!path) return;
    try {
      const r = await invoke('mods_import_folder', { id: state.params.id, path });
      await dialog({ title: 'Mods importados', body: `<div class="small">Copiados: <strong>${r.copied.length}</strong>. La carpeta original no se modificó.</div>${r.skippedClient.length ? `<div class="small" style="margin-top:8px">Omitidos por ser solo de cliente: ${esc(r.skippedClient.join(', '))}</div>` : ''}${r.skippedDuplicate.length ? `<div class="small" style="margin-top:8px">Duplicados omitidos (se usó el más nuevo): ${esc(r.skippedDuplicate.join(', '))}</div>` : ''}${r.skippedExisting.length ? `<div class="small" style="margin-top:8px">Ya estaban: ${r.skippedExisting.length}</div>` : ''}`, confirm: 'Listo', cancel: null });
    } catch (e) { fail(e); }
    drawMods();
  },
  'world-new': async () => {
    const name = state.server.config.name;
    if (isActive(state.server)) { toast('Detené el servidor primero.', 'error'); return; }
    const ok = await dialog({ title: 'Crear un mundo nuevo', body: `El mundo actual se guarda como punto de restauración y se mueve a la Papelera. <label class="field" style="margin-top:10px">Semilla (vacío = la actual)<input class="input mono" id="wn-seed" maxlength="64"></label>`, confirm: 'Crear mundo nuevo', requireText: name });
    if (!ok) return;
    try { await invoke('world_new', { id: state.params.id, seed: state.wnSeed || null, confirmName: name }); toast('Listo: el próximo inicio genera un mundo nuevo'); } catch (e) { fail(e); }
    state.wnSeed = null;
  },
  diag: async (el) => {
    el.disabled = true;
    try { await invoke('diagnostics_report', { id: state.params.id }); toast('Informe guardado en la carpeta de registros'); await invoke('open_folder', { id: null, which: 'applogs' }); } catch (e) { fail(e); }
    el.disabled = false;
  },
  delete: async () => {
    const name = state.server.config.name;
    if (isActive(state.server)) { toast('Detené el servidor antes de borrarlo.', 'error'); return; }
    if (!(await dialog({ title: `Borrar «${name}»`, body: 'El servidor, su mundo y sus backups se mueven a la Papelera de 939 Host. Podés recuperarlos desde esa carpeta.', confirm: 'Borrar', requireText: name }))) return;
    try { await invoke('delete_server', { id: state.params.id, confirmName: name }); toast('Servidor movido a la Papelera'); go('servers'); } catch (e) { fail(e); }
  },
  backup: async (el) => {
    el.disabled = true; el.textContent = 'Creando…';
    try { await invoke('create_backup', { id: state.params.id, label: 'Copia completa' }); toast('Copia .zip creada'); } catch (e) { fail(e); }
    drawBackups();
  },
  'snap-take': async (el) => {
    el.disabled = true;
    try { const i = await invoke('snapshot_take', { id: state.params.id, label: document.getElementById('snap-label').value }); toast(`Punto creado: ${fmtMB(i.newBytes / 1048576)} nuevos`); } catch (e) { fail(e); }
    drawBackups();
  },
  'snap-restore': async (el) => {
    if (isActive(state.server)) { toast('Detené el servidor antes de restaurar.', 'error'); return; }
    if (!(await dialog({ title: 'Restaurar este punto', body: 'Antes de restaurar, 939 Host guarda el estado actual como punto de «Seguridad» y aparta el mundo actual en la Papelera. Se puede deshacer restaurando ese punto.', confirm: 'Restaurar' }))) return;
    try { await invoke('snapshot_restore', { id: state.params.id, snapId: el.dataset.snap }); toast('Punto restaurado'); } catch (e) { fail(e); }
    drawBackups();
  },
  'snap-delete': async (el) => {
    if (!(await dialog({ title: 'Borrar punto de restauración', body: 'Se borra este punto. Los datos que comparte con otros puntos se conservan.', confirm: 'Borrar' }))) return;
    try { await invoke('snapshot_delete', { id: state.params.id, snapId: el.dataset.snap }); } catch (e) { fail(e); }
    drawBackups();
  },
  restore: async (el) => {
    if (isActive(state.server)) { toast('Detené el servidor antes de restaurar.', 'error'); return; }
    if (!(await dialog({ title: 'Restaurar este backup', body: 'Antes de restaurar, 939 Host guarda el estado actual como backup «Antes de restaurar» y aparta el mundo actual en la Papelera. Se puede deshacer.', confirm: 'Restaurar' }))) return;
    try { await invoke('restore_backup', { id: state.params.id, file: el.dataset.file }); toast('Backup restaurado'); } catch (e) { fail(e); }
    drawBackups();
  },
  'del-backup': async (el) => {
    if (!(await dialog({ title: 'Borrar backup', body: 'Este archivo se elimina del disco y no se puede recuperar.', confirm: 'Borrar' }))) return;
    try { await invoke('delete_backup', { id: state.params.id, file: el.dataset.file }); } catch (e) { fail(e); }
    drawBackups();
  },
  'import-world': async () => {
    if (isActive(state.server)) { toast('Detené el servidor antes de importar un mundo.', 'error'); return; }
    const path = await TAURI.dialog.open({ directory: true, title: 'Elegí la carpeta del mundo (la que tiene level.dat)' });
    if (!path) return;
    try { await invoke('validate_world', { path }); } catch (e) { fail(e); return; }
    if (!(await dialog({ title: 'Reemplazar el mundo', body: `Se copia <span class="mono">${esc(path)}</span> al servidor. El original no se modifica. El mundo actual del servidor se mueve a la Papelera de 939 Host.`, confirm: 'Copiar mundo' }))) return;
    try { const mb = await invoke('import_world', { id: state.params.id, path }); toast(`Mundo copiado (${fmtMB(mb)})`); } catch (e) { fail(e); }
  },
  bench: async (el) => {
    el.disabled = true; el.textContent = 'Midiendo…';
    try { await invoke('run_benchmark'); toast('Prueba completada'); } catch (e) { fail(e); }
    state.hw = null; renderNode();
  },
  quit: () => confirmQuit(),
  invite: async (el) => {
    const role = el.dataset.role || 'jugador';
    try {
      const ticket = await invoke('invite_create', { id: state.params.id, role, label: role });
      await dialog({ title: role === 'coanfitrion' ? 'Invitación de coanfitriona' : 'Invitación de jugador', body: `<div class="small" style="margin-bottom:10px">${role === 'coanfitrion' ? 'La PC que la use guarda una copia sincronizada y puede hostear el servidor cuando esta no esté.' : 'La PC que la use puede jugar, pero no guarda copia ni hostea.'} Se pega en 939 Host → Red → Unirme. Sirve para una sola PC; esta PC tiene que estar con 939 Host abierto cuando la usen.</div><div class="code-box" style="word-break:break-all;align-items:flex-start"><span class="small">${esc(ticket)}</span></div>`, confirm: 'Copiar y cerrar', cancel: null });
      try { await navigator.clipboard.writeText(ticket); toast('Código copiado'); } catch { /* sin portapapeles */ }
    } catch (e) { fail(e); }
    drawShare();
  },
  'invite-revoke': async (el) => { try { await invoke('invite_revoke', { id: state.params.id, secret: el.dataset.secret }); toast('Invitación revocada'); } catch (e) { fail(e); } drawShare(); },
  'member-remove': async (el) => {
    if (!(await dialog({ title: 'Quitar miembro', body: 'Pierde el acceso a este servidor. Para volver necesita una invitación nueva.', confirm: 'Quitar' }))) return;
    try { await invoke('member_remove', { id: state.params.id, nodeId: el.dataset.node }); } catch (e) { fail(e); } drawShare();
  },
  join: async (el) => {
    const code = document.getElementById('join-code').value.trim();
    if (!code) return;
    el.disabled = true; el.textContent = 'Uniéndome…';
    try { const r = await invoke('remote_join', { ticket: code }); toast(`Te uniste a ${r.name}`); } catch (e) { fail(e); }
    renderNetwork();
  },
  'remote-remove': async (el) => {
    if (!(await dialog({ title: 'Salir del grupo', body: 'Esta PC deja de participar en este servidor. Si eras coanfitriona, tu copia queda en Servidores y la podés borrar aparte. Para volver vas a necesitar otra invitación.', confirm: 'Salir del grupo' }))) return;
    try { await invoke('remote_remove', { id: el.dataset.id }); } catch (e) { fail(e); } renderNetwork();
  },
  'open-server': (el) => { state.tab = 'summary'; state.edit = null; go('server', { id: el.dataset.id }); },
  handover: async () => {
    const g = state.share?.group; if (!g) return;
    const eligible = g.members.filter((m) => !m.isMe && m.online && m.canHost);
    let target = eligible[0]?.nodeId || null;
    const ok = await dialog({ title: 'Pasar el servidor a otra PC', body: `<div class="small" style="margin-bottom:10px">Se guarda y apaga acá, la otra PC copia lo que le falta y lo enciende. Los jugadores se desconectan unos segundos y vuelven a entrar con la misma dirección.</div>
      <label class="field">PC de destino<select class="input" id="ho-target">${eligible.map((m) => `<option value="${esc(m.nodeId)}">${esc(m.name)} · estado ${esc(ago(m.latestAt))}${m.path ? ` · ${m.path === 'relay' ? 'por relay' : 'directa'}` : ''}</option>`).join('')}</select></label>`, confirm: 'Pasar ahora' });
    if (!ok) return;
    target = state.hoTarget || target;
    toast('Pasando el servidor…');
    try { const to = await invoke('group_handover', { id: state.params.id, target }); toast(`Servidor pasado a ${to}`); } catch (e) { fail(e); }
    state.hoTarget = null; refreshServer(); drawShare();
  },
  release: async () => {
    if (!(await dialog({ title: 'Guardar y soltar', body: 'Se apaga acá, se toma un punto final y se espera a que otra PC del grupo lo copie. Después cualquiera puede encenderlo con ese estado.', confirm: 'Guardar y soltar' }))) return;
    try { const n = await invoke('group_release', { id: state.params.id }); toast(n > 0 ? `Soltado: ${n} PC tiene la última versión` : 'Soltado. Ninguna otra PC estaba conectada: la última versión quedó solo en esta PC', n > 0 ? 'ok' : 'error'); } catch (e) { fail(e); }
    refreshServer(); drawShare();
  },
  'take-here': async () => {
    if (!(await dialog({ title: 'Recuperar el servidor en esta PC', body: 'Usalo solo si sabés que la PC anfitriona se apagó o se quedó sin internet. Se enciende acá con el último estado que copió esta PC: lo que se jugó después en la otra PC no está incluido. Si la otra PC vuelve, se detiene sola y aparta su copia (no se mezclan).', confirm: 'Recuperar aquí' }))) return;
    toast('Recuperando…');
    try { await invoke('group_take_here', { id: state.params.id }); toast('El servidor está en línea en esta PC'); } catch (e) { fail(e); }
    refreshServer(); drawShare();
  },
  'group-sync': async () => { try { await invoke('group_sync', { id: state.params.id }); toast('Sincronizado'); } catch (e) { fail(e); } drawShare(); },
  'open-mods': (el) => { state.tab = 'mods'; state.edit = null; go('server', { id: el.dataset.id }); },
  'mod-install': (el) => installMod(el.dataset.project, el.dataset.title),
  'mod-update': (el) => installMod(el.dataset.project, el.dataset.title),
  'mod-remove': async (el) => {
    if (!(await dialog({ title: `Quitar ${el.dataset.title}`, body: 'Se borra el archivo del mod. Las dependencias que solo usaba este mod también se quitan.', confirm: 'Quitar' }))) return;
    try { const r = await invoke('mods_remove', { id: state.params.id, projectId: el.dataset.project }); toast(`Quitado: ${r.join(', ')}`); } catch (e) { fail(e); }
    drawMods();
  },
  'mod-aside': async (el) => {
    if (!(await dialog({ title: 'Apartar archivo', body: `<span class="mono">${esc(el.dataset.file)}</span> se mueve a la Papelera de 939 Host. Podés recuperarlo desde ahí.`, confirm: 'Apartar' }))) return;
    try { await invoke('mods_set_aside', { id: state.params.id, fileName: el.dataset.file }); toast('Archivo apartado'); } catch (e) { fail(e); }
    drawMods();
  },
  'mod-updates': async (el) => {
    el.disabled = true;
    try { state.mods.updates = await invoke('mods_updates', { id: state.params.id }); const n = Object.keys(state.mods.updates).length; toast(n ? `${n} actualización(es) disponibles` : 'Todo está actualizado'); } catch (e) { fail(e); }
    paintMods(state.mods.perf);
  },
  /* asistente */
  loader: (el) => { state.wizard.loader = el.dataset.v; checkLoader(); },
  worldmode: (el) => { state.wizard.worldMode = el.dataset.v; drawCreate(); },
  wnext: () => { if (stepValid()) { state.wizard.step++; drawCreate(); } },
  wback: () => { state.wizard.step--; drawCreate(); },
  'pick-world': async () => {
    const path = await TAURI.dialog.open({ directory: true, title: 'Elegí la carpeta del mundo (la que tiene level.dat)' });
    if (!path) return;
    try { await invoke('validate_world', { path }); state.wizard.importPath = path; state.wizard.importOk = true; } catch (e) { state.wizard.importOk = false; state.wizard.importPath = ''; fail(e); }
    drawCreate();
  },
  wcreate: async () => {
    const w = state.wizard;
    w.creating = true; drawCreate();
    try {
      const cfg = await invoke('create_server', { input: {
        name: w.name.trim(), description: w.description.trim(), mcVersion: w.mcVersion, loader: w.loader, memoryMb: w.memoryMb,
        difficulty: w.difficulty, gamemode: w.gamemode, maxPlayers: w.maxPlayers, port: w.port, onlineMode: w.onlineMode,
        whitelist: w.whitelist, viewDistance: w.viewDistance, simulationDistance: w.simulationDistance, levelSeed: w.worldMode === 'new' ? w.seed : '',
        acceptEula: w.eula, importWorldPath: w.worldMode === 'import' ? w.importPath : null,
      } });
      state.wizard = null; state.tab = 'summary';
      toast('Servidor creado. Descargando lo necesario…');
      go('server', { id: cfg.id });
    } catch (e) { w.creating = false; fail(e); drawCreate(); }
  },
};

main.addEventListener('click', (e) => {
  const el = e.target.closest('[data-action]');
  if (!el || el.disabled) return;
  const fn = actions[el.dataset.action];
  if (fn) { e.preventDefault(); fn(el); }
});

/* Campos del asistente (data-w), ajustes del servidor (data-bind) y de la app (data-s). */
main.addEventListener('input', (e) => {
  const el = e.target;
  const val = el.type === 'checkbox' ? el.checked : el.dataset.type === 'int' ? parseInt(el.value, 10) || 0 : el.value;
  if (el.dataset.w && state.wizard) {
    const key = el.dataset.w;
    state.wizard[key] = val;
    if (key === 'snapshots') { toggleSnapshots(val); return; }
    if (key === 'mcVersion') { checkLoader(); return; }
    if (el.type === 'range' || el.type === 'checkbox' || el.tagName === 'SELECT') { const y = main.scrollTop; drawCreate(); main.scrollTop = y; return; }
    // Texto: actualizar solo el botón y el resumen sin perder el foco.
    const btn = main.querySelector('[data-action="wnext"]');
    if (btn) btn.disabled = !stepValid();
    return;
  }
  if (el.dataset.bind && state.edit) {
    state.edit[el.dataset.bind] = val;
    if (el.dataset.label) document.getElementById(el.dataset.label).textContent = el.dataset.bind === 'memoryMb' ? fmtMB(val) : `${val}${el.dataset.suffix || ''}`;
    updateSavebar(); updateJvmPreview();
    return;
  }
  if (el.id === 'extra-props') { state.extraProps = el.value; updateSavebar(); return; }
});
main.addEventListener('change', async (e) => {
  const el = e.target;
  if (!el.dataset.s) return;
  const val = el.type === 'checkbox' ? el.checked : el.dataset.type === 'int' ? parseInt(el.value, 10) || 0 : el.value;
  const next = { ...state.info.settings, [el.dataset.s]: val };
  try {
    state.info.settings = await invoke('save_settings', { settings: next });
    document.getElementById('foot-node').textContent = state.info.settings.nodeName;
    toast('Ajuste guardado');
  } catch (err) { fail(err); }
});
async function toggleSnapshots(on) {
  if (on && !state.snapshots) { try { state.snapshots = await invoke('list_versions', { includeSnapshots: true }); } catch (e) { fail(e); state.wizard.snapshots = false; } }
  const list = (state.wizard.snapshots ? state.snapshots : state.versions) || [];
  if (!list.some((v) => v.id === state.wizard.mcVersion)) state.wizard.mcVersion = list[0]?.id || '';
  drawCreate();
}

async function confirmQuit() {
  const running = (await invoke('list_servers')).filter(isActive);
  if (running.length && !(await dialog({ title: 'Salir de 939 Host', body: `${running.length === 1 ? 'Hay un servidor encendido' : `Hay ${running.length} servidores encendidos`}. Los compartidos pasan a otra PC del grupo que esté conectada (si está activado); si no hay ninguna, se guardan, se copian a quien se pueda y se detienen. Los demás se guardan y se detienen.`, confirm: 'Salir' }))) return;
  invoke('quit_app');
}

/* ───────── eventos del backend ───────── */
let refreshTimer = null;
function scheduleRefresh(id) {
  clearTimeout(refreshTimer);
  refreshTimer = setTimeout(() => {
    if (state.route === 'server' && state.params.id === id) refreshServer();
    else if (state.route === 'home' || state.route === 'servers') refreshList();
  }, 250);
}

async function boot() {
  document.querySelectorAll('.nav[data-route]').forEach((n) => n.addEventListener('click', async () => {
    if (state.route === 'server' && state.tab === 'settings' && isDirty() && !(await dialog({ title: 'Hay cambios sin guardar', body: 'Si salís de esta pantalla se descartan.', confirm: 'Descartar cambios' }))) return;
    state.edit = null; go(n.dataset.route);
  }));
  document.getElementById('quit-btn').addEventListener('click', confirmQuit);
  document.addEventListener('change', (e) => { if (e.target.id === 'ho-target') state.hoTarget = e.target.value; });
  document.addEventListener('input', (e) => { if (e.target.id === 'pl-reason') state.plReason = e.target.value; if (e.target.id === 'wn-seed') state.wnSeed = e.target.value; });
  try { state.info = await invoke('app_info'); } catch (e) { fail(e); return; }
  document.getElementById('foot-node').textContent = state.info.settings.nodeName;

  await listen('server-changed', (ev) => scheduleRefresh(ev.payload.id));
  await listen('console-line', (ev) => { if (state.route === 'server' && state.tab === 'console' && state.params.id === ev.payload.id) appendConsole(ev.payload.line); });
  await listen('install-progress', (ev) => {
    const p = ev.payload; state.progress[p.id] = p;
    if (p.done) { if (p.error) toast('La instalación falló', 'error', p.error); else toast('Servidor listo para iniciar'); }
    if (state.route === 'server' && state.params.id === p.id) { if (p.done) refreshServer(); else drawServerHeader(); }
  });
  await listen('mods-progress', (ev) => {
    const msg = document.getElementById('mp-msg'), bar = document.getElementById('mp-bar');
    if (msg && state.params.id === ev.payload.id) { msg.textContent = ev.payload.message; bar.style.width = `${Math.round(ev.payload.fraction * 100)}%`; }
  });
  let shareTimer = null;
  await listen('share-changed', (ev) => {
    clearTimeout(shareTimer);
    shareTimer = setTimeout(() => { if (state.route === 'server' && state.params.id === ev.payload.id && state.tab === 'summary') drawShare(); else if (state.route === 'network') renderNetwork(); }, 300);
  });
  await listen('group-note', (ev) => {
    const m = ev.payload.message || '';
    if (/anfitriona|pasado|tomó|Recuperando|Deteniendo|se unió|no pudo|soltado/i.test(m)) toast(m, /no pudo|Deteniendo/.test(m) ? 'error' : 'ok');
  });
  await listen('app-quitting', () => {
    document.getElementById('modal-root').innerHTML = `<div class="overlay"><div class="dialog"><h2>Cerrando 939 Host</h2><div class="muted">Pasando o guardando los servidores de forma ordenada. Puede tardar hasta un minuto y medio si otra PC está copiando el último estado. No apagues la PC todavía.</div><div class="progress"><div style="width:60%"></div></div></div></div>`;
  });

  go('home');

  // Evaluación inicial del nodo, una sola vez.
  if (!state.info.settings.onboardingDone) {
    const ok = await dialog({ title: 'Bienvenido a 939 Host', body: 'Para elegir buenas configuraciones, 939 Host evalúa esta PC una vez: procesador, RAM, disco y una prueba rápida de ~2 segundos. No vuelve a correr pruebas pesadas sin que lo pidas.', confirm: 'Evaluar ahora', cancel: 'Más tarde' });
    if (ok) { try { await invoke('run_benchmark'); state.hw = null; toast('Evaluación completada'); } catch (e) { fail(e); } }
    state.info.settings = await invoke('save_settings', { settings: { ...state.info.settings, onboardingDone: true } }).catch(() => state.info.settings);
    if (state.route === 'home') renderHome();
  }
}

boot();
