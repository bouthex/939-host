'use strict';
/* Consola: filtros, autocompletado con ayuda de argumentos, historial y acciones rápidas.
   Acepta comandos con o sin «/». Las consultas internas de 939 Host no aparecen acá. */

const CMDS = [
  ['help', '', 'Lista de comandos'], ['list', '', 'Jugadores conectados'], ['say', '<mensaje>', 'Mensaje para todos'],
  ['tell', '<jugador> <mensaje>', 'Mensaje privado'], ['msg', '<jugador> <mensaje>', 'Mensaje privado'],
  ['tp', '<jugador> <destino|x y z>', 'Teletransportar'], ['give', '<jugador> <ítem> [cantidad]', 'Dar un ítem'],
  ['clear', '<jugador> [ítem]', 'Vaciar inventario'], ['gamemode', '<survival|creative|adventure|spectator> [jugador]', 'Modo de juego'],
  ['defaultgamemode', '<modo>', 'Modo por defecto'], ['difficulty', '<peaceful|easy|normal|hard>', 'Dificultad'],
  ['time', 'set <day|night|noon|midnight|ticks>', 'Hora del mundo'], ['weather', '<clear|rain|thunder> [duración]', 'Clima'],
  ['gamerule', '<regla> [valor]', 'Reglas del juego'], ['kill', '[objetivo]', 'Eliminar entidades'],
  ['effect', 'give <jugador> <efecto> [segundos] [nivel]', 'Dar un efecto'], ['enchant', '<jugador> <encantamiento> [nivel]', 'Encantar el ítem en mano'],
  ['xp', 'add <jugador> <cantidad> [levels|points]', 'Experiencia'], ['spawnpoint', '[jugador] [x y z]', 'Punto de aparición'],
  ['setworldspawn', '[x y z]', 'Spawn del mundo'], ['locate', 'structure <estructura>', 'Buscar estructura'],
  ['seed', '', 'Semilla del mundo'], ['kick', '<jugador> [motivo]', 'Expulsar'], ['ban', '<jugador> [motivo]', 'Banear'],
  ['pardon', '<jugador>', 'Quitar ban'], ['ban-ip', '<ip|jugador> [motivo]', 'Banear IP'], ['pardon-ip', '<ip>', 'Quitar ban de IP'],
  ['banlist', '[ips|players]', 'Ver baneados'], ['op', '<jugador>', 'Hacer operador'], ['deop', '<jugador>', 'Quitar operador'],
  ['whitelist', '<add|remove|list|on|off|reload> [jugador]', 'Lista blanca'], ['save-all', '[flush]', 'Guardar el mundo'],
  ['save-off', '', 'Pausar el guardado automático'], ['save-on', '', 'Reanudar el guardado automático'],
  ['tick', '<freeze|unfreeze|rate|query>', 'Control del tiempo de juego'], ['worldborder', '<set|add|center> …', 'Borde del mundo'],
  ['summon', '<entidad> [x y z]', 'Invocar entidad'], ['setblock', '<x y z> <bloque>', 'Poner bloque'], ['fill', '<desde> <hasta> <bloque>', 'Rellenar área'],
  ['scoreboard', '<objectives|players> …', 'Marcadores'], ['title', '<jugador> <title|subtitle|actionbar> <texto>', 'Texto en pantalla'],
  ['particle', '<partícula> [x y z]', 'Partículas'], ['playsound', '<sonido> <fuente> <jugador>', 'Reproducir sonido'],
  ['stop', '', 'Detener el servidor (mejor usá el botón Detener)'],
];
const PLAYER_CMDS = new Set(['tell', 'msg', 'tp', 'give', 'clear', 'kick', 'ban', 'pardon', 'op', 'deop', 'enchant', 'spawnpoint']);
const FILTERS = [['all', 'Todo'], ['cmd', 'Comandos'], ['chat', 'Chat'], ['warn', 'Avisos'], ['err', 'Errores']];
const con = { filter: 'all', follow: true, hist: {}, idx: -1, ac: [], acSel: 0 };

function lineKind(l) {
  if (l.startsWith('> ')) return 'cmd';
  if (l.startsWith('──')) return 'sys';
  if (l.startsWith('[stderr]') || /\/(ERROR|FATAL)\]|Exception|^\s+at /.test(l)) return 'err';
  if (/\/WARN\]/.test(l)) return 'warn';
  if (/\]: (\[Not Secure\] )?<[^>]+> |\]: \[Server\] |joined the game|left the game/.test(l)) return 'chat';
  return 'info';
}
function lineHTML(l) {
  const k = lineKind(l);
  const hide = con.filter !== 'all' && !(con.filter === k || (con.filter === 'warn' && k === 'err'));
  return `<div class="${k}" ${hide ? 'hidden' : ''}>${esc(l)}</div>`;
}

tabDraw.console = async (el) => {
  const id = state.params.id;
  const lines = await invoke('console_history', { id });
  const v = state.server;
  const on = v.status === 'running' || v.status === 'starting';
  el.innerHTML = `<div class="stack" style="gap:12px">
    <div class="spread"><div class="quick" aria-label="Acciones rápidas">
      ${[['save-all', 'Guardar mundo'], ['list', 'Ver conectados'], ['time set day', 'Hacer de día'], ['weather clear', 'Despejar clima']].map(([c, l]) => `<button class="btn s" data-a="qcmd" data-cmd="${c}" ${v.status === 'running' ? '' : 'disabled'}>${l}</button>`).join('')}
      <button class="btn s" data-a="qsay" ${v.status === 'running' ? '' : 'disabled'}>Avisar a todos…</button></div>
      <div class="btns"><button class="btn s ghost" data-a="con-clear">Limpiar vista</button><button class="btn s ghost" data-a="folder" data-which="logs">${icon('folder', 14)}Registros</button></div></div>
    <div class="term">
      <div class="term-head"><div class="seg" role="group" aria-label="Filtrar">${FILTERS.map(([k, l]) => `<button class="${con.filter === k ? 'on' : ''}" data-a="con-filter" data-v="${k}">${l}</button>`).join('')}</div>
        <div class="row" style="gap:14px">${on ? '<span class="live"><span class="dot"></span>En vivo</span>' : '<span class="live">Servidor detenido</span>'}
        <label class="live" style="cursor:pointer"><input type="checkbox" id="con-follow" ${con.follow ? 'checked' : ''} style="accent-color:#fff">Seguir al final</label></div></div>
      <div class="term-log" id="con-log" tabindex="0" aria-live="off" aria-label="Salida de la consola"></div>
      <div class="term-in"><span style="color:#8A8A8A">›</span><input id="con-in" placeholder="${on ? 'Escribí un comando (con o sin /) — Tab completa, ↑ historial' : 'Iniciá el servidor para usar la consola'}" ${on ? '' : 'disabled'} autocomplete="off" spellcheck="false" aria-label="Comando" aria-autocomplete="list"><div class="ac hidden" id="con-ac" role="listbox"></div></div>
      <div class="hintline" id="con-hint"></div>
    </div></div>`;
  const log = $('#con-log');
  log.innerHTML = lines.map(lineHTML).join('');
  log.scrollTop = log.scrollHeight;
  log.addEventListener('scroll', () => {
    const atEnd = log.scrollHeight - log.scrollTop - log.clientHeight < 24;
    if (atEnd !== con.follow) { con.follow = atEnd; const f = $('#con-follow'); if (f) f.checked = atEnd; }
  });
  $('#con-follow').addEventListener('change', (e) => { con.follow = e.target.checked; if (con.follow) log.scrollTop = log.scrollHeight; });
  const inp = $('#con-in');
  inp.addEventListener('input', () => updateAc(inp));
  inp.addEventListener('keydown', (e) => conKey(e, inp));
  inp.addEventListener('blur', () => setTimeout(() => $('#con-ac')?.classList.add('hidden'), 150));
  if (!inp.disabled) inp.focus();
};
tabRefresh.console = () => {
  const inp = $('#con-in');
  const on = state.server.status === 'running' || state.server.status === 'starting';
  if (inp && inp.disabled === on) { const el = $('#srv-tab'); if (el) tabDraw.console(el); }
};
function appendConsole(line) {
  const log = $('#con-log');
  if (!log) return;
  log.insertAdjacentHTML('beforeend', lineHTML(line));
  while (log.childElementCount > 3000) log.firstElementChild.remove();
  if (con.follow) log.scrollTop = log.scrollHeight;
}
async function sendCmd(raw) {
  const cmd = raw.trim().replace(/^\/+/, '');
  if (!cmd) return;
  const id = state.params.id;
  const h = (con.hist[id] = con.hist[id] || []);
  if (h[0] !== cmd) h.unshift(cmd);
  if (h.length > 100) h.pop();
  con.idx = -1;
  appendConsole(`> ${cmd}`);
  try { await invoke('send_command', { id, command: cmd }); } catch (e) { fail(e); }
}
function players() { return state.server?.status === 'running' ? state.server.players : []; }
function updateAc(inp) {
  const box = $('#con-ac'), hint = $('#con-hint');
  const raw = inp.value.replace(/^\/+/, '');
  const parts = raw.split(' ');
  let items = [];
  if (parts.length === 1 && parts[0]) {
    items = CMDS.filter(([c]) => c.startsWith(parts[0].toLowerCase())).slice(0, 8).map(([c, a, d]) => ({ value: c + (a ? ' ' : ''), label: c, args: a, desc: d }));
  } else if (parts.length === 2 && PLAYER_CMDS.has(parts[0])) {
    items = players().filter((p) => p.toLowerCase().startsWith(parts[1].toLowerCase())).map((p) => ({ value: `${parts[0]} ${p} `, label: p, args: '', desc: 'Conectado' }));
  }
  const def = CMDS.find(([c]) => c === parts[0]);
  hint.textContent = def && parts.length > 1 ? `${def[0]} ${def[1]} — ${def[2]}` : '';
  con.ac = items; con.acSel = 0;
  if (!items.length || (items.length === 1 && items[0].value.trim() === raw.trim())) { box.classList.add('hidden'); return; }
  box.innerHTML = items.map((it, i) => `<button type="button" role="option" class="${i === 0 ? 'on' : ''}" data-i="${i}"><span>${esc(it.label)}</span><span class="args">${esc(it.args)}</span><span class="desc">${esc(it.desc)}</span></button>`).join('');
  box.querySelectorAll('button').forEach((b) => b.addEventListener('mousedown', (e) => { e.preventDefault(); pickAc(inp, +b.dataset.i); }));
  box.classList.remove('hidden');
}
function pickAc(inp, i) {
  const it = con.ac[i];
  if (!it) return;
  inp.value = it.value;
  inp.focus();
  updateAc(inp);
}
function conKey(e, inp) {
  const box = $('#con-ac');
  const open = box && !box.classList.contains('hidden') && con.ac.length;
  const h = con.hist[state.params.id] || [];
  if (e.key === 'Tab') { e.preventDefault(); if (open) pickAc(inp, con.acSel); return; }
  if (open && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
    e.preventDefault();
    con.acSel = (con.acSel + (e.key === 'ArrowDown' ? 1 : con.ac.length - 1)) % con.ac.length;
    box.querySelectorAll('button').forEach((b, i) => b.classList.toggle('on', i === con.acSel));
    return;
  }
  if (e.key === 'Escape') { box?.classList.add('hidden'); return; }
  if (e.key === 'Enter') {
    if (open && inp.value.replace(/^\/+/, '').split(' ').length === 1 && con.ac[con.acSel] && con.ac[con.acSel].label !== inp.value.replace(/^\/+/, '')) { e.preventDefault(); pickAc(inp, con.acSel); return; }
    const v = inp.value; inp.value = ''; box?.classList.add('hidden'); $('#con-hint').textContent = '';
    sendCmd(v);
    return;
  }
  if (e.key === 'ArrowUp') { if (con.idx < h.length - 1) inp.value = h[++con.idx]; e.preventDefault(); }
  else if (e.key === 'ArrowDown') { inp.value = con.idx > 0 ? h[--con.idx] : (con.idx = -1, ''); e.preventDefault(); }
}
A['con-filter'] = (el) => {
  con.filter = el.dataset.v;
  el.parentElement.querySelectorAll('button').forEach((b) => b.classList.toggle('on', b === el));
  const log = $('#con-log');
  if (!log) return;
  for (const d of log.children) { const k = d.className; d.hidden = con.filter !== 'all' && !(con.filter === k || (con.filter === 'warn' && k === 'err')); }
  log.scrollTop = log.scrollHeight;
};
A['con-clear'] = () => { const log = $('#con-log'); if (log) log.innerHTML = ''; };
A.qcmd = (el) => sendCmd(el.dataset.cmd);
A.qsay = async () => {
  let msg = '';
  const ok = await dialog({ title: 'Avisar a todos', body: '<label class="field">Mensaje<input class="input" id="say-in" maxlength="200" placeholder="Por ejemplo: en 10 minutos reinicio el servidor"></label>', confirm: 'Enviar', onMount: (d) => { const i = d.querySelector('#say-in'); i.focus(); i.addEventListener('input', () => { msg = i.value; }); } });
  if (ok && msg.trim()) sendCmd(`say ${msg.trim()}`);
};
