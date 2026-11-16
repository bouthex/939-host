'use strict';
/* Pantalla de un servidor: encabezado con estado y controles siempre visibles, y las pestañas
   Resumen, Mundo, Copias, Grupo y Ajustes (Consola, Jugadores y Mods tienen su propio archivo). */

const TABS = [['summary', 'Resumen'], ['console', 'Consola'], ['players', 'Jugadores'], ['mods', 'Mods'], ['world', 'Mundo'], ['files', 'Archivos'], ['backups', 'Copias'], ['group', 'Grupo'], ['settings', 'Ajustes']];
const tabDraw = {};

routes.server = async function renderServer() {
  const id = state.params.id;
  state.server = await invoke('get_server', { id });
  state.share = await invoke('share_status', { id }).catch(() => null);
  if (!TABS.some(([k]) => k === state.tab)) state.tab = 'summary';
  main.innerHTML = `<div class="page wide">
    <div class="crumbs"><button data-a="nav" data-route="servers">Servidores</button><span>/</span><span class="ellipsis">${esc(state.server.config.name)}</span></div>
    <div id="srv-head" class="stack" style="gap:16px"></div>
    <div class="tabs" role="tablist">${TABS.map(([k, v]) => `<button class="tab ${state.tab === k ? 'on' : ''}" role="tab" aria-selected="${state.tab === k}" data-a="tab" data-v="${k}">${v}${k === 'players' && state.server.status === 'running' && state.server.players.length ? `<span class="n">${state.server.players.length}</span>` : ''}</button>`).join('')}</div>
    <div id="srv-tab"></div></div>`;
  drawHeader();
  await drawTab();
  every(1000, tickCountdown);
  every(5000, () => { if (state.route === 'server' && state.server && (isActive(state.server) || state.server.installing)) refreshServer(); });
};
async function drawTab() {
  const el = $('#srv-tab');
  if (!el) return;
  try { await (tabDraw[state.tab] || tabDraw.summary)(el); } catch (e) { el.innerHTML = `<div class="note">${icon('alert')}<div class="grow"><b>No se pudo cargar esta sección</b><span class="sm t2">${esc(errMsg(e))}</span></div></div>`; }
}
A.tab = async (el) => {
  if (!(await canLeave())) return;
  guards.length = 0;
  while (leaveHooks.length) { try { leaveHooks.pop()(); } catch { /* nada */ } }
  state.tab = el.dataset.v;
  document.querySelectorAll('.tabs .tab').forEach((t) => { const on = t.dataset.v === state.tab; t.classList.toggle('on', on); t.setAttribute('aria-selected', on); });
  await drawTab();
  const box = $('#srv-tab');
  if (box) { box.classList.remove('tab-enter'); void box.offsetWidth; box.classList.add('tab-enter'); }
};
function openTab(tab) { const b = document.querySelector(`.tabs .tab[data-v="${tab}"]`); if (b) A.tab(b); }

async function refreshServer() {
  try {
    state.server = await invoke('get_server', { id: state.params.id });
    state.share = await invoke('share_status', { id: state.params.id }).catch(() => state.share);
    drawHeader();
    const pl = document.querySelector('.tabs .tab[data-v="players"]');
    if (pl) pl.innerHTML = `Jugadores${state.server.status === 'running' && state.server.players.length ? `<span class="n">${state.server.players.length}</span>` : ''}`;
    tabRefresh[state.tab]?.();
  } catch { /* el servidor pudo haberse borrado */ }
}
const tabRefresh = {};

/* ───────── encabezado y controles ───────── */
function drawHeader() {
  const v = state.server, c = v.config, el = $('#srv-head');
  if (!el) return;
  const s = serverState(v);
  const prog = state.progress[c.id];
  const id = esc(c.id);
  const busy = ['restarting', 'stopping'].includes(s.key) || v.installing || !c.installed;
  const TONES = { start: 'go', resume: 'go', pause: 'hold', restart: 'cycle', stop: 'halt', repair: 'cycle' };
  const btn = (a, ic, label, opts = {}) => `<button class="btn ${opts.pri ? 'pri' : ''}" data-a="${a}" data-id="${id}" ${TONES[a] ? `data-tone="${TONES[a]}"` : ''} ${opts.disabled ? 'disabled' : ''} ${opts.title ? `title="${esc(opts.title)}"` : ''}>${icon(ic, 16)}${label}</button>`;
  let ctl = '';
  const g = state.share?.enabled ? state.share.group : null;
  const hostedElsewhere = g && !g.iAmHost && g.lease?.state === 'hosting' && !isActive(v);
  if (hostedElsewhere) {
    const host = g.members.find((m) => m.isHost);
    const canReq = g.canRequest && g.canRequest.Ok !== undefined;
    const coHost = g.members.find((m) => m.isMe)?.role !== 'jugador';
    ctl = `<span class="host-chip row" style="gap:8px">${pcAvatar(host?.symbol, 26)}<span class="sm"><span class="t3">Lo aloja</span> <b>${esc(host?.name || g.lease.hostName || 'otra PC')}</b>${g.hostOnline ? '' : ' <span class="t3">· no responde</span>'}</span></span>`
      + (g.outgoingRequest ? `<button class="btn" disabled>${SHAPES.ring}Solicitud enviada</button>`
        : coHost ? `<button class="btn pri" data-a="req-send" data-id="${id}" ${canReq ? '' : `disabled title="${esc(g.canRequest?.Err || '')}"`}>${icon('host', 16)}Solicitar ser anfitrión</button>` : '');
  } else if (s.key === 'stopped' || s.key === 'error') {
    ctl = `${s.key === 'error' && c.installed ? btn('repair', 'wrench', 'Reparar') : ''}${btn('start', 'play', 'Iniciar', { pri: true, disabled: busy })}`;
  } else {
    const pauseBtn = s.key === 'paused' ? btn('resume', 'play', 'Reanudar', { pri: true })
      : btn('pause', 'pause', 'Pausar', { disabled: s.key !== 'running' && s.key !== 'saving' || !v.pauseSupported, title: v.pauseSupported ? 'Congela el mundo con /tick freeze; nadie se desconecta' : 'Pausar necesita Minecraft 1.20.3 o posterior' });
    ctl = `${pauseBtn}${btn('restart', 'restart', 'Reiniciar', { disabled: busy || s.key === 'starting' || !!v.countdown })}${btn('stop', 'stop', s.key === 'stopping' ? 'Deteniendo…' : 'Detener', { disabled: busy || !!v.countdown })}`;
  }
  const changed = state.lastState && state.lastState.id === c.id && state.lastState.key !== s.key;
  state.lastState = { id: c.id, key: s.key };
  const meta = [`${loaderName(c.loader)} ${esc(c.mcVersion)}${c.javaMajor ? ` · Java ${c.javaMajor}` : ''}`, `<span class="mono">localhost:${c.port}</span>`];
  if (v.status === 'running') meta.push(`Encendido hace ${fmtUptime(v.startedAt)}`);
  if (s.detail && !OPS[s.key]) meta.push(esc(s.detail));
  el.innerHTML = `<div class="srv-head">
      <div class="srv-title"><div class="row" style="gap:14px">${hostedElsewhere ? srvLogo(v.logoUrl, 44) : `<button class="logo-btn" data-a="logo-edit" title="Cambiar el logo" aria-label="Cambiar el logo">${srvLogo(v.logoUrl, 44)}</button>`}<h1 class="ellipsis">${esc(c.name)}</h1></div><div class="srv-meta">${hostedElsewhere ? `<span class="statepill">${SHAPES.dot}En otra PC</span>` : stateHTML(v, true).replace('class="statepill"', `class="statepill${changed ? ' changed' : ''}"`)}${meta.map((m) => `<span>${m}</span>`).join('')}</div></div>
      <div class="ctl">${ctl}<button class="btn icon" data-a="folder" data-which="server" title="Abrir la carpeta del servidor" aria-label="Abrir la carpeta del servidor">${icon('folder', 16)}</button></div></div>
    ${v.countdown ? `<div class="countdown" role="status"><div class="row"><span class="num" id="cd-num">${cdLeft(v.countdown)}</span><div><b>${v.countdown.kind === 'restart' ? 'El servidor se reinicia' : 'El servidor se detiene'} en <span id="cd-txt">${cdLeft(v.countdown)}</span> segundos</b><div class="sm t2">Los jugadores ven la cuenta regresiva en el chat. El mundo se guarda antes de cerrar.</div></div></div>
      <div class="btns"><button class="btn s" data-a="cd-cancel" data-id="${id}">Cancelar</button><button class="btn s pri" data-a="cd-now" data-id="${id}" data-kind="${esc(v.countdown.kind)}">${v.countdown.kind === 'restart' ? 'Reiniciar ya' : 'Detener ya'}</button></div></div>` : ''}
    ${!hostedElsewhere && !v.countdown && !(v.installing || (prog && !prog.done)) ? opStrip(v, s) : ''}
    ${v.installing || (prog && !prog.done) ? `<div class="card pad row" style="gap:14px"><span id="inst-ring">${pring(prog?.fraction || 0, 28, !prog)}</span><div class="grow stack" style="gap:6px"><div class="spread"><b>${esc(prog?.message || 'Preparando la instalación')}</b><span class="mono sm">${Math.round((prog?.fraction || 0) * 100)} %</span></div><div class="bar"><i style="width:${Math.round((prog?.fraction || 0) * 100)}%"></i></div></div></div>` : ''}
    ${g?.outgoingRequest ? `<div class="note">${SHAPES.ring}<div class="grow"><b>Le pediste a ${esc(g.outgoingRequest.hostName)} alojar este servidor</b><span class="sm t2">Le apareció un aviso en su 939 Host. La solicitud vence en ${fmtDuration(g.outgoingRequest.expiresInSecs)}.</span></div><button class="btn s" data-a="req-cancel" data-id="${id}">Cancelar</button></div>` : ''}
    ${g?.lastAnswer && !g.outgoingRequest ? `<div class="note ${g.lastAnswer.accepted ? '' : 'strong'}">${icon(g.lastAnswer.accepted ? 'check' : 'info')}<div class="grow"><b>${g.lastAnswer.accepted ? 'Solicitud aceptada' : 'Solicitud sin éxito'}</b><span class="sm t2">${esc(g.lastAnswer.message)}</span></div></div>` : ''}
    ${g?.incomingRequest ? `<div class="note strong">${pcAvatar(g.incomingRequest.symbol, 32)}<div class="grow"><b>${esc(g.incomingRequest.name)} quiere alojar este servidor</b><span class="sm t2">${g.incomingRequest.warnings.length ? `${g.incomingRequest.warnings.length === 1 ? '1 aviso' : `${g.incomingRequest.warnings.length} avisos`} para revisar antes de decidir.` : 'No vemos limitaciones importantes.'}</span></div><div class="btns"><button class="btn s" data-a="req-no" data-id="${id}">Rechazar</button><button class="btn s pri" data-a="req-review" data-id="${id}">Revisar</button></div></div>` : ''}
    ${v.lastError && !v.installing && !isActive(v) ? `<div class="note strong">${icon('alert')}<div class="grow"><b>${!c.installed ? 'La instalación no se completó' : 'El servidor se detuvo con un error'}</b><span class="sm t2">${esc(v.lastError)}</span></div>
      <div class="btns">${!c.installed ? `<button class="btn s pri" data-a="reinstall">Reintentar</button>` : `<button class="btn s" data-a="tab" data-v="console">Ver consola</button><button class="btn s pri" data-a="repair" data-id="${id}">${icon('wrench', 14)}Reparar</button>`}</div></div>` : ''}
    ${v.needsRestart && isActive(v) ? `<div class="note">${icon('restart')}<div class="grow"><b>Hay cambios que se aplican al reiniciar</b><span class="sm t2">Ajustes o mods nuevos quedan activos después de reiniciar el servidor.</span></div><button class="btn s" data-a="restart" data-id="${id}">Reiniciar</button></div>` : ''}`;
}
/* Operación en curso, bien visible: qué pasa, si hay que esperar y si algo falló. */
const OPS = {
  starting: (v) => ({ tone: 'cycle', title: v.phase ? `Iniciando · ${v.phase}` : 'Iniciando', text: 'Minecraft está cargando. Los jugadores pueden entrar cuando aparezca «Listo para jugar».' }),
  checking: () => ({ tone: 'cycle', title: 'Comprobando la conexión', text: '939 Host está consultando al servidor como lo hace Minecraft para confirmar que recibe jugadores.' }),
  saving: () => ({ tone: 'cycle', title: 'Guardando el mundo', text: 'Se escribe el mundo en el disco. No apagues la PC; los jugadores siguen conectados.' }),
  restarting: () => ({ tone: 'cycle', title: 'Reiniciando', text: 'Guardando, apagando y volviendo a encender. Tarda unos segundos.' }),
  stopping: () => ({ tone: 'halt', title: 'Deteniendo', text: 'Guardando el mundo y cerrando de forma ordenada.' }),
  paused: () => ({ tone: 'hold', title: 'Pausado', text: 'El mundo está congelado: no pasa el tiempo ni se mueven los mobs. Nadie se desconecta.', still: true }),
  unreachable: (v) => ({ tone: 'halt', title: 'El servidor no recibe conexiones', text: v.reachError ? `Dice estar encendido, pero la consulta falló: ${v.reachError}` : 'Dice estar encendido, pero no responde a Minecraft.', action: '<button class="btn s" data-a="reach-check" data-tone="cycle">Comprobar de nuevo</button>', still: true }),
};
function opStrip(v, s) {
  const f = OPS[s.key];
  if (!f) return '';
  const o = f(v);
  return `<div class="opstrip" data-tone="${o.tone}" role="status" aria-live="polite">${o.still ? `<span class="op-ic">${icon(s.key === 'paused' ? 'pause' : 'alert', 16)}</span>` : '<span class="op-spin" aria-hidden="true"></span>'}
    <div class="grow"><b>${esc(o.title)}</b><span class="sm t2">${esc(o.text)}</span></div>${o.action || ''}${o.still ? '' : '<span class="op-bar" aria-hidden="true"><i></i></span>'}</div>`;
}
function cdLeft(cd) { return Math.max(0, Math.ceil((new Date(cd.endsAt).getTime() - Date.now()) / 1000)); }
function tickCountdown() {
  const cd = state.server?.countdown;
  if (!cd) return;
  const n = cdLeft(cd);
  const a = $('#cd-num'), b = $('#cd-txt');
  if (a) a.textContent = n;
  if (b) b.textContent = n;
}

/* Detener o reiniciar: si hay jugadores, ofrece avisarles con una cuenta regresiva. */
async function stopFlow(id, kind) {
  let v = state.servers.find((s) => s.config.id === id) || (state.server?.config.id === id ? state.server : null);
  try { v = await invoke('get_server', { id }); } catch { /* usar lo que hay */ }
  const players = v && v.status === 'running' ? v.players : [];
  let seconds = 0;
  if (players.length) {
    const names = players.length <= 3 ? players.map(esc).join(', ').replace(/, ([^,]*)$/, ' y $1') : `${esc(players.slice(0, 2).join(', '))} y ${players.length - 2} más`;
    const verb = kind === 'restart' ? 'reiniciar' : 'detener';
    const r = await dialog({
      title: kind === 'restart' ? 'Reiniciar con jugadores conectados' : 'Detener con jugadores conectados',
      body: `<p>Hay <b>${players.length === 1 ? '1 jugador conectado' : `${players.length} jugadores conectados`}</b> (${names}). 939 Host guarda el mundo antes de cerrar: no se pierde nada.</p><p class="sm">Con aviso, ven una cuenta regresiva en el chat y pueden terminar lo que están haciendo.</p>`,
      buttons: [{ label: `${kind === 'restart' ? 'Reiniciar' : 'Detener'} ahora`, value: 'now' }, { label: `Avisar y ${verb} en 30 s`, value: 'warn', kind: 'pri' }],
    });
    if (!r) return;
    seconds = r === 'warn' ? 30 : 0;
  }
  const cmd = kind === 'restart' ? 'restart_server_warned' : 'stop_server_warned';
  if (!seconds) toast(kind === 'restart' ? 'Reiniciando…' : 'Guardando y deteniendo…');
  invoke(cmd, { id, seconds }).then((ok) => { if (ok && seconds) toast(kind === 'restart' ? 'Reiniciando…' : 'Guardando y deteniendo…'); }).catch(fail);
  setTimeout(() => { if (state.route === 'server') refreshServer(); else if (state.route === 'servers') routes.servers(); }, 400);
}
A['req-send'] = async (el) => {
  el.disabled = true;
  try { const host = await invoke('host_request_send', { id: el.dataset.id }); toast(`Solicitud enviada a ${host}`); } catch (e) { fail(e); }
  refreshServer();
};
A['req-cancel'] = async (el) => {
  el.disabled = true;
  try { await invoke('host_request_cancel', { id: el.dataset.id }); toast('Solicitud cancelada'); } catch (e) { fail(e); }
  refreshServer();
};
A.restart = (el) => stopFlow(el.dataset.id || state.params.id, 'restart');
A.pause = async (el) => { el.disabled = true; try { await invoke('pause_server', { id: el.dataset.id }); toast('Mundo en pausa. Los jugadores siguen conectados.'); } catch (e) { fail(e); } refreshServer(); };
A.resume = async (el) => { el.disabled = true; try { await invoke('resume_server', { id: el.dataset.id }); toast('Mundo reanudado'); } catch (e) { fail(e); } refreshServer(); };
A['cd-cancel'] = async (el) => { try { await invoke('cancel_countdown', { id: el.dataset.id }); toast('Cancelado: el servidor sigue encendido'); } catch (e) { fail(e); } refreshServer(); };
A['cd-now'] = async (el) => {
  const id = el.dataset.id;
  try { await invoke('cancel_countdown', { id }); } catch { /* ya terminó */ }
  invoke(el.dataset.kind === 'restart' ? 'restart_server_warned' : 'stop_server_warned', { id, seconds: 0 }).catch(fail);
  setTimeout(refreshServer, 400);
};
A.reinstall = async () => {
  if (!(await dialog({ title: 'Reinstalar archivos del servidor', body: '<p>Se vuelven a descargar y verificar el servidor y Java. El mundo, la configuración y las copias no se tocan.</p>', confirm: 'Reinstalar' }))) return;
  try { await invoke('reinstall_server', { id: state.params.id }); toast('Reinstalando…'); refreshServer(); } catch (e) { fail(e); }
};

A['reach-check'] = async (el) => {
  el.setAttribute('aria-busy', 'true');
  try { await invoke('server_check_connection', { id: state.params.id }); } catch (e) { fail(e); }
  setTimeout(() => el.removeAttribute('aria-busy'), 800);
};
A['issue-ok'] = async () => { try { await invoke('server_clear_join_issue', { id: state.params.id }); refreshServer(); } catch (e) { fail(e); } };

/* ───────── Resumen ───────── */
tabDraw.summary = async (el) => {
  if (!state.aliases) await loadAliases();
  const v = state.server, c = v.config;
  const running = v.status === 'running';
  const gg = state.share?.enabled ? state.share.group : null;
  const remoteHost = gg && !gg.iAmHost && gg.lease?.state === 'hosting' && !isActive(v) ? (gg.members.find((m) => m.isHost)?.name || gg.lease.hostName) : null;
  const ji = v.joinIssue;
  el.innerHTML = `<div class="stack" style="gap:16px">
    ${ji ? `<div class="issue" role="status">${icon('alert', 18)}<div class="stack" style="gap:4px"><b>${esc(ji.title)}</b><span class="sm t2">${esc(ji.detail)}</span><span class="mono" title="Línea de la consola del servidor">${esc(ji.logLine)}</span></div>
      <div class="btns">${c.loader !== 'vanilla' ? `<button class="btn s" data-a="tab" data-v="mods">${icon('box', 14)}Mods del servidor</button>` : ''}<button class="btn s quiet" data-a="issue-ok">Entendido</button></div></div>` : ''}
    <div class="kpis">
      <div class="card kpi"><span class="l">Jugadores</span><span class="v">${running ? v.players.length : '—'}<small>de ${c.maxPlayers}</small></span></div>
      <div class="card kpi"><span class="l">Tiempo por tick</span><span class="v">${v.mspt != null ? v.mspt.toFixed(1).replace('.', ',') : '—'}<small>ms${v.msptP95 != null ? ` · p95 ${v.msptP95.toFixed(1).replace('.', ',')}` : ''}</small></span><span class="xs t3">${v.mspt == null ? 'Disponible con el servidor encendido' : v.mspt < 40 ? 'Fluido (menos de 50 ms)' : v.mspt < 50 ? 'Al límite' : 'Con demoras'}</span></div>
      <div class="card kpi"><span class="l">Memoria</span><span class="v">${v.memoryUsedMb != null ? fmtMB(v.memoryUsedMb) : '—'}<small>en uso · ${fmtMB(c.memoryMb)} asignados al mundo</small></span>${v.memoryUsedMb != null ? `<div class="bar"><i style="width:${Math.min(100, Math.round(v.memoryUsedMb / c.memoryMb * 100))}%"></i></div>` : ''}</div>
      <div class="card kpi"><span class="l">Mundo</span><span class="v">${fmtMB(v.worldSizeMb)}</span><span class="xs t3">${DIFF[c.difficulty] || c.difficulty} · ${MODE[c.gamemode] || c.gamemode}</span></div>
    </div>
    <div class="grid2">
      <div class="stack" style="gap:16px">
        <div class="card pad stack"><div class="spread"><h2>Conexión</h2>${running ? `<button class="btn s quiet" data-a="reach-check" data-tone="cycle">${icon('sync', 14)}Comprobar</button>` : ''}</div>
          <div>
          <div class="kv"><span class="k">Servidor</span>${!running ? `<span class="chk">${isActive(v) ? 'Arrancando' : 'Detenido'}</span>`
            : v.reach === 'ok' ? `<span class="chk ok">Listo · Minecraft ${esc(v.reachInfo?.version || c.mcVersion)} responde en ${v.reachInfo?.latencyMs ?? '–'} ms</span>`
            : v.reach === 'fail' ? `<span class="chk bad" title="${esc(v.reachError || '')}">No atiende conexiones</span>`
            : '<span class="chk wait">Comprobando que reciba jugadores…</span>'}</div>
          <div class="kv"><span class="k">Para entrar hace falta</span><span class="stack" style="gap:6px;align-items:flex-end"><span class="ta-r">Minecraft <b>${esc(c.mcVersion)}</b>${c.loader === 'vanilla' ? ' sin mods' : ` con ${loaderName(c.loader)}${v.modsCount ? ` y los mismos ${v.modsCount} mods` : ' (el servidor no tiene mods)'}`}</span>${profileButton(c, v)}</span></div>
          ${aliasRow(c.id)}
          <div class="kv"><span class="k">Desde esta PC</span><span class="row" style="gap:6px"><span class="mono">localhost:${c.port}</span><button class="btn s icon ghost" data-a="copy" data-text="localhost:${c.port}" data-msg="Dirección copiada" aria-label="Copiar">${icon('copy', 14)}</button></span></div>
          ${gg ? (() => {
            const others = gg.members.filter((m) => !m.isMe);
            const on = others.filter((m) => m.online);
            const paths = [...new Set(on.map((m) => m.path).filter(Boolean))].join(' · ');
            return `<div class="kv"><span class="k">Grupo</span><span class="row" style="gap:8px">${state.share.routePort ? `<span class="mono">127.0.0.1:${state.share.routePort}</span>` : ''}<span class="chk ${on.length ? 'ok' : others.length ? 'bad' : ''}">${others.length ? `${on.length} de ${others.length} PCs conectadas${paths ? ` · ${esc(paths)}` : ''}` : 'Nadie más en el grupo'}</span></span></div>`;
          })() : `<div class="kv"><span class="k">Desde otras casas</span><button class="btn s ghost" data-a="tab" data-v="group">Invitar con un código${icon('chev', 14)}</button></div>`}
          <div class="kv"><span class="k">Desde tu red</span><span class="t2">IP de esta PC + puerto ${c.port}</span></div></div></div>
        <div class="card pad stack"><h2>Identidad del servidor</h2>
          <p class="sm t2" style="margin:0">Identifica a este servidor en todas las PCs del grupo. No depende de la IP: sigue siendo el mismo aunque cambie la PC anfitriona.</p>
          <div class="addr"><span>${esc(c.identity.accessCode)}</span><button class="btn s" data-a="copy" data-text="${esc(c.identity.accessCode)}">${icon('copy', 14)}Copiar</button></div></div>
      </div>
      <div class="stack" style="gap:16px">
        <div class="card"><div class="card-head"><h2>Conectados ahora</h2>${running ? '<span class="live"><span class="dot"></span>En vivo</span>' : ''}</div>
          ${running && v.players.length ? `<div class="list">${v.players.map((p) => `<button class="li" data-a="open-player" data-name="${esc(p)}"><span class="avatar" data-skin="${esc(p)}">${esc(initials(p))}</span><span class="grow b">${esc(p)}</span>${icon('chev', 16)}</button>`).join('')}</div>`
            : `<div class="pad sm t2">${remoteHost ? `Se está jugando en la PC de <b>${esc(remoteHost)}</b>. Entrá con la dirección del grupo (pestaña Grupo).` : running ? 'No hay nadie conectado.' : 'El servidor está detenido.'}</div>`}</div>
        <div class="card pad stack"><div class="spread"><h2>Configuración</h2><button class="btn s ghost" data-a="tab" data-v="settings">Editar${icon('chev', 14)}</button></div>
          <div><div class="kv"><span class="k">Distancias</span><span>${c.viewDistance} de visión · ${c.simulationDistance} de simulación</span></div>
          <div class="kv"><span class="k">Lista blanca</span><span>${c.whitelist ? 'Activada' : 'Desactivada'}</span></div>
          <div class="kv"><span class="k">Cuentas</span><span>${c.onlineMode ? 'Verificadas con Mojang' : 'Sin verificar'}</span></div>
          <div class="kv"><span class="k">Java</span><span>${{ balanced: 'Equilibrado (G1)', lowlatency: 'Baja latencia (ZGC)', custom: 'Personalizado' }[c.jvmProfile] || c.jvmProfile}</span></div></div></div>
      </div></div></div>`;
  paintSkins(el);
};
tabRefresh.summary = () => { const el = $('#srv-tab'); if (el) tabDraw.summary(el); };
A['open-player'] = (el) => { state.playerSel = el.dataset.name; openTab('players'); };

/* ───────── Mundo ───────── */
tabDraw.world = async (el) => {
  const c = state.server.config;
  const w = await invoke('world_info', { id: c.id });
  const active = isActive(state.server);
  el.innerHTML = `<div class="stack" style="gap:16px">
    <div class="kpis">
      <div class="card kpi"><span class="l">Tamaño</span><span class="v">${fmtMB(w.sizeBytes / 1048576)}</span></div>
      <div class="card kpi"><span class="l">Dimensiones</span><span class="v" style="font-size:16px">${w.dimensions.length ? esc(w.dimensions.join(' · ')) : '—'}</span></div>
      <div class="card kpi"><span class="l">Jugadores con datos</span><span class="v">${w.playersData}</span></div>
      <div class="card kpi"><span class="l">Semilla</span><span class="v mono" style="font-size:14px;overflow-wrap:anywhere">${esc(w.seed || c.levelSeed || '—')}</span></div>
    </div>
    ${c.loader !== 'vanilla' ? await (async () => { try { const ml = await invoke('mods_list', { id: c.id, online: false }); return hasChunky(ml) ? chunkyHTML(c.id) : ''; } catch { return ''; } })() : ''}
    ${!w.exists ? `<div class="note">${icon('info')}<div class="grow"><b>Todavía no hay mundo</b><span class="sm t2">Se genera la primera vez que inicies el servidor.</span></div></div>` : ''}
    <div class="card list">
      <div class="li"><span class="srv-ic">${icon('folder')}</span><div class="grow"><b>Abrir la carpeta del mundo</b><div class="sm t2">«${esc(w.levelName)}» dentro de la carpeta del servidor.</div></div><button class="btn s" data-a="folder" data-which="world">Abrir</button></div>
      <div class="li"><span class="srv-ic">${icon('upload')}</span><div class="grow"><b>Reemplazar por una copia de otro mundo</b><div class="sm t2">Se copia; el original no se modifica. El mundo actual va a la Papelera de 939 Host.</div></div><button class="btn s" data-a="import-world" ${active ? 'disabled title="Detené el servidor primero"' : ''}>Elegir…</button></div>
      <div class="li"><span class="srv-ic">${icon('clock')}</span><div class="grow"><b>Volver a un momento anterior</b><div class="sm t2">Los puntos de restauración están en Copias.</div></div><button class="btn s" data-a="tab" data-v="backups">Ver copias</button></div>
      <div class="li"><span class="srv-ic">${icon('globe')}</span><div class="grow"><b>Empezar un mundo nuevo</b><div class="sm t2">El actual queda guardado como punto de restauración y en la Papelera.</div></div><button class="btn s" data-a="world-new" ${active ? 'disabled title="Detené el servidor primero"' : ''}>Mundo nuevo…</button></div>
    </div></div>`;
};
A['import-world'] = async () => {
  if (isActive(state.server)) { toast('Detené el servidor antes de reemplazar el mundo.', 'error'); return; }
  const path = await TAURI.dialog.open({ directory: true, title: 'Elegí la carpeta del mundo (la que tiene level.dat)' });
  if (!path) return;
  try { await invoke('validate_world', { path }); } catch (e) { fail(e); return; }
  if (!(await dialog({ title: 'Reemplazar el mundo', body: `<p>Se copia <span class="mono sm">${esc(path)}</span> al servidor. El original no se modifica. El mundo actual va a la Papelera de 939 Host.</p>`, confirm: 'Copiar mundo' }))) return;
  try { const mb = await invoke('import_world', { id: state.params.id, path }); toast(`Mundo copiado (${fmtMB(mb)})`); } catch (e) { fail(e); }
  drawTab();
};
A['world-new'] = async () => {
  const name = state.server.config.name;
  if (isActive(state.server)) { toast('Detené el servidor primero.', 'error'); return; }
  let seed = '';
  const ok = await dialog({ title: 'Empezar un mundo nuevo', body: '<p>El mundo actual se guarda como punto de restauración y se mueve a la Papelera. Al iniciar, se genera uno nuevo.</p><label class="field">Semilla <span class="hint">Vacío = la configurada</span><input class="input mono" id="wn-seed" maxlength="64"></label>', confirm: 'Crear mundo nuevo', requireText: name, onMount: (d) => d.querySelector('#wn-seed').addEventListener('input', (e) => { seed = e.target.value; }) });
  if (!ok) return;
  try { await invoke('world_new', { id: state.params.id, seed: seed.trim() || null, confirmName: name }); toast('Listo: el próximo inicio genera un mundo nuevo'); } catch (e) { fail(e); }
  drawTab();
};

/* ───────── Copias ───────── */
tabDraw.backups = async (el) => {
  const id = state.params.id, c = state.server.config;
  const [snaps, zips] = await Promise.all([invoke('snapshots_list', { id }), invoke('list_backups', { id })]);
  const running = state.server.status === 'running';
  const st = snaps.stats;
  const KIND = { auto: 'Automático', manual: 'Manual', seguridad: 'Seguridad' };
  const groups = {};
  for (const s of snaps.items) { const d = new Date(s.createdAt).toLocaleDateString('es-AR', { weekday: 'long', day: 'numeric', month: 'long' }); (groups[d] = groups[d] || []).push(s); }
  el.innerHTML = `<div class="stack" style="gap:16px">
    <div class="kpis">
      <div class="card kpi"><span class="l">Puntos de restauración</span><span class="v">${st.snapshots}</span></div>
      <div class="card kpi"><span class="l">Espacio que ocupan</span><span class="v">${fmtMB(st.storeBytes / 1048576)}</span><span class="xs t3">Equivale a copias de ${fmtMB(st.logicalBytes / 1048576)}</span></div>
      <div class="card kpi"><span class="l">Automático cada</span><span class="v">${c.restoreIntervalMin}<small>min</small></span><span class="xs t3">Con el servidor encendido</span></div>
      <div class="card kpi"><span class="l">Copias .zip</span><span class="v">${zips.length}</span></div>
    </div>
    <div class="card pad stack"><div class="spread"><h2>Crear un punto ahora</h2><span class="sm t3">${running ? 'Nadie se desconecta: pausa el guardado unos milisegundos.' : 'Guarda solo lo que cambió desde el último punto.'}</span></div>
      <div class="row"><input class="input" id="snap-label" placeholder="Nombre (opcional), por ejemplo: antes del jefe" maxlength="80" style="max-width:420px"><button class="btn pri" data-a="snap-take">${icon('plus', 16)}Crear punto</button></div></div>
    <div class="card">${snaps.items.length ? Object.entries(groups).map(([day, items]) => `<div class="sec" style="padding:14px 20px 6px">${esc(day)}</div><div class="list">
      ${items.map((s) => `<div class="li"><span class="mono" style="width:52px;white-space:nowrap">${new Date(s.createdAt).toLocaleTimeString('es-AR', { hour: '2-digit', minute: '2-digit', hour12: false })}</span>
        <div class="grow"><div class="row" style="gap:8px"><b class="ellipsis">${esc(s.label)}</b><span class="chip s">${KIND[s.kind] || esc(s.kind)}</span></div><div class="sm t3">+${fmtMB(s.newBytes / 1048576)} nuevos · mundo de ${fmtMB(s.totalBytes / 1048576)} · ${s.fileCount} archivos</div></div>
        <div class="btns"><button class="btn s" data-a="snap-restore" data-snap="${esc(s.id)}">Restaurar</button><button class="btn s icon ghost" data-tone="danger" data-a="snap-delete" data-snap="${esc(s.id)}" aria-label="Borrar punto">${icon('trash', 15)}</button></div></div>`).join('')}</div>`).join('')
      : `<div class="pad sm t2">Todavía no hay puntos. Con el servidor encendido se crea uno cada ${c.restoreIntervalMin} minutos; también podés crear uno ahora.</div>`}</div>
    <p class="xs t3" style="margin:0">Se conservan todos los de las últimas 6 h, uno por hora durante 2 días, uno por día durante 14 días y uno por semana durante 8 semanas. Los manuales no se borran solos.</p>
    <div class="card pad stack"><div class="spread"><div><h2>Copias completas (.zip)</h2><p class="sm t2" style="margin:2px 0 0">Un archivo independiente con el mundo y la configuración, para guardarlo afuera o compartirlo.</p></div>
      <div class="btns"><button class="btn" data-a="backup">${icon('download', 16)}Crear copia .zip</button><button class="btn icon ghost" data-a="folder" data-which="backups" aria-label="Abrir carpeta de copias">${icon('folder', 16)}</button></div></div>
      ${zips.length ? `<div>${zips.map((b) => `<div class="kv"><span class="ellipsis">${esc(fmtDate(b.createdAt))} · ${esc(b.label)} <span class="mono sm t3">${fmtMB(b.sizeMb)}</span></span><span class="btns"><button class="btn s" data-a="restore" data-file="${esc(b.file)}">Restaurar</button><button class="btn s icon ghost" data-tone="danger" data-a="del-backup" data-file="${esc(b.file)}" aria-label="Borrar copia">${icon('trash', 15)}</button></span></div>`).join('')}</div>` : ''}
    </div></div>`;
};
A['snap-take'] = async (el) => {
  el.disabled = true;
  try { const i = await invoke('snapshot_take', { id: state.params.id, label: $('#snap-label').value }); toast(`Punto creado: ${fmtMB(i.newBytes / 1048576)} nuevos`); } catch (e) { fail(e); }
  drawTab();
};
A['snap-restore'] = async (el) => {
  if (isActive(state.server)) { toast('Detené el servidor antes de restaurar.', 'error'); return; }
  if (!(await dialog({ title: 'Restaurar este punto', body: '<p>Antes de restaurar, 939 Host guarda el estado actual como punto de «Seguridad» y aparta el mundo actual en la Papelera. Se puede deshacer restaurando ese punto.</p>', confirm: 'Restaurar' }))) return;
  try { await invoke('snapshot_restore', { id: state.params.id, snapId: el.dataset.snap }); toast('Punto restaurado'); } catch (e) { fail(e); }
  drawTab();
};
A['snap-delete'] = async (el) => {
  if (!(await dialog({ title: 'Borrar punto de restauración', body: '<p>Se borra este punto. Los datos que comparte con otros puntos se conservan.</p>', confirm: 'Borrar' }))) return;
  try { await invoke('snapshot_delete', { id: state.params.id, snapId: el.dataset.snap }); } catch (e) { fail(e); }
  drawTab();
};
A.backup = async (el) => {
  el.disabled = true; el.textContent = 'Creando…';
  try { await invoke('create_backup', { id: state.params.id, label: 'Copia completa' }); toast('Copia .zip creada'); } catch (e) { fail(e); }
  drawTab();
};
A.restore = async (el) => {
  if (isActive(state.server)) { toast('Detené el servidor antes de restaurar.', 'error'); return; }
  if (!(await dialog({ title: 'Restaurar esta copia', body: '<p>Antes de restaurar, 939 Host guarda el estado actual como copia «Antes de restaurar» y aparta el mundo actual en la Papelera. Se puede deshacer.</p>', confirm: 'Restaurar' }))) return;
  try { await invoke('restore_backup', { id: state.params.id, file: el.dataset.file }); toast('Copia restaurada'); } catch (e) { fail(e); }
  drawTab();
};
A['del-backup'] = async (el) => {
  if (!(await dialog({ title: 'Borrar copia', body: '<p>Este archivo se elimina del disco y no se puede recuperar.</p>', confirm: 'Borrar' }))) return;
  try { await invoke('delete_backup', { id: state.params.id, file: el.dataset.file }); } catch (e) { fail(e); }
  drawTab();
};

/* ───────── Grupo ───────── */
const ROLE = { propietario: 'Propietario', coanfitrion: 'Coanfitrión', jugador: 'Jugador' };
tabDraw.group = async (el) => {
  const st = await invoke('share_status', { id: state.params.id });
  state.share = st;
  const g = st.group;
  const head = `<div class="card pad"><label class="switch"><div class="grow"><b>Compartir con el grupo</b><span class="sm t2">Tus amigos juegan desde sus casas sin abrir puertos, y el servidor puede pasar de una PC a otra sin perder progreso. Conexión directa cifrada${st.relays ? '; relays públicos gratuitos solo como último recurso' : ' (relays públicos desactivados en Configuración)'}.</span></div><input type="checkbox" id="share-toggle" ${st.enabled ? 'checked' : ''}></label></div>`;
  const inviteBtns = `<div class="btns"><button class="btn pri" data-a="invite" data-role="coanfitrion">${icon('plus', 16)}Invitar coanfitrión</button><button class="btn" data-a="invite" data-role="jugador">${icon('plus', 16)}Invitar jugador</button></div>`;
  if (!st.enabled || !g) {
    el.innerHTML = `<div class="stack" style="gap:16px">${head}
      <div class="grid2"><div class="card pad stack">${icon('friends', 22)}<h2>Coanfitrión</h2><p class="sm t2" style="margin:0">Guarda una copia sincronizada y puede hostear el servidor cuando tu PC no esté.</p></div>
      <div class="card pad stack">${icon('user', 22)}<h2>Jugador</h2><p class="sm t2" style="margin:0">Entra a jugar con una dirección fija. No guarda copia ni hostea.</p></div></div>
      ${inviteBtns}</div>`;
  } else {
    const l = g.lease;
    const hostName = l ? (l.host === g.me ? 'Esta PC' : (l.hostName || 'otra PC')) : '—';
    const hostState = !l ? '' : l.state === 'released' ? 'libre: nadie lo está hosteando' : g.hostOnline ? 'en línea' : 'no responde';
    const online = g.members.filter((m) => m.online).length;
    const eligible = g.members.filter((m) => !m.isMe && m.online && m.canHost);
    el.innerHTML = `<div class="stack" style="gap:16px">${head}
      <div class="grid2"><div class="stack" style="gap:16px">
        <div class="card pad stack"><h2>Dirección para jugar</h2>
          <div class="addr"><span>127.0.0.1:${st.routePort}</span><button class="btn s" data-a="copy" data-text="127.0.0.1:${st.routePort}" data-msg="Dirección copiada">${icon('copy', 14)}Copiar</button></div>
          <p class="sm t2" style="margin:0">Desde cualquier PC del grupo. No cambia aunque cambie la PC anfitriona.</p></div>
        <div class="card pad stack"><h2>Anfitrión</h2>
          ${g.busy ? `<div class="note">${SHAPES.ring}<div class="grow"><b>${esc(g.busy)}…</b></div></div>` : ''}
          ${g.fenced ? `<div class="note strong">${icon('alert')}<div class="grow"><b>Servidor detenido en esta PC por seguridad</b><span class="sm t2">Se perdió el contacto con el grupo u otra PC tomó el servidor. Tu copia quedó como punto de seguridad.</span></div></div>` : ''}
          <p class="sm t2" style="margin:0">939 Host elige solo la mejor PC cuando hace falta. ${g.iAmHost ? 'Cualquier coanfitrión puede pedirte alojarlo: te va a llegar un aviso para aceptar o rechazar.' : 'Si querés alojarlo desde esta PC, pedíselo a la anfitriona: ella decide.'}</p>
          <div><div class="kv"><span class="k">Ahora</span><span><b>${esc(hostName)}</b>${l ? ` <span class="sm t3">· ${esc(hostState)} · época ${l.epoch}</span>` : ''}</span></div>
          ${g.myLatest ? `<div class="kv"><span class="k">Último estado en esta PC</span><span>${esc(ago(g.myLatest.createdAt))}</span></div>` : ''}</div>
          <div class="btns">
            ${g.iAmHost ? `<button class="btn" data-a="handover" ${eligible.length ? '' : 'disabled title="No hay otra PC coanfitriona conectada"'}>${icon('restart', 16)}Pasar a otra PC…</button><button class="btn" data-a="release">Guardar y soltar</button>` : ''}
            ${!g.iAmHost && g.hasCopy && l && l.state === 'hosting' && g.hostOnline && !g.outgoingRequest ? `<button class="btn pri" data-a="req-send" data-id="${esc(state.params.id)}" ${g.canRequest?.Ok !== undefined ? '' : `disabled title="${esc(g.canRequest?.Err || '')}"`}>${icon('host', 16)}Solicitar ser anfitrión</button>` : ''}
            ${g.outgoingRequest ? `<span class="row sm" style="gap:8px">${SHAPES.ring}Esperando a ${esc(g.outgoingRequest.hostName)}</span><button class="btn" data-a="req-cancel" data-id="${esc(state.params.id)}">Cancelar solicitud</button>` : ''}
            ${!g.iAmHost && g.hasCopy && l && l.state === 'hosting' && !g.hostOnline ? `<button class="btn" data-a="take-here">Recuperar aquí</button>` : ''}
            ${!g.iAmHost && g.hasCopy ? `<button class="btn ghost" data-a="group-sync">${icon('refresh', 16)}Sincronizar ahora</button>` : ''}
          </div>
          ${g.canStart && g.canStart.Err && !g.iAmHost ? `<p class="sm t3" style="margin:0">${esc(g.canStart.Err)}</p>` : ''}</div>
        <div class="card pad stack"><h2>Opciones</h2><div>
          <label class="switch"><div class="grow"><b>Recuperación automática</b><span class="sm t2">Si el anfitrión deja de responder, las PCs conectadas eligen otra por mayoría (hacen falta ${g.majority} de ${g.members.length}).${g.members.length < 3 ? ' Con dos PCs no hay mayoría posible: queda «Recuperar aquí».' : ''}</span></div><input type="checkbox" id="opt-failover" ${g.autoFailover ? 'checked' : ''}></label>
          <label class="switch"><div class="grow"><b>Pasar al salir</b><span class="sm t2">Al salir de 939 Host, pasar el servidor a otra PC conectada en vez de apagarlo.</span></div><input type="checkbox" id="opt-handover" ${g.handoverOnQuit ? 'checked' : ''}></label></div></div>
      </div>
      <div class="stack" style="gap:16px">
        <div class="card"><div class="card-head"><h2>PCs del grupo</h2><span class="sm t3">${online} de ${g.members.length} conectadas</span></div><div class="list">
          ${g.members.map((m) => `<div class="li">${pcAvatar(m.symbol, 32, m.isHost ? 'inv' : '')}<div class="grow"><div class="row" style="gap:8px"><b class="ellipsis">${esc(m.name)}</b>${m.isMe ? '<span class="sm t3">(vos)</span>' : ''}${m.isHost ? '<span class="chip s fill">Anfitrión</span>' : ''}<span class="chip s">${ROLE[m.role] || esc(m.role)}</span></div>
            <div class="sm t3">${m.online ? `${SHAPES.dot.replace('class="dot"', 'class="dot" style="display:inline-block;width:6px;height:6px;margin-right:4px"')}Conectada${m.path ? ` · ${m.path === 'relay' ? 'por relay' : 'directa'}${m.rttMs != null ? ` · ${m.rttMs} ms` : ''}` : ''}` : 'Desconectada'}${m.canHost ? ` · estado ${esc(ago(m.latestAt))}` : ''}</div></div>
            ${m.isMe ? `<button class="btn s icon ghost" data-a="identity" title="Cambiar el nombre de esta PC" aria-label="Cambiar el nombre de esta PC">${icon('edit', 15)}</button>` : ''}${!m.isMe && m.role !== 'propietario' && g.members.find((x) => x.isMe)?.role === 'propietario' ? `<button class="btn s ghost" data-a="member-transfer" data-node="${esc(m.nodeId)}" data-name="${esc(m.name)}">Hacer propietaria</button>` : ''}${!m.isMe && m.role !== 'propietario' && !m.isHost && g.members.find((x) => x.isMe)?.role === 'propietario' ? `<button class="btn s ghost" data-a="member-remove" data-node="${esc(m.nodeId)}" data-tone="halt">Quitar</button>` : ''}</div>`).join('')}</div>
          ${g.members.find((x) => x.isMe)?.role !== 'propietario' ? `<div class="spread" style="margin-top:12px"><span class="sm t2">Si salís, esta PC deja de participar y la propietaria te quita de la lista.</span><button class="btn s" data-a="group-leave" data-tone="halt">Salir del grupo</button></div>` : '<p class="sm t2" style="margin-top:12px">Los cambios del grupo los firma esta PC (propietaria): ninguna otra puede agregarse, quitar a otras ni declararse propietaria. Para irte, primero pasá la propiedad.</p>'}</div>
        <div class="card pad stack"><h2>Invitar</h2>${inviteBtns}
          ${st.invites.length ? `<div><div class="sec" style="margin:6px 0">Invitaciones sin usar</div>${st.invites.map((i) => `<div class="kv"><span>${ROLE[i.role] || 'Jugador'} <span class="sm t3">${esc(fmtDate(i.createdAt))}</span></span><button class="btn s ghost" data-a="invite-revoke" data-secret="${esc(i.secret)}">Revocar</button></div>`).join('')}</div>` : ''}</div>
      </div></div></div>`;
    const fo = $('#opt-failover', el), ho = $('#opt-handover', el);
    const opts = () => invoke('group_options', { id: state.params.id, autoFailover: fo.checked, handoverOnQuit: ho.checked }).then(() => toast('Opciones del grupo guardadas')).catch(fail);
    fo.addEventListener('change', opts);
    ho.addEventListener('change', opts);
  }
  $('#share-toggle', el).addEventListener('change', async (e) => {
    try { await invoke('share_set', { id: state.params.id, enabled: e.target.checked }); toast(e.target.checked ? 'Servidor compartido con el grupo' : 'Dejaste de compartir'); } catch (err) { fail(err); }
    drawTab();
  });
};
A.invite = async (el) => {
  const role = el.dataset.role || 'jugador';
  try {
    const ticket = await invoke('invite_create', { id: state.params.id, role, label: role });
    await dialog({ title: role === 'coanfitrion' ? 'Invitación de coanfitrión' : 'Invitación de jugador', body: `<div class="invite-card">${srvLogo(state.server.logoUrl, 48)}<div><b>${esc(state.server.config.name)}</b><div class="sm t2">${loaderName(state.server.config.loader)} ${esc(state.server.config.mcVersion)} · quien se una ve este logo en «Servidores de amigos»</div></div></div><p class="sm">${role === 'coanfitrion' ? 'La PC que la use guarda una copia sincronizada y puede hostear el servidor cuando esta no esté.' : 'La PC que la use puede jugar, pero no guarda copia ni hostea.'} Se pega en 939 Host → Servidores de amigos → Unirme. Sirve para una sola PC; esta PC tiene que tener 939 Host abierto cuando la usen.</p><div class="addr"><span class="sm">${esc(ticket)}</span></div>`, confirm: 'Copiar y cerrar', cancel: null });
    try { await navigator.clipboard.writeText(ticket); toast('Código copiado'); } catch { /* sin portapapeles */ }
  } catch (e) { fail(e); }
  drawTab();
};
A['invite-revoke'] = async (el) => { try { await invoke('invite_revoke', { id: state.params.id, secret: el.dataset.secret }); toast('Invitación revocada'); } catch (e) { fail(e); } drawTab(); };
A['member-transfer'] = async (el) => {
  const id = state.server.config.id;
  if (!(await dialog({ title: `Pasar la propiedad a ${el.dataset.name}`, confirm: 'Pasar la propiedad', danger: true,
    body: `<p>${esc(el.dataset.name)} va a ser la PC propietaria: podrá invitar, quitar PCs y borrar el servidor. Esta PC queda como integrante.</p><p class="sm t2">El cambio lo firma esta PC y todo el grupo lo verifica. Para deshacerlo, la nueva propietaria tiene que devolvértela.</p>` }))) return;
  try { await invoke('group_transfer', { id, nodeId: el.dataset.node }); toast('Propiedad transferida'); } catch (e) { fail(e); }
  refreshServer();
};
A['group-leave'] = async () => {
  const id = state.server.config.id;
  if (!(await dialog({ title: 'Salir del grupo', confirm: 'Salir', danger: true, body: '<p>Esta PC deja de participar en este servidor: no lo vas a poder alojar ni ver su estado. Para volver necesitás una invitación nueva.</p>' }))) return;
  try { const ok = await invoke('group_leave', { id }); toast(ok ? 'Saliste del grupo' : 'Saliste. La propietaria está desconectada: te quita de la lista cuando vuelva.'); } catch (e) { fail(e); }
  refreshServer();
};
A['member-remove'] = async (el) => {
  if (!(await dialog({ title: 'Quitar del grupo', body: '<p>Pierde el acceso a este servidor. Para volver necesita una invitación nueva.</p>', confirm: 'Quitar' }))) return;
  try { await invoke('member_remove', { id: state.params.id, nodeId: el.dataset.node }); } catch (e) { fail(e); }
  drawTab();
};
A.handover = async () => {
  const g = state.share?.group; if (!g) return;
  const eligible = g.members.filter((m) => !m.isMe && m.online && m.canHost);
  let target = eligible[0]?.nodeId || null;
  const ok = await dialog({ title: 'Pasar el servidor a otra PC', body: `<p class="sm">Se guarda y apaga acá, la otra PC copia lo que le falta y lo enciende. Los jugadores se desconectan unos segundos y vuelven a entrar con la misma dirección.</p>
    <label class="field">PC de destino<select class="input" id="ho-target">${eligible.map((m) => `<option value="${esc(m.nodeId)}">${esc(m.name)} · estado ${esc(ago(m.latestAt))}${m.path ? ` · ${m.path === 'relay' ? 'por relay' : 'directa'}` : ''}</option>`).join('')}</select></label>`, confirm: 'Pasar ahora', onMount: (d) => d.querySelector('#ho-target').addEventListener('change', (e) => { target = e.target.value; }) });
  if (!ok) return;
  toast('Pasando el servidor…');
  try { const to = await invoke('group_handover', { id: state.params.id, target }); toast(`Servidor pasado a ${to}`); } catch (e) { fail(e); }
  refreshServer(); drawTab();
};
A.release = async () => {
  if (!(await dialog({ title: 'Guardar y soltar', body: '<p>Se apaga acá, se toma un punto final y se espera a que otra PC del grupo lo copie. Después cualquiera puede encenderlo con ese estado.</p>', confirm: 'Guardar y soltar' }))) return;
  try { const n = await invoke('group_release', { id: state.params.id }); toast(n > 0 ? `Soltado: ${n} PC tiene la última versión` : 'Soltado. Ninguna otra PC estaba conectada: la última versión quedó solo en esta PC', n > 0 ? 'ok' : 'error'); } catch (e) { fail(e); }
  refreshServer(); drawTab();
};
A['take-here'] = async () => {
  if (!(await dialog({ title: 'Recuperar el servidor en esta PC', body: '<p>Usalo solo si sabés que la PC anfitriona se apagó o se quedó sin internet. Se enciende acá con el último estado que copió esta PC: lo que se jugó después en la otra PC no está incluido. Si la otra PC vuelve, se detiene sola y aparta su copia (no se mezclan).</p>', confirm: 'Recuperar aquí' }))) return;
  toast('Recuperando…');
  try { await invoke('group_take_here', { id: state.params.id }); toast('El servidor está en línea en esta PC'); } catch (e) { fail(e); }
  refreshServer(); drawTab();
};
A['group-sync'] = async () => { try { await invoke('group_sync', { id: state.params.id }); toast('Sincronizado'); } catch (e) { fail(e); } drawTab(); };

/* ───────── Ajustes del servidor (copia de trabajo + barra de guardado) ───────── */
const srvEdit = { cfg: null, extra: '', extraOrig: '' };
const isDirty = () => srvEdit.cfg && (JSON.stringify(srvEdit.cfg) !== JSON.stringify(state.server.config) || srvEdit.extra !== srvEdit.extraOrig);
tabDraw.settings = async (el) => {
  const c = state.server.config;
  if (!srvEdit.cfg || srvEdit.cfg.id !== c.id) {
    srvEdit.cfg = clone(c);
    const extra = await invoke('server_properties_extra', { id: c.id }).catch(() => ({}));
    srvEdit.extra = srvEdit.extraOrig = Object.entries(extra).map(([k, v]) => `${k}=${v}`).join('\n');
  }
  const e = srvEdit.cfg;
  const opt = (map, cur) => Object.entries(map).map(([k, v]) => `<option value="${k}" ${String(k) === String(cur) ? 'selected' : ''}>${v}</option>`).join('');
  const hw = state.hw || (state.hw = await invoke('hardware_snapshot').catch(() => null));
  const maxMem = hw ? hw.assessment.maxServerRamMb : 16384;
  const sec = (id, title, body, badge = '') => `<section class="section" id="ss-${id}"><div class="spread"><h2>${title}</h2>${badge}</div><div class="card pad stack" style="gap:16px">${body}</div></section>`;
  el.innerHTML = `<div class="settings">
    <nav class="toc" aria-label="Secciones">${[['general', 'General'], ['perf', 'Memoria y Java'], ['world', 'Juego'], ['dist', 'Distancias'], ['net', 'Jugadores y red'], ['save', 'Copias'], ['expert', 'Avanzado'], ['danger', 'Zona de peligro']].map(([k, v]) => `<button data-a="ss-toc" data-v="${k}">${v}</button>`).join('')}</nav>
    <div class="stack" style="gap:24px">
      ${sec('general', 'General', `<div class="form">
        <label class="field">Nombre<input class="input" data-bind="name" maxlength="40" value="${esc(e.name)}"></label>
        <div class="field">Versión de Minecraft<div class="verrow"><span class="input ro">${loaderName(e.loader)} ${esc(e.mcVersion)}</span><button class="btn" data-a="ver-change" data-tone="cycle">${icon('sync', 15)}Cambiar versión</button></div><span class="hint">939 Host revisa qué implica el cambio para este servidor y te ofrece actualizarlo, hacer una copia o empezar de cero.</span></div>
        <div class="field">Logo<div class="verrow">${srvLogo(state.server.logoUrl, 40)}<span class="grow sm t2">${e.logo ? (e.logo === 'custom' ? 'Imagen propia' : 'Diseño de 939 Host') : 'Ícono genérico'}</span><button class="btn" data-a="logo-edit">${icon('image', 15)}Cambiar logo</button></div><span class="hint">Se ve en tus listas, en la Papelera y en la lista de servidores de Minecraft.</span></div>
        <label class="field full">Descripción<input class="input" data-bind="description" maxlength="120" value="${esc(e.description)}"></label>
        <label class="field full">Mensaje del servidor (MOTD)<input class="input" data-bind="motd" maxlength="120" value="${esc(e.motd)}"></label></div>`)}
      ${sec('perf', 'Memoria y Java', `<div class="field"><div class="range-head"><span>Memoria (RAM)</span><span class="mono" id="mem-label">${fmtMB(e.memoryMb)}</span></div>
          <input type="range" min="1024" max="${Math.max(maxMem, e.memoryMb)}" step="512" value="${e.memoryMb}" data-bind="memoryMb" data-type="int" data-label="mem-label">
          ${hw ? `<span class="hint">Recomendado para esta PC: ${fmtMB(hw.assessment.recommendedServerRamMb)} · máximo seguro ${fmtMB(maxMem)}.</span>` : ''}</div>
        <label class="field">Versión de Java<select class="input" data-bind="javaChoice" data-type="java">${[[null, `Automática — Java ${c.javaRequired === 16 ? 17 : (c.javaRequired || c.javaMajor || '?')} (recomendada)`], ...[8, 11, 17, 21, 25].filter((j) => j >= Math.max(8, c.javaRequired || 0) && j !== (c.javaRequired === 16 ? 17 : c.javaRequired)).map((j) => [j, `Java ${j}`])].map(([k, l]) => `<option value="${k ?? ''}" ${(e.javaChoice ?? null) === k ? 'selected' : ''}>${l}</option>`).join('')}</select>
          <span class="hint">939 Host descarga cada Java desde Eclipse Temurin y lo verifica. Cada servidor puede usar uno distinto; solo se ofrecen versiones compatibles con Minecraft ${esc(c.mcVersion)}.${c.javaMajor ? ` Ahora usa Java ${c.javaMajor}.` : ''}</span></label>
        <label class="field">Perfil de Java<select class="input" data-bind="jvmProfile">${opt({ balanced: 'Equilibrado (G1) — recomendado', lowlatency: 'Baja latencia (ZGC) — para mucha memoria', custom: 'Personalizado — solo mis argumentos' }, e.jvmProfile)}</select></label>
        <label class="field">Argumentos extra de Java<input class="input mono" data-bind="extraJvmArgs" value="${esc(e.extraJvmArgs)}" placeholder="-Dejemplo=1"><span class="hint">Se agregan al final. Con el perfil Personalizado son los únicos (además de la memoria).</span></label>
        <div class="field">Línea de Java resultante<div class="addr"><span id="jvm-preview" class="xs">…</span></div></div>`, '<span class="chip">Se aplica al reiniciar</span>')}
      ${sec('world', 'Juego', `<div class="form">
        <label class="field">Dificultad<select class="input" data-bind="difficulty">${opt(DIFF, e.difficulty)}</select></label>
        <label class="field">Modo de juego<select class="input" data-bind="gamemode">${opt(MODE, e.gamemode)}</select></label>
        <label class="field">Tipo de mundo<select class="input" data-bind="levelType">${opt({ 'minecraft:normal': 'Normal', 'minecraft:flat': 'Plano', 'minecraft:large_biomes': 'Biomas grandes', 'minecraft:amplified': 'Amplificado' }, e.levelType)}</select><span class="hint">Solo afecta a mundos nuevos.</span></label>
        <label class="field">Semilla<input class="input mono" data-bind="levelSeed" value="${esc(e.levelSeed)}"><span class="hint">Solo afecta a mundos nuevos.</span></label>
        <label class="field">Protección del spawn (bloques)<input class="input mono" type="number" min="0" max="1000" data-bind="spawnProtection" data-type="int" value="${e.spawnProtection}"></label>
        <div></div>
        <label class="check"><input type="checkbox" data-bind="pvp" ${e.pvp ? 'checked' : ''}><span>Combate entre jugadores (PvP)</span></label>
        <label class="check"><input type="checkbox" data-bind="hardcore" ${e.hardcore ? 'checked' : ''}><span>Hardcore<span class="hint">Una sola vida.</span></span></label>
        <label class="check"><input type="checkbox" data-bind="allowFlight" ${e.allowFlight ? 'checked' : ''}><span>Permitir vuelo<span class="hint">Evita expulsiones falsas con mods de movimiento.</span></span></label>
        <label class="check"><input type="checkbox" data-bind="enableCommandBlock" ${e.enableCommandBlock ? 'checked' : ''}><span>Bloques de comandos</span></label></div>`)}
      ${sec('dist', 'Distancias', `<div class="form">
        <div class="field"><div class="range-head"><span>Visión</span><span class="mono" id="vd-label">${e.viewDistance} chunks</span></div><input type="range" min="2" max="32" value="${e.viewDistance}" data-bind="viewDistance" data-type="int" data-label="vd-label" data-suffix=" chunks"></div>
        <div class="field"><div class="range-head"><span>Simulación</span><span class="mono" id="sd-label">${e.simulationDistance} chunks</span></div><input type="range" min="2" max="32" value="${e.simulationDistance}" data-bind="simulationDistance" data-type="int" data-label="sd-label" data-suffix=" chunks"></div></div>
        <p class="sm t2" style="margin:0">Se ve lejos, pero solo se simula cerca: es lo que más ayuda con muchos mobs.</p>`)}
      ${sec('net', 'Jugadores y red', `<div class="form">
        <label class="field">Puerto<input class="input mono" type="number" min="1024" max="65535" data-bind="port" data-type="int" value="${e.port}"></label>
        <label class="field">Jugadores máximos<input class="input mono" type="number" min="1" max="500" data-bind="maxPlayers" data-type="int" value="${e.maxPlayers}"></label>
        <label class="check"><input type="checkbox" data-bind="onlineMode" ${e.onlineMode ? 'checked' : ''}><span>Verificar cuentas con Mojang<span class="hint">Recomendado.</span></span></label>
        <label class="check"><input type="checkbox" data-bind="whitelist" ${e.whitelist ? 'checked' : ''}><span>Lista blanca<span class="hint">Agregá jugadores desde la pestaña Jugadores.</span></span></label></div>`)}
      ${sec('save', 'Copias y sincronización', `<div class="form">
        <label class="field">Puntos de restauración<select class="input" data-bind="restoreIntervalMin" data-type="int">${[15, 30, 60, 120].map((m) => `<option value="${m}" ${m === e.restoreIntervalMin ? 'selected' : ''}>Cada ${m} minutos</option>`).join('')}</select><span class="hint">Momentos a los que se puede volver.</span></label>
        <label class="field">Sincronización entre PCs<select class="input" data-bind="syncIntervalMin" data-type="int">${[5, 10, 15, 30, 60].map((m) => `<option value="${m}" ${m === e.syncIntervalMin ? 'selected' : ''}>Cada ${m} minutos</option>`).join('')}</select><span class="hint">Lo máximo que se pierde si el anfitrión se apaga de golpe.</span></label></div>`)}
      ${sec('expert', 'Avanzado', `<label class="field">Otras claves de server.properties<textarea class="input" id="extra-props" spellcheck="false">${esc(srvEdit.extra)}</textarea><span class="hint">Una por línea: clave=valor. Las claves que manejan las secciones de arriba se ignoran acá.</span></label>
        <div class="spread"><div><b>Reinstalar archivos del servidor</b><div class="sm t2">Vuelve a descargar y verificar el servidor y Java. No toca el mundo.</div></div><button class="btn" data-a="reinstall">Reinstalar</button></div>`)}
      <section class="section" id="ss-danger"><h2>Zona de peligro</h2><div class="card list">
        <div class="li"><div class="grow"><b>Informe de diagnóstico</b><div class="sm t2">Un .zip con registros, errores, mods y configuración (sin el mundo ni claves) para pedir ayuda.</div></div><button class="btn s" data-a="diag">Generar informe</button></div>
        <div class="li"><div class="grow"><b>Borrar servidor</b><div class="sm t2">Se mueve a la Papelera de 939 Host; no se elimina del disco.</div></div><button class="btn s" data-a="delete" data-tone="danger">${icon('trash', 14)}Borrar…</button></div></div></section>
      <div class="savebar" id="savebar"></div>
    </div></div>`;
  updateSavebar();
  updateJvmPreview();
  onLeave(() => { srvEdit.cfg = null; });
  scrollSpy('.settings .toc', 'ss');
  guards.push(async () => !isDirty() || (await dialog({ title: 'Hay cambios sin guardar', body: '<p>Si salís de Ajustes se descartan.</p>', confirm: 'Descartar cambios' }) && (srvEdit.cfg = null, true)));
};
function updateSavebar() {
  const bar = $('#savebar');
  if (!bar) return;
  const d = isDirty();
  bar.classList.toggle('dirty', d);
  bar.innerHTML = d ? `<b>Cambios sin guardar</b><div class="btns"><button class="btn" data-a="ss-discard">Descartar</button><button class="btn pri" data-a="ss-save">Guardar cambios</button></div>`
    : `<span class="sm t3">${icon('check', 14)}</span><span class="sm t3 grow">Todo guardado.</span>`;
}
const updateJvmPreview = debounce(async () => {
  const el = $('#jvm-preview');
  if (!el || !srvEdit.cfg) return;
  try { el.textContent = `java ${await invoke('jvm_preview', { config: srvEdit.cfg })} -jar … nogui`; } catch { /* nada */ }
}, 150);
A['ss-toc'] = (el) => { el.parentElement.querySelectorAll('button').forEach((b) => b.classList.toggle('on', b === el)); document.getElementById(`ss-${el.dataset.v}`)?.scrollIntoView({ behavior: scrollBehavior(), block: 'start' }); };
A['ss-discard'] = () => { srvEdit.cfg = null; drawTab(); };
A['ss-save'] = async () => {
  const e = clone(srvEdit.cfg);
  const extra = {};
  for (const line of srvEdit.extra.split('\n')) {
    const t = line.trim();
    if (!t || t.startsWith('#')) continue;
    const i = t.indexOf('=');
    if (i <= 0) { toast(`Línea inválida en server.properties: «${t}»`, 'error'); return; }
    extra[t.slice(0, i).trim()] = t.slice(i + 1).trim();
  }
  e.extraProperties = extra;
  try {
    const saved = await invoke('update_server', { config: e });
    state.server.config = saved; srvEdit.cfg = clone(saved); srvEdit.extraOrig = srvEdit.extra;
    toast(isActive(state.server) ? 'Guardado. Se aplica al reiniciar el servidor.' : 'Cambios guardados');
    state.server = await invoke('get_server', { id: saved.id });
    drawHeader(); updateSavebar();
  } catch (err) { fail(err); }
};
main.addEventListener('input', (ev) => {
  const el = ev.target;
  if (el.id === 'extra-props') { srvEdit.extra = el.value; updateSavebar(); return; }
  if (!el.dataset.bind || !srvEdit.cfg) return;
  const val = el.type === 'checkbox' ? el.checked : el.dataset.type === 'int' ? parseInt(el.value, 10) || 0 : el.dataset.type === 'java' ? (el.value ? parseInt(el.value, 10) : null) : el.value;
  srvEdit.cfg[el.dataset.bind] = val;
  if (el.dataset.label) document.getElementById(el.dataset.label).textContent = el.dataset.bind === 'memoryMb' ? fmtMB(val) : `${val}${el.dataset.suffix || ''}`;
  updateSavebar(); updateJvmPreview();
});
A.diag = async (el) => {
  el.disabled = true;
  try { await invoke('diagnostics_report', { id: state.params.id }); toast('Informe guardado en la carpeta de registros'); await invoke('open_folder', { id: null, which: 'applogs' }); } catch (e) { fail(e); }
  el.disabled = false;
};
A.delete = async () => {
  const name = state.server.config.name;
  if (isActive(state.server)) { toast('Detené el servidor antes de borrarlo.', 'error'); return; }
  if (!(await dialog({ title: `Borrar «${name}»`, danger: true, body: '<p>El servidor, su mundo, sus mods y sus copias van a la <b>Papelera</b>. Se conservan 30 días y podés restaurarlos completos desde <b>Papelera</b> en la barra lateral.</p>', confirm: 'Mover a la Papelera', requireText: name }))) return;
  try { await invoke('delete_server', { id: state.params.id, confirmName: name }); toast('Movido a la Papelera · se conserva 30 días'); srvEdit.cfg = null; guards.length = 0; refreshTrashCount(); go('servers'); } catch (e) { fail(e); }
};
