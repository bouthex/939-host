'use strict';
/* Arranque: datos de la app, tema, eventos del backend y primera pantalla. */

let refreshTimer = null;
function scheduleRefresh(id) {
  clearTimeout(refreshTimer);
  refreshTimer = setTimeout(() => {
    if (state.route === 'server' && state.params.id === id) refreshServer();
    else if (state.route === 'servers') routes.servers();
    else if (state.route === 'repair' && state.params.id === id) invoke('get_server', { id }).then((v) => { state.server = v; }).catch(() => {});
  }, 250);
}

async function boot() {
  $('#quit-btn').innerHTML = icon('logout', 16);
  $('#foot-ic').innerHTML = icon('pc', 16);
  try { state.info = await invoke('app_info'); } catch (e) { fail(e); $('#splash')?.classList.add('out'); return; }
  applyTheme();
  paintFoot();
  refreshFootState();
  refreshTrashCount();
  setInterval(() => { if (!document.hidden) refreshFootState(); }, 30000);

  await listen('server-changed', (ev) => scheduleRefresh(ev.payload.id));
  await listen('console-line', (ev) => { if (state.route === 'server' && state.tab === 'console' && state.params.id === ev.payload.id) appendConsole(ev.payload.line); });
  await listen('install-progress', (ev) => {
    const p = ev.payload; state.progress[p.id] = p;
    if (p.done) { if (p.error) toast('La instalación falló', 'error', p.error); else toast('Servidor listo para iniciar'); }
    if (state.route === 'server' && state.params.id === p.id) { if (p.done) refreshServer(); else drawHeader(); }
  });
  await listen('mods-progress', (ev) => {
    const msg = $('#mp-msg'), bar = $('#mp-bar');
    if (msg && state.params.id === ev.payload.id) { msg.textContent = ev.payload.message; bar.style.width = `${Math.round(ev.payload.fraction * 100)}%`; setPring($('#mp-ring'), ev.payload.fraction); }
  });
  await listen('assets-progress', (ev) => {
    const m = $('#assets-msg'), b = $('#assets-bar');
    if (m) m.textContent = ev.payload.message;
    if (b) b.style.width = `${Math.round(ev.payload.fraction * 100)}%`;
  });
  await listen('repair-progress', (ev) => onRepairProgress(ev.payload));
  await listen('host-request', (ev) => { toast(`${ev.payload.name} quiere alojar uno de tus servidores`); refreshRequests(); });
  let shareTimer = null;
  await listen('share-changed', (ev) => {
    clearTimeout(shareTimer);
    shareTimer = setTimeout(() => {
      refreshRequests();
      if (state.route === 'server' && state.params.id === ev.payload.id && state.tab === 'group') drawTab();
      else if (state.route === 'friends') routes.friends();
    }, 300);
  });
  await listen('group-note', (ev) => {
    const m = ev.payload.message || '';
    if (/anfitri|pasado|tomó|Recuperando|Deteniendo|se unió|no pudo|soltado|mod\(s\)/i.test(m)) toast(m, /no pudo|Deteniendo/.test(m) ? 'error' : 'ok');
  });
  // Primera vez que se cierra la ventana con servidores encendidos: se explica qué pasa.
  await listen('background-hint', async (ev) => {
    const names = ev.payload?.servers || [];
    const what = names.length === 1 ? `<b>${esc(names[0])}</b> sigue encendido` : `<b>${names.length} servidores</b> siguen encendidos`;
    const r = await dialog({ title: 'Cerrar la ventana', body: `<p>${what}. Al cerrar la ventana, 939 Host sigue funcionando en la bandeja (junto al reloj) y tus amigos pueden seguir jugando.</p><p class="sm t2">Para detener todo y salir: ícono de la bandeja → <b>Salir de 939 Host…</b>. Si la PC se apaga o se suspende, el servidor no puede seguir en esta PC.</p>`, buttons: [{ label: 'Salir y detener', value: 'quit' }, { label: 'Cerrar la ventana', value: 'bg', kind: 'pri' }] });
    if (r === 'bg') invoke('close_to_background').catch(fail);
    else if (r === 'quit') invoke('quit_app');
  });
  await listen('app-quitting', () => {
    if (dialogOpen) dialogOpen(false);
    $('#modal-root').innerHTML = `<div class="overlay"><div class="dialog"><div class="row">${CINTA(48)}<h2>Cerrando 939 Host</h2></div><p>Pasando o guardando los servidores de forma ordenada. Puede tardar hasta un minuto y medio si otra PC está copiando el último estado. No apagues la PC todavía.</p><div class="bar indet"><i></i></div></div></div>`;
  });
  setupDragDrop();
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && dialogOpen) return;
    if ((e.ctrlKey || e.metaKey) && e.key === ',') { e.preventDefault(); go('settings'); }
  });

  if (!state.info.settings.onboardingDone) saveSetting({ onboardingDone: true }, null);
  refreshRequests();
  await go('servers');
  $('#splash')?.classList.add('out');
  // Momento en que la app queda usable (para medir el arranque en las pruebas).
  window.__h939Ready = performance.timeOrigin + performance.now();
  setTimeout(() => $('#splash')?.remove(), 300);
  invoke('notify_ui_ready').catch(() => {});
}

boot();
