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

/* Presentación: se ve al menos ~2,3 s en cada apertura (sin frenar la carga: la app se arma
   debajo) y muestra el paso real del arranque. Con «menos movimiento», se va apenas está lista. */
const splashT0 = performance.now();
const splashStep = (t) => { const el = $('#sp-status'); if (el) el.textContent = t; };
function splashDone() {
  const sp = $('#splash');
  if (!sp) return;
  sp.style.pointerEvents = 'none';
  sp.classList.add('ready');
  const still = (() => { try { return localStorage.getItem('939-motion') === 'reduce'; } catch { return false; } })() || window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  const wait = still ? 0 : Math.max(0, 2300 - (performance.now() - splashT0));
  setTimeout(() => { sp.classList.add('out'); setTimeout(() => sp.remove(), 520); }, wait);
}

/* Espera (como mucho `ms`) a que la consulta de actualizaciones del arranque tenga resultado. */
function waitUpdateResult(ms) {
  return new Promise((res) => {
    const t0 = performance.now();
    const tick = () => {
      const u = state.upd;
      if (u && u.status !== 'buscando') return res(u);
      if (performance.now() - t0 > ms) return res(null);
      setTimeout(tick, 100);
    };
    tick();
  });
}

async function boot() {
  splashStep('Leyendo tu configuración…');
  $('#quit-btn').innerHTML = icon('logout', 16);
  $('#foot-ic').innerHTML = icon('pc', 16);
  try { state.info = await invoke('app_info'); } catch (e) { fail(e); splashStep('No se pudo iniciar'); $('#splash')?.classList.add('out'); return; }
  splashStep('Preparando tu perfil…');
  state.photo = (await invoke('profile_photo_get').catch(() => ({}))).hash || null;
  applyTheme();
  paintFoot();
  refreshFootState();
  refreshTrashCount();
  setInterval(() => { if (!document.hidden) refreshFootState(); }, 30000);

  splashStep('Conectando con los servidores…');
  await setupOps();
  await listen('server-changed', (ev) => scheduleRefresh(ev.payload.id));
  await listen('update-status', (ev) => {
    state.upd = ev.payload;
    refreshUpdatesCard(ev.payload);
    updOnStatus(ev.payload);
    paintUpdateBar(ev.payload);
  });
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
  // Volver con Alt+← o con el botón «atrás» del mouse, como en cualquier programa.
  document.addEventListener('mouseup', (e) => { if (e.button === 3 && !dialogOpen) { e.preventDefault(); goBack(); } });
  document.addEventListener('keydown', (e) => { if (e.altKey && e.key === 'ArrowLeft' && !dialogOpen && !/INPUT|TEXTAREA|SELECT/.test(document.activeElement?.tagName || '')) { e.preventDefault(); goBack(); } });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && dialogOpen) return;
    if ((e.ctrlKey || e.metaKey) && e.key === ',') { e.preventDefault(); go('settings'); }
  });

  if (!state.info.settings.onboardingDone) saveSetting({ onboardingDone: true }, null);
  refreshRequests();
  splashStep('Cargando tus servidores…');
  await go('servers');
  // Actualizaciones: la versión recién instalada confirma que abrió bien (si no, el vigía vuelve a la
  // anterior). Si hay una versión nueva, se espera un instante el resultado de la consulta del
  // arranque, sin pasar el tiempo de la presentación más que lo justo.
  const notice = await invoke('update_confirm').catch(() => null);
  let upd = await invoke('update_status').catch(() => null);
  if (notice?.kind === 'ok') splashStep(`Actualización completada · 939 Host ${notice.version}`);
  else if (upd?.status === 'buscando') {
    splashStep('Buscando actualizaciones…');
    upd = await waitUpdateResult(Math.min(2500, Math.max(800, 2300 - (performance.now() - splashT0)))) || upd;
  }
  const autoUpdate = upd && ['descargando', 'verificando', 'lista'].includes(upd.status) && upd.available !== upd.failed;
  if (!autoUpdate && notice?.kind !== 'ok') splashStep('Listo');
  // Momento en que la app queda usable (para medir el arranque en las pruebas).
  window.__h939Ready = performance.timeOrigin + performance.now();
  splashDone();
  if (autoUpdate) openUpdateScreen(upd, true);
  else paintUpdateBar(upd);
  if (notice || upd?.notice) showUpdateNotice(notice || upd.notice, upd);
  invoke('notify_ui_ready').catch(() => {});
  invoke('last_close_notes').then((notes) => { for (const n of notes || []) showLastClose(n); }).catch(() => {});
  invoke('data_notice').then((m) => { if (m) dialog({ title: 'Datos de 939 Host', body: `<p>${esc(m)}</p>`, buttons: [{ label: 'Entendido', value: 'ok', kind: 'pri' }] }); }).catch(() => {});
}

/* Qué pasó con cada servidor la última vez que 939 Host se cerró de golpe o se apagó Windows. */
function showLastClose(n) {
  const name = `«${esc(n.name)}»`;
  if (n.cause === 'cierre-sin-ver') {
    dialog({ title: 'Último cierre', body: `<p>La última vez, ${name} estaba encendido y se cortó sin que 939 Host pudiera guardarlo (corte de luz, cierre forzado desde el Administrador de tareas o una falla de Windows).</p><p>El mundo quedó como estaba en el último guardado automático de Minecraft. Revisalo antes de seguir; si falta algo o no abre, en <b>Copias</b> podés volver a un punto de restauración. Si le pasás el servidor a otra PC, ese punto queda marcado como «sin confirmar».</p>`, buttons: [{ label: 'Entendido', value: 'ok', kind: 'pri' }] });
    return;
  }
  const why = n.cause === 'cierre-inesperado' ? `939 Host se cerró de forma inesperada mientras ${name} estaba encendido.`
    : n.cause === 'cierre-de-sesion' ? `Se cerró la sesión de Windows mientras ${name} estaba encendido.`
    : `Windows se apagó o se reinició mientras ${name} estaba encendido.`;
  if (n.outcome === 'guardado') { toast(`${n.name}: guardado y detenido correctamente`, 'ok', why); return; }
  const what = n.outcome === 'detenido'
    ? 'El servidor se detuvo, pero el registro de Minecraft no confirmó el guardado. Revisá el mundo; si falta algo, en <b>Copias</b> podés volver a un punto de restauración.'
    : 'El servidor no terminó de guardar a tiempo y se cerró. Puede faltar lo jugado desde el último guardado; en <b>Copias</b> podés volver a un punto de restauración.';
  dialog({ title: 'Último cierre', body: `<p>${why}</p><p>${what}</p>`, buttons: [{ label: 'Entendido', value: 'ok', kind: 'pri' }] });
}

boot();
