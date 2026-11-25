'use strict';
/* Servidores (inicio), primera ejecución, asistente de creación y servidores de amigos. */

/* ───────── Servidores ───────── */
routes.servers = async function renderServers() {
  const [servers, hosting] = await Promise.all([invoke('list_servers'), invoke('servers_hosting').catch(() => [])]);
  state.servers = servers;
  state.hosting = Object.fromEntries(hosting.map((h) => [h.id, h]));
  if (!servers.length) return paintWelcome();
  const hw = state.hw || (state.hw = await invoke('hardware_snapshot').catch(() => null));
  const online = servers.filter((s) => s.status === 'running').length;
  const players = servers.reduce((n, s) => n + (s.status === 'running' ? s.players.length : 0), 0);
  main.innerHTML = `<div class="page">
    <div class="head"><div><h1>Servidores</h1><p class="sub">${online ? `${online} funcionando · ${players} ${players === 1 ? 'jugador conectado' : 'jugadores conectados'}` : 'Ninguno está funcionando ahora.'}</p></div>
      <div class="btns"><button class="btn" data-a="nav" data-route="friends">${icon('link')}Unirme con un código</button><button class="btn" data-a="nav" data-route="modpack">${icon('layers')}Desde un modpack</button><button class="btn pri" data-a="new">${icon('plus')}Crear servidor</button></div></div>
    <div class="card list srv-list">${servers.map(serverRow).join('')}</div>
    <div id="linked-box"></div>
    <div class="grid2">
      <div class="card pad stack"><div class="spread"><h2>Esta PC</h2><button class="btn s ghost" data-a="settings-section" data-sec="pc">Ver detalle${icon('chev', 14)}</button></div>
        ${hw ? `<div><div class="kv"><span class="k">Evaluación</span><span class="b">${esc(hw.assessment.rating)}</span></div>
        <div class="kv"><span class="k">Procesador</span><span class="ellipsis">${esc(hw.cpuModel)}</span></div>
        <div class="kv"><span class="k">RAM recomendada por servidor</span><span class="mono">${fmtMB(hw.assessment.recommendedServerRamMb)}</span></div></div>` : '<p class="sm t2" style="margin:0">No se pudo leer el hardware.</p>'}</div>
      <div class="card pad stack"><div class="spread"><h2>Jugar con amigos</h2><button class="btn s ghost" data-a="nav" data-route="friends">Servidores de amigos${icon('chev', 14)}</button></div>
        <p class="sm t2" style="margin:0">Abrí un servidor → <b>Grupo</b> → <b>Invitar</b>. Tus amigos pegan el código en 939 Host y entran siempre por la misma dirección, sin abrir puertos. Si lo invitás como coanfitrión, guarda una copia y puede hostearlo cuando tu PC no esté.</p></div>
    </div></div>`;
  paintLinked();
  // Una sola actualización periódica (antes cada actualización agregaba otro temporizador).
  if (servers.some((s) => isActive(s) || s.installing) && !state.timers.includes(serversTimer)) serversTimer = every(10000, () => state.route === 'servers' && routes.servers());
};

/* Servidores encendidos fuera de 939 Host (otra carpeta, otro programa u otra PC de la red),
   vinculados por dirección y puerto. Se muestra lo que responde de verdad: un puerto abierto que
   no contesta como Minecraft no cuenta como encendido. */
const LINK_STATE = { minecraft: ['dot', 'Responde'], abierto: ['tri', 'No responde como Minecraft'], cerrado: ['off', 'No responde'], 'sin-resolver': ['tri', 'Dirección desconocida'] };
const LINK_SCOPE = { 'esta-pc': 'En esta PC', 'red-local': 'En tu red local', internet: 'Dirección de Internet' };
async function paintLinked() {
  const box = document.getElementById('linked-box'); if (!box) return;
  const list = await invoke('linked_list').catch(() => []);
  if (!document.getElementById('linked-box')) return;
  box.innerHTML = `<div class="section" style="margin-top:20px"><div class="spread"><div><h2>Iniciados fuera de 939 Host</h2><p class="sm t2" style="margin:2px 0 0">Servidores que encendés con otro programa o en otro puerto. 939 Host muestra si responden, su versión y jugadores; para administrarlos (consola, jugadores, copias) importalos como servidor de 939 Host.</p></div><button class="btn s" data-a="linked-add">${icon('plus', 14)}Vincular</button></div>
    ${list.length ? `<div class="card list" style="margin-top:12px">${list.map((l) => {
      const [shape, label] = LINK_STATE[l.state] || ['tri', l.state];
      return `<div class="li"><div class="grow" style="min-width:0"><div class="row wrap" style="gap:6px 8px"><b>${esc(l.name)}</b><span class="chip s">${LINK_SCOPE[l.scope] || ''}</span>${l.sameAs ? `<span class="chip s" title="Usa el mismo puerto que tu servidor «${esc(l.sameAs)}»">Puerto de «${esc(l.sameAs)}»</span>` : ''}</div>
        <div class="sm t3"><span class="mono">${esc(l.host)}:${l.port}</span> · ${esc(l.detail)}${l.program ? ` · ${esc(l.program)}` : ''}</div></div>
        <div class="cols"><div class="c" style="min-width:150px"><span class="st">${SHAPES[shape]}${esc(label)}</span>${l.state === 'minecraft' ? `<span class="xs t3">${l.online}/${l.max} jugadores</span>` : ''}</div></div>
        <button class="btn s icon ghost" data-a="copy" data-text="${esc(l.host)}:${l.port}" data-msg="Dirección copiada" aria-label="Copiar dirección">${icon('copy', 14)}</button>
        <button class="btn s icon ghost" data-a="linked-remove" data-id="${esc(l.id)}" data-tone="halt" aria-label="Desvincular" title="Desvincular (no toca el servidor)">${icon('trash', 14)}</button></div>`;
    }).join('')}</div>` : ''}</div>`;
  // Un solo temporizador por pantalla (los temporizadores se limpian al cambiar de pantalla).
  if (list.length && !state.timers.includes(linkedTimer)) linkedTimer = every(15000, () => state.route === 'servers' && paintLinked());
}
let linkedTimer = null, serversTimer = null;
A['linked-add'] = async () => {
  let name = '', host = '127.0.0.1', port = 25565;
  const ok = await dialog({ title: 'Vincular un servidor iniciado fuera de 939 Host', confirm: 'Vincular',
    body: `<label class="field">Nombre<input class="input" id="lk-n" maxlength="48" placeholder="Por ejemplo: Servidor de prueba"></label>
      <div class="row" style="gap:10px"><label class="field grow">Dirección<input class="input mono" id="lk-h" value="127.0.0.1" spellcheck="false"><span class="hint">127.0.0.1 si está en esta PC; su IP si está en otra PC de tu red.</span></label>
      <label class="field" style="width:120px">Puerto<input class="input mono" id="lk-p" type="number" min="1" max="65535" value="25565"></label></div>
      <p class="xs t3">Solo se consulta su estado (como la lista de Multijugador): no se cambia nada en ese servidor.</p>`,
    onMount: (d) => { d.querySelector('#lk-n').addEventListener('input', (e) => { name = e.target.value; }); d.querySelector('#lk-h').addEventListener('input', (e) => { host = e.target.value; }); d.querySelector('#lk-p').addEventListener('input', (e) => { port = +e.target.value; }); d.querySelector('#lk-n').focus(); } });
  if (!ok) return;
  try { await invoke('linked_add', { name, host, port }); toast('Servidor vinculado'); } catch (e) { fail(e); }
  paintLinked();
};
A['linked-remove'] = async (el) => { try { await invoke('linked_remove', { id: el.dataset.id }); } catch (e) { fail(e); } paintLinked(); };
/* Servidor compartido que ahora se juega en otra PC del grupo. */
const elsewhere = (s) => { const h = state.hosting?.[s.config.id]; return h && h.hosting && !h.iAmHost && !isActive(s) ? h : null; };
function serverRow(s) {
  const c = s.config;
  const st = serverState(s);
  const away = elsewhere(s);
  return `<div class="li click" data-a="open" data-id="${esc(c.id)}" role="button" tabindex="0">
    ${srvLogo(s.logoUrl)}
    <div class="grow"><div class="b ellipsis">${esc(c.name)}</div><div class="sm t3 ellipsis">${versionLabel(c)}${c.description ? ` · ${esc(c.description)}` : ''}</div></div>
    <div class="cols">
      <div class="c" style="min-width:132px">${away ? awayState(away) : `${stateHTML(s)}${st.detail ? `<span class="xs t3 ellipsis">${esc(st.detail)}</span>` : ''}`}</div>
      <div class="c"><span class="xs t3">Jugadores</span><span class="mono sm">${away ? (['running', 'ready'].includes(away.state) ? `${away.players}/${c.maxPlayers}` : '—') : s.status === 'running' ? `${s.players.length}/${c.maxPlayers}` : '—'}</span></div>
      <div class="c opt"><span class="xs t3">Memoria</span><span class="mono sm">${s.memoryUsedMb != null ? fmtMB(s.memoryUsedMb) : fmtMB(c.memoryMb)}</span></div>
      <div class="c" style="min-width:96px;align-items:flex-end">${away ? awayAction(s, away) : quickAction(s)}</div>
    </div>${icon('chev', 16)}</div>`;
}
/* Estado real del servidor en la PC que lo aloja (el mismo que ven todas las PCs del grupo). */
function awayState(a) {
  const st = !a.hostOnline ? 'unreachable' : (a.state || 'unknown');
  const label = REMOTE_STATE[st] || 'En otra PC';
  const shape = st === 'unreachable' || st === 'stopped' ? SHAPES.off : ['starting', 'preparing', 'saving', 'restarting', 'transferring', 'recovering'].includes(st) ? SHAPES.ring : st === 'error' ? SHAPES.tri : SHAPES.dot;
  return `<span class="st">${shape}${esc(label)}</span><span class="xs t3 ellipsis" title="${esc(a.detail || '')}">En la PC de ${esc(a.hostName || 'otra PC')}</span>`;
}
/* Lo aloja otra PC del grupo: Iniciar y Detener actúan allá (cualquier integrante). */
function awayAction(s, a) {
  const id = esc(s.config.id);
  if (!a.hostOnline) return pcAvatar(a.hostSymbol, 30);
  if (a.state === 'stopped' || a.state === 'error') return `<button class="btn s" data-a="start" data-tone="go" data-id="${id}" title="Se enciende en la PC de ${esc(a.hostName || 'quien lo aloja')}">${icon('play', 14)}Iniciar</button>`;
  if (['ready', 'running', 'starting', 'preparing'].includes(a.state)) return `<button class="btn s" data-a="remote-stop" data-tone="halt" data-id="${id}">${icon('stop', 14)}Detener</button>`;
  return '<span class="xs t3">Esperá…</span>';
}
function quickAction(s) {
  const id = esc(s.config.id);
  if (!s.config.installed || s.installing) return '';
  if (s.restarting || s.status === 'stopping' || s.status === 'starting') return '<span class="xs t3">Esperá…</span>';
  if (s.outside && s.status !== 'running') return '<span class="xs t3" title="Cerralo desde su ventana (stop) para administrarlo desde 939 Host">Fuera de 939 Host</span>';
  if (s.status === 'running') return `<button class="btn s" data-a="stop" data-tone="halt" data-id="${id}">${icon('stop', 14)}Detener</button>`;
  if (s.status === 'crashed') return `<button class="btn s" data-a="repair" data-id="${id}">${icon('wrench', 14)}Reparar</button>`;
  return `<button class="btn s" data-a="start" data-tone="go" data-id="${id}">${icon('play', 14)}Iniciar</button>`;
}
main.addEventListener('keydown', (e) => { if ((e.key === 'Enter' || e.key === ' ') && e.target.matches('.li.click[data-a]')) { e.preventDefault(); e.target.click(); } });

function paintWelcome() {
  main.innerHTML = `<div class="page">
    <div class="head"><div><h1>Te damos la bienvenida a 939 Host</h1><p class="sub">Advanced hosting. Simplified. Elegí cómo empezar.</p></div></div>
    <div class="welcome">
      <button class="card main-choice" data-a="new"><span class="wi">${icon('plus', 22)}</span><h2>Crear un servidor</h2><span class="sm t2">Elegís la versión y la memoria; 939 Host descarga, verifica y prepara todo.</span></button>
      <button class="card" data-a="nav" data-route="friends"><span class="wi">${icon('link', 22)}</span><h2>Unirme con un código</h2><span class="sm t2">Si un amigo te invitó, pegá su código y entrá a su servidor.</span></button>
      <button class="card" data-a="bench-welcome"><span class="wi">${icon('gauge', 22)}</span><h2>Evaluar esta PC</h2><span class="sm t2">Una prueba de unos segundos para saber cuánta memoria conviene darle a cada servidor.</span></button>
    </div>
    <div class="note">${icon('info')}<div class="grow"><b>Nada viene preconfigurado</b><span class="sm t2">Cada servidor que crees tiene su propia carpeta, mundo y configuración. 939 Host no toca tu instalación de Minecraft.</span></div></div>
  </div>`;
}
A['bench-welcome'] = async (el) => {
  el.disabled = true;
  el.querySelector('h2').textContent = 'Evaluando…';
  try { await invoke('run_benchmark'); state.hw = null; toast('Evaluación completada'); } catch (e) { fail(e); }
  goSettings('pc');
};
A.open = (el) => { state.tab = 'summary'; go('server', { id: el.dataset.id }); };
A.new = () => { state.wizard = null; go('create'); };
A.start = async (el) => {
  const id = el.dataset.id || state.params.id;
  el.disabled = true;
  markPending(id, 'start');
  repaintState(id);
  try { await invoke('start_server', { id }); } catch (e) { clearPending(id); repaintState(id); fail(e); el.disabled = false; }
};
/* Vuelve a dibujar el estado de un servidor donde se esté viendo (lista o pantalla del servidor). */
function repaintState(id) {
  if (state.route === 'server' && state.params.id === id && state.server) drawHeader();
  else if (state.route === 'servers') routes.servers();
}
A.stop = (el) => stopFlow(el.dataset.id, 'stop');

/* ───────── Crear servidor ───────── */
const WSTEPS = ['Versión y tipo', 'Memoria y jugadores', 'Mundo', 'Revisar'];
routes.create = async function renderCreate() {
  if (!state.wizard) {
    const hw = state.hw || (state.hw = await invoke('hardware_snapshot'));
    state.wizard = {
      step: 1, name: '', description: '', mcVersion: '', loader: 'vanilla', support: null,
      worldMode: 'new', seed: '', difficulty: 'normal', gamemode: 'survival', importPath: '', importOk: false,
      memoryMb: hw.assessment.recommendedServerRamMb, maxMem: hw.assessment.maxServerRamMb, maxPlayers: 8,
      viewDistance: 10, simulationDistance: 8, port: await invoke('suggest_port'), onlineMode: true, whitelist: false,
      eula: false, creating: false, snapshots: false,
    };
    // «Empezar de cero» desde el cambio de versión: llega con la versión y el tipo elegidos.
    if (state.prefill) { Object.assign(state.wizard, state.prefill); state.prefill = null; }
  }
  if (!state.versions) state.versions = await call('list_versions', { includeSnapshots: false });
  const w = state.wizard;
  if (!w.mcVersion && state.versions.length) w.mcVersion = state.versions[0].id;
  drawCreate();
  guards.push(async () => !state.wizard || state.wizard.creating || (!state.wizard.name && state.wizard.step === 1) || dialog({ title: 'Salir sin crear', body: '<p>Se pierde lo que elegiste en el asistente.</p>', confirm: 'Salir' }));
};
function drawCreate() {
  const w = state.wizard, hw = state.hw;
  const versions = (w.snapshots ? state.snapshots : state.versions) || [];
  const opt = (map, cur) => Object.entries(map).map(([k, v]) => `<option value="${k}" ${k === cur ? 'selected' : ''}>${v}</option>`).join('');
  let body = '';
  if (w.step === 1) body = `
    <div class="form">
      <label class="field full">Nombre del servidor<input class="input" data-w="name" maxlength="40" value="${esc(w.name)}" placeholder="Por ejemplo: Supervivencia con amigos" autofocus></label>
      <label class="field">Versión de Minecraft<select class="input" data-w="mcVersion">${versions.map((v) => `<option value="${esc(v.id)}" ${v.id === w.mcVersion ? 'selected' : ''}>${esc(v.id)}${v.kind === 'snapshot' ? ' (snapshot)' : ''}</option>`).join('')}</select></label>
      <div class="field"><span>&nbsp;</span><label class="check" style="height:40px;align-items:center"><input type="checkbox" data-w="snapshots" ${w.snapshots ? 'checked' : ''}><span>Mostrar snapshots</span></label></div>
      <label class="field full">Descripción <span class="hint">Opcional, solo para vos</span><input class="input" data-w="description" maxlength="120" value="${esc(w.description)}"></label>
    </div>
    <div class="stack"><h3>Tipo de servidor</h3><div class="choices">
      ${[['vanilla', 'Vanilla', 'Minecraft oficial, sin mods.'], ['fabric', 'Fabric', 'Liviano y rápido. El más usado para mods modernos.'], ['neoforge', 'NeoForge', 'Para modpacks grandes basados en NeoForge.'], ['forge', 'Forge', 'Para mods y modpacks clásicos de Forge.']]
        .map(([k, t, d]) => `<button class="choice ${w.loader === k ? 'on' : ''}" data-a="loader" data-v="${k}" aria-pressed="${w.loader === k}"><span class="t">${t}</span><span class="d">${k === 'vanilla' ? d : loaderText(k, d)}</span></button>`).join('')}
    </div></div>
    ${wizardOptHTML(w)}`;
  if (w.step === 2) body = `
    <div class="field"><div class="range-head"><span>Memoria (RAM)</span><span class="mono">${fmtMB(w.memoryMb)}</span></div>
      <input type="range" min="1024" max="${w.maxMem}" step="512" value="${w.memoryMb}" data-w="memoryMb" data-type="int" aria-label="Memoria">
      <div class="reco">${icon('gauge', 16)}<span>Recomendado para esta PC: <b>${fmtMB(hw.assessment.recommendedServerRamMb)}</b> · máximo seguro ${fmtMB(hw.assessment.maxServerRamMb)}. Más memoria no es más rendimiento: alcanza con que no falte.</span>${w.memoryMb !== hw.assessment.recommendedServerRamMb ? `<button class="btn s ghost" data-a="wreco">Usar recomendado</button>` : ''}</div></div>
    <div class="form">
      <label class="field">Jugadores máximos<input class="input mono" type="number" min="1" max="500" data-w="maxPlayers" data-type="int" value="${w.maxPlayers}"></label>
      <label class="field">Puerto<input class="input mono" type="number" min="1024" max="65535" data-w="port" data-type="int" value="${w.port}"><span class="hint">Libre en esta PC. Tus amigos no lo necesitan si entran con 939 Host.</span></label>
      <div class="field"><div class="range-head"><span>Distancia de visión</span><span class="mono">${w.viewDistance} chunks</span></div><input type="range" min="2" max="32" value="${w.viewDistance}" data-w="viewDistance" data-type="int"></div>
      <div class="field"><div class="range-head"><span>Distancia de simulación</span><span class="mono">${w.simulationDistance} chunks</span></div><input type="range" min="2" max="32" value="${w.simulationDistance}" data-w="simulationDistance" data-type="int"><span class="hint">Simular menos de lo que se ve es lo que más ayuda con muchos mobs.</span></div>
      <label class="check full"><input type="checkbox" data-w="onlineMode" ${w.onlineMode ? 'checked' : ''}><span>Verificar cuentas con Mojang<span class="hint">Recomendado. Solo entran cuentas oficiales de Minecraft.</span></span></label>
      <label class="check full"><input type="checkbox" data-w="whitelist" ${w.whitelist ? 'checked' : ''}><span>Lista blanca<span class="hint">Solo entran los jugadores que agregues.</span></span></label>
    </div>`;
  if (w.step === 3) body = `
    <div class="seg" role="group" aria-label="Origen del mundo"><button class="${w.worldMode === 'new' ? 'on' : ''}" data-a="worldmode" data-v="new">Mundo nuevo</button><button class="${w.worldMode === 'import' ? 'on' : ''}" data-a="worldmode" data-v="import">Usar una copia de un mundo</button></div>
    ${w.worldMode === 'import' ? `<div class="note">${icon('shield')}<div class="grow"><b>Se copia; el original nunca se modifica</b><span class="sm t2">Elegí la carpeta del mundo (la que tiene level.dat). Si está abierto en Minecraft, salí al menú antes de copiarlo.</span></div></div>
      <div class="row"><button class="btn" data-a="pick-world">${icon('folder')}Elegir carpeta…</button><span class="sm mono ellipsis ${w.importOk ? '' : 't3'}">${esc(w.importPath || 'Ninguna carpeta elegida')}</span></div>` : ''}
    <div class="form">
      <label class="field">Dificultad<select class="input" data-w="difficulty">${opt(DIFF, w.difficulty)}</select></label>
      <label class="field">Modo de juego<select class="input" data-w="gamemode">${opt(MODE, w.gamemode)}</select></label>
      ${w.worldMode === 'new' ? `<label class="field full">Semilla <span class="hint">Vacío = aleatoria</span><input class="input mono" data-w="seed" value="${esc(w.seed)}" maxlength="64"></label>` : ''}
    </div>`;
  if (w.step === 4) body = `
    <div class="card pad"><div>${[
      ['Nombre', esc(w.name)], ['Versión', `${loaderName(w.loader)} ${esc(w.mcVersion)}`],
      ['Memoria', fmtMB(w.memoryMb)], ['Jugadores', `${w.maxPlayers} como máximo · ${w.whitelist ? 'con lista blanca' : 'sin lista blanca'}`],
      ['Mundo', w.worldMode === 'import' ? `Copia de <span class="mono sm">${esc(w.importPath)}</span>` : `Nuevo · ${DIFF[w.difficulty]} · ${MODE[w.gamemode]}`],
      ['Distancias', `${w.viewDistance} de visión · ${w.simulationDistance} de simulación`],
      ['Red', `Puerto ${w.port} · ${w.onlineMode ? 'cuentas verificadas' : 'sin verificar cuentas'}`],
      ['Copias', 'Puntos de restauración automáticos cada 15 min'],
      ...(w.optOn && w.loader !== 'vanilla' ? [['Rendimiento', w.optSlugs?.length ? `${w.optSlugs.length} mods de optimización` : 'Ninguno disponible']] : []),
    ].map(([k, v]) => `<div class="kv"><span class="k">${k}</span><span>${v}</span></div>`).join('')}</div></div>
    <label class="check"><input type="checkbox" data-w="eula" ${w.eula ? 'checked' : ''}><span>Acepto el <button class="link" data-a="link" data-url="https://aka.ms/MinecraftEULA">EULA de Minecraft</button><span class="hint">Mojang exige aceptarlo para ejecutar un servidor. 939 Host lo registra con fecha.</span></span></label>`;

  main.innerHTML = `<div class="page">
    <div class="crumbs"><button data-a="nav" data-route="servers">Servidores</button><span>/</span><span>Crear servidor</span></div>
    <div class="head"><h1>Crear servidor</h1></div>
    <div class="steps">${WSTEPS.map((s, i) => `${i ? '<span class="step-sep"></span>' : ''}<span class="step ${w.step === i + 1 ? 'on' : w.step > i + 1 ? 'done' : ''}"><span class="n">${w.step > i + 1 ? icon('check', 12, 2) : i + 1}</span>${s}</span>`).join('')}</div>
    <div class="wizard"><div class="card pad stack" style="gap:20px">${body}
      <div class="spread" style="border-top:1px solid var(--line);padding-top:18px">
        <button class="btn" data-a="${w.step === 1 ? 'nav' : 'wback'}" data-route="servers">${w.step === 1 ? 'Cancelar' : 'Atrás'}</button>
        ${w.step < 4 ? `<button class="btn pri" data-a="wnext" ${stepValid() ? '' : 'disabled'}>Continuar</button>` : `<button class="btn pri" data-a="wcreate" ${w.eula && !w.creating ? '' : 'disabled'}>${w.creating ? 'Creando…' : 'Crear servidor'}</button>`}
      </div></div>
      ${w.step < 4 ? `<aside class="card pad stack"><h3>Resumen</h3><div>
        <div class="kv"><span class="k">Nombre</span><span class="ellipsis">${esc(w.name) || '—'}</span></div>
        <div class="kv"><span class="k">Versión</span><span class="num">${loaderName(w.loader)} ${esc(w.mcVersion)}</span></div>
        <div class="kv"><span class="k">Memoria</span><span class="num">${fmtMB(w.memoryMb)}</span></div>
        <div class="kv"><span class="k">Mundo</span><span>${w.worldMode === 'import' ? 'Copia' : 'Nuevo'}</span></div>
        <div class="kv"><span class="k">Puerto</span><span class="num">${w.port}</span></div></div>` : `<aside class="card pad stack"><h3>Qué pasa al crear</h3><ol class="next-steps">
        <li><b>Descarga verificada</b><span class="sm t2">Servidor oficial de Mojang${w.loader !== 'vanilla' ? `, ${loaderName(w.loader)}` : ''} y el Java que corresponde. Cada archivo se comprueba antes de usarlo.</span></li>
        <li><b>Primer encendido</b><span class="sm t2">Se genera el mundo. Puede tardar un par de minutos la primera vez.</span></li>
        <li><b>Listo para invitar</b><span class="sm t2">Desde la pestaña Grupo compartís el servidor con tus amigos.</span></li></ol>`}</div>
      </aside></div></div>`;
}
function loaderText(loader, idle) {
  const w = state.wizard;
  const f = w.support && w.support.loader === loader ? w.support : null;
  if (!f) return idle;
  if (f.loading) return 'Comprobando compatibilidad…';
  if (f.error) return `No se pudo comprobar ${loaderName(loader)} (sin conexión).`;
  return f.supported ? `Compatible · ${esc(f.loaderVersion)}${f.unverified ? ' · instalación todavía no probada por 939 Host' : ''}` : `Todavía no hay ${loaderName(loader)} para ${esc(w.mcVersion)}.`;
}
function stepValid() {
  const w = state.wizard;
  if (w.step === 1) return w.name.trim().length > 0 && !!w.mcVersion && (w.loader === 'vanilla' || (w.support && w.support.loader === w.loader && w.support.supported));
  if (w.step === 2) return w.port >= 1024 && w.port <= 65535 && w.maxPlayers >= 1 && w.maxPlayers <= 500;
  if (w.step === 3) return w.worldMode === 'new' || w.importOk;
  return true;
}
async function checkLoader() {
  const w = state.wizard;
  const loader = w.loader, mc = w.mcVersion;
  if (loader === 'vanilla') { w.support = null; drawCreate(); return; }
  w.support = { loader, loading: true };
  drawCreate();
  let r;
  try { r = { loader, ...(await invoke('loader_support', { loader, mcVersion: mc })) }; } catch { r = { loader, error: true }; }
  if (w.loader === loader && w.mcVersion === mc) w.support = r;
  if (state.route === 'create') drawCreate();
}
async function toggleSnapshots(on) {
  if (on && !state.snapshots) { try { state.snapshots = await invoke('list_versions', { includeSnapshots: true }); } catch (e) { fail(e); state.wizard.snapshots = false; } }
  const list = (state.wizard.snapshots ? state.snapshots : state.versions) || [];
  if (!list.some((v) => v.id === state.wizard.mcVersion)) state.wizard.mcVersion = list[0]?.id || '';
  checkLoader();
}
A.loader = (el) => { const w = state.wizard; if (w.loader !== el.dataset.v) { w.optSlugs = null; w.optOn = false; } w.loader = el.dataset.v; checkLoader(); };
A.worldmode = (el) => { state.wizard.worldMode = el.dataset.v; drawCreate(); };
A.wnext = () => { if (stepValid()) { state.wizard.step++; drawCreate(); main.scrollTop = 0; } };
A.wback = () => { state.wizard.step--; drawCreate(); };
A.wreco = () => { state.wizard.memoryMb = state.hw.assessment.recommendedServerRamMb; drawCreate(); };
A['pick-world'] = async () => {
  const path = await TAURI.dialog.open({ directory: true, title: 'Elegí la carpeta del mundo (la que tiene level.dat)' });
  if (!path) return;
  try { await invoke('validate_world', { path }); state.wizard.importPath = path; state.wizard.importOk = true; } catch (e) { state.wizard.importOk = false; state.wizard.importPath = ''; fail(e); }
  drawCreate();
};
A.wcreate = async () => {
  const w = state.wizard;
  w.creating = true; drawCreate();
  try {
    const cfg = await invoke('create_server', { input: {
      name: w.name.trim(), description: w.description.trim(), mcVersion: w.mcVersion, loader: w.loader, memoryMb: w.memoryMb,
      difficulty: w.difficulty, gamemode: w.gamemode, maxPlayers: w.maxPlayers, port: w.port, onlineMode: w.onlineMode,
      whitelist: w.whitelist, viewDistance: w.viewDistance, simulationDistance: w.simulationDistance, levelSeed: w.worldMode === 'new' ? w.seed : '',
      acceptEula: w.eula, importWorldPath: w.worldMode === 'import' ? w.importPath : null,
      optimize: w.optOn && w.loader !== 'vanilla' ? (w.optSlugs || []) : [],
    } });
    state.wizard = null; state.tab = 'summary';
    toast('Servidor creado. Descargando lo necesario…');
    go('server', { id: cfg.id });
  } catch (e) { w.creating = false; fail(e); drawCreate(); }
};
main.addEventListener('input', (e) => {
  const el = e.target;
  if (!el.dataset.w || !state.wizard || state.route !== 'create') return;
  const val = el.type === 'checkbox' ? el.checked : el.dataset.type === 'int' ? parseInt(el.value, 10) || 0 : el.value;
  const key = el.dataset.w;
  state.wizard[key] = val;
  if (key === 'snapshots') { toggleSnapshots(val); return; }
  if (key === 'mcVersion') { state.wizard.optSlugs = null; state.wizard.optOn = false; checkLoader(); return; }
  if (el.type === 'range' || el.type === 'checkbox' || el.tagName === 'SELECT') { const y = main.scrollTop; drawCreate(); main.scrollTop = y; return; }
  const btn = main.querySelector('[data-a="wnext"]');
  if (btn) btn.disabled = !stepValid();
});

/* ───────── Servidores de amigos ───────── */
routes.friends = async function renderFriends() {
  const remotes = await invoke('remote_list');
  main.innerHTML = `<div class="page">
    <div class="head"><div><h1>Servidores de amigos</h1><p class="sub">Servidores de otras PCs a los que te uniste. Esta PC: <span class="mono">${esc(state.info.node.shortId)}</span></p></div></div>
    <div class="card pad stack"><h2>Unirme con un código</h2>
      <p class="sm t2" style="margin:0">Pegá el código que te mandaron (empieza con <span class="mono">939-</span>). Cada código sirve para una sola PC y la PC de tu amigo tiene que tener 939 Host abierto.</p>
      <div class="row"><input class="input mono" id="join-code" placeholder="939-…" autocomplete="off" spellcheck="false" aria-label="Código de invitación"><button class="btn pri" data-a="join">${icon('link')}Unirme</button></div></div>
    ${remotes.length ? `<div class="grid2">${remotes.map((r) => `<div class="card pad stack">
        <div class="spread"><div class="row">${srvLogo(r.logo)}<div><div class="b">${esc(r.name || 'Servidor')}</div><div class="sm t3">${versionLabel(r)} · ${r.hasCopy ? 'coanfitrión' : 'jugador'}</div></div></div>
          <span class="st">${r.iAmHost ? SHAPES.dot + 'En esta PC' : r.hostOnline ? SHAPES.dot + 'En línea' : SHAPES.off + 'Sin anfitrión'}</span></div>
        <div class="addr"><span>127.0.0.1:${r.routePort}</span><button class="btn s" data-a="copy" data-text="127.0.0.1:${r.routePort}" data-msg="Dirección copiada">${icon('copy', 14)}Copiar</button></div>
        <p class="sm t2" style="margin:0">En Minecraft: Multijugador → Agregar servidor → esta dirección. Siempre lleva a la PC que lo esté alojando${r.hostName ? `: ahora, <b>${esc(r.hostName)}</b>` : ''}. Si cambia de PC, no tenés que tocar nada.</p>
        <div class="btns">${profileButton({ loader: r.loader }, {}, r.id)}${r.hasCopy ? `<button class="btn s" data-a="open" data-id="${esc(r.id)}">Abrir</button>` : ''}<button class="btn s ghost" data-a="remote-remove" data-id="${esc(r.id)}">${icon('logout', 14)}Salir del grupo</button></div>
      </div>`).join('')}</div>`
      : `<div class="empty">${icon('friends', 28)}<h2>Todavía no te uniste a ningún servidor</h2><p>Pedile a un amigo que te invite desde su servidor en 939 Host (Grupo → Invitar).</p></div>`}
  </div>`;
  $('#join-code').addEventListener('keydown', (e) => { if (e.key === 'Enter') A.join($('[data-a="join"]')); });
};
A.join = async (el) => {
  const code = $('#join-code').value.trim();
  if (!code) { toast('Pegá el código de invitación.', 'error'); return; }
  el.disabled = true; el.textContent = 'Uniéndome…';
  try { const r = await invoke('remote_join', { ticket: code }); toast(`Te uniste a ${r.name}`); } catch (e) { fail(e); }
  routes.friends();
};
A['remote-remove'] = async (el) => {
  if (!(await dialog({ title: 'Salir del grupo', body: '<p>Esta PC deja de participar en este servidor. Si eras coanfitrión, tu copia queda en Servidores y la podés borrar aparte. Para volver vas a necesitar otra invitación.</p>', confirm: 'Salir del grupo' }))) return;
  try { await invoke('remote_remove', { id: el.dataset.id }); } catch (e) { fail(e); }
  routes.friends();
};
