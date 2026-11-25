'use strict';
/* 939 Host — Amigos: invitar con un código, aceptar o rechazar, ver quién está conectado y
   mandar mensajes de texto. Sin GIF, stickers ni llamadas. La amistad no da acceso a ningún
   servidor: los grupos y sus permisos son aparte. Los mensajes viajan cifrados entre las PCs y
   quedan solo en cada una. */

const ppl = { view: null, open: null, chat: [], timer: null, notices: [] };

async function loadPeople() {
  try { ppl.view = await invoke('friends_view'); } catch (e) { ppl.view = { error: errMsg(e), friends: [], requests: [] }; }
  for (const n of ppl.view.notices || []) if (!ppl.notices.includes(n)) ppl.notices.push(n);
  const unread = (ppl.view.friends || []).reduce((n, f) => n + f.unread, 0) + (ppl.view.requests || []).length;
  if (unread !== state.peopleBadge) { state.peopleBadge = unread; paintNav(); }
}

routes.people = async function renderPeople() {
  await loadPeople();
  paintPeople();
  every(15000, async () => { if (state.route !== 'people') return; await loadPeople(); paintPeople(true); });
};

function friendRow(f) {
  const st = f.online ? (f.playing ? `Jugando en ${esc(f.playing)}` : f.available ? 'Disponible para jugar' : 'Conectado') : 'Desconectado';
  return `<button class="li click${ppl.open === f.node ? ' on' : ''}" data-a="ppl-open" data-node="${esc(f.node)}">${pcAvatar('', 36, '', f.avatar)}
    <div class="grow" style="min-width:0;text-align:left"><b class="ellipsis" style="display:block">${esc(f.name)}</b><span class="xs ${f.online ? 't2' : 't3'}">${f.online ? SHAPES.dot.replace('class="dot"', 'class="dot" style="display:inline-block;width:6px;height:6px;margin-right:5px"') : ''}${st}</span></div>
    ${f.unread ? `<span class="count">${f.unread}</span>` : ''}</button>`;
}

function paintPeople(soft = false) {
  const v = ppl.view;
  const fr = [...(v.friends || [])].sort((a, b) => (b.online - a.online) || a.name.localeCompare(b.name, 'es'));
  const groups = v.groups || [];
  const cur = fr.find((f) => f.node === ppl.open);
  const curG = groups.find((g) => `g:${g.id}` === ppl.open);
  if (soft && $('#ppl-list')) { $('#ppl-list').innerHTML = fr.length ? fr.map(friendRow).join('') : ''; if ($('#ppl-groups')) $('#ppl-groups').innerHTML = groups.map(groupRow).join(''); paintTyping(); return; }
  main.innerHTML = `<div class="page">
    <div class="head"><div><h1>Amigos</h1><p class="sub">Invitá a quien quieras, mirá quién está conectado y mandá mensajes. Ser amigos no da acceso a tus servidores.</p></div>
      <div class="btns"><button class="btn" data-a="ppl-add">${icon('link', 16)}Usar un código</button><button class="btn pri" data-a="ppl-invite">${icon('plus', 16)}Invitar</button></div></div>
    ${v.error ? `<div class="note">${icon('alert')}<div class="grow sm">${esc(v.error)}</div></div>` : ''}
    ${ppl.notices.map((n) => `<div class="note">${icon('alert')}<div class="grow sm">${esc(n)}</div></div>`).join('')}
    ${(v.requests || []).map((r) => `<div class="note strong">${pcAvatar('', 32)}<div class="grow"><b>${esc(r.name)} quiere ser tu amigo</b><span class="sm t2">Si aceptás, se ven conectados y pueden mandarse mensajes. No le da acceso a ningún servidor.</span></div><div class="btns"><button class="btn s" data-a="ppl-answer" data-node="${esc(r.node)}" data-v="no">Rechazar</button><button class="btn s pri" data-a="ppl-answer" data-node="${esc(r.node)}" data-v="si">Aceptar</button></div></div>`).join('')}
    <div class="grid2" style="grid-template-columns:minmax(260px,0.8fr) minmax(0,1.6fr)">
      <div class="stack" style="gap:12px">
        <label class="switch card pad" style="margin:0"><div class="grow"><b>Disponible para jugar</b><span class="sm t2">Tus amigos lo ven mientras estás conectado.</span></div><input type="checkbox" id="ppl-av" ${v.available ? 'checked' : ''}></label>
        <div class="card"><div class="card-head"><h2>Tus amigos</h2><span class="sm t3">${fr.filter((f) => f.online).length} de ${fr.length} conectados</span></div>
          <div class="list" id="ppl-list">${fr.length ? fr.map(friendRow).join('') : ''}</div>${fr.length ? '' : '<div class="pad sm t2">Todavía no agregaste a nadie. Tocá «Invitar» y mandale el código a quien quieras.</div>'}</div>
        <div class="card"><div class="card-head"><h2>Grupos</h2><button class="btn s" data-a="grp-new" ${fr.length ? '' : 'disabled title="Primero agregá amigos"'}>${icon('plus', 14)}Nuevo grupo</button></div>
          <div class="list" id="ppl-groups">${groups.map(groupRow).join('')}</div>${groups.length ? '' : '<div class="pad sm t2">Un grupo junta a varios amigos en un mismo chat.</div>'}</div>
        ${v.blocked ? `<button class="btn s ghost" data-a="ppl-unblock" style="align-self:flex-start">Desbloquear a ${v.blocked} ${v.blocked === 1 ? 'persona' : 'personas'}</button>` : ''}
      </div>
      <div class="card stack" id="ppl-chat" style="min-height:420px">${curG ? groupChatHTML(curG) : cur ? chatHTML(cur) : `<div class="empty" style="padding:48px 16px">${icon('user', 28)}<h2>Elegí un amigo</h2><p class="sm t2">Los mensajes viajan cifrados directo entre las dos PCs y se guardan solo en ellas. Si tu amigo no está conectado, se entregan cuando aparezca (con tu 939 Host abierto).</p></div>`}</div>
    </div></div>`;
  $('#ppl-av')?.addEventListener('change', (e) => invoke('friend_set_available', { on: e.target.checked }).catch(fail));
  if (curG) wireChat({ group: curG.id });
  else if (cur) wireChat(cur);
}

function groupRow(g) {
  const on = g.members.filter((m) => m.online && !m.me).length;
  return `<button class="li click${ppl.open === `g:${g.id}` ? ' on' : ''}" data-a="grp-open" data-id="${esc(g.id)}"><span class="avatar" style="width:36px;height:36px">${icon('friends', 18)}</span>
    <div class="grow" style="min-width:0;text-align:left"><b class="ellipsis" style="display:block">${esc(g.name)}</b><span class="xs t3 ellipsis">${g.typing.length ? `${esc(g.typing.join(', '))} ${g.typing.length > 1 ? 'están' : 'está'} escribiendo…` : `${g.members.length} personas · ${on} conectadas`}</span></div>
    ${g.unread ? `<span class="count">${g.unread}</span>` : ''}</button>`;
}

/* Códigos dentro de un mensaje (direcciones, puertos, comandos, ids de mods, enlaces): se
   marcan y se copian con un toque. Nunca se ejecuta nada: un comando solo se copia y un enlace
   se abre en el navegador solo si es http(s). */
const CODE_RE = /(https?:\/\/[^\s<>"]+)|([a-z0-9-]{1,63}\.939host(?::\d{1,5})?)|((?:\d{1,3}\.){3}\d{1,3}(?::\d{1,5})?)|(\[[0-9a-f:]+\](?::\d{1,5})?)|((?:[a-z0-9-]+\.)+[a-z]{2,}:\d{1,5})|((?:^|(?<=\s))\/[a-z][a-z0-9_:.-]*(?: [^\n]*)?)|(\b[a-z0-9_.-]{2,}:[a-z0-9_./-]{2,}\b)|(\b(?:puerto|port)\s+\d{2,5}\b)/gi;
const CODE_KIND = ['Enlace', 'Dirección .939host', 'IP', 'IP', 'Dirección', 'Comando', 'Id', 'Puerto'];
function linkify(text) {
  let out = '', last = 0;
  for (const m of text.matchAll(CODE_RE)) {
    const t = m[0], k = m.slice(1).findIndex((x) => x != null);
    out += esc(text.slice(last, m.index)).replace(/\n/g, '<br>');
    const val = k === 7 ? t.replace(/\D+/g, '') : t;
    out += k === 0
      ? `<span class="code-chip"><button class="lnk" data-a="chat-link" data-url="${esc(t)}" title="Abrir en el navegador">${esc(t)}</button><button class="cc" data-a="copy" data-text="${esc(t)}" data-msg="Enlace copiado" aria-label="Copiar enlace">${icon('copy', 12)}</button></span>`
      : `<button class="code-chip mono" data-a="copy" data-text="${esc(val)}" data-msg="${CODE_KIND[k]} copiado" title="${CODE_KIND[k]}: tocá para copiar${k === 5 ? ' (no se ejecuta)' : ''}">${esc(t)}${icon('copy', 12)}</button>`;
    last = m.index + t.length;
  }
  return out + esc(text.slice(last)).replace(/\n/g, '<br>');
}
// Enlaces de sitios conocidos se abren; el resto se copia (939 Host no abre direcciones desconocidas).
A['chat-link'] = async (el) => {
  try { await invoke('open_link', { url: el.dataset.url }); }
  catch { try { await navigator.clipboard.writeText(el.dataset.url); toast('Enlace copiado: pegalo en tu navegador si confiás en él.'); } catch { toast('No se pudo copiar', 'error'); } }
};
function msgHTML(m, group) {
  const who = group && !m.fromMe ? `<span class="xs b">${esc(m.fromName || 'Alguien')}</span>` : '';
  const pend = m.fromMe && !m.delivered ? (group && m.pending?.length ? ` · falta entregar a ${m.pending.length}` : ' · pendiente') : '';
  return `<div class="msg ${m.fromMe ? 'me' : ''}">${who}<div class="bubble">${linkify(m.text)}</div><span class="xs t3 row" style="gap:6px">${esc(new Date(m.at).toLocaleString('es-AR', { dateStyle: 'short', timeStyle: 'short' }))}${pend}<button class="msg-copy" data-a="copy" data-text="${esc(m.text)}" data-msg="Mensaje copiado" aria-label="Copiar mensaje" title="Copiar mensaje">${icon('copy', 12)}</button></span></div>`;
}

function groupChatHTML(g) {
  const msgs = ppl.chat.map((m) => msgHTML(m, true)).join('');
  return `<div class="card-head"><div class="row" style="gap:10px"><span class="avatar" style="width:32px;height:32px">${icon('friends', 16)}</span><div style="min-width:0"><b>${esc(g.name)}</b><div class="xs t3 ellipsis">${esc(g.members.map((m) => (m.me ? 'Vos' : m.name)).join(', '))}</div></div></div>
      <div class="btns"><button class="btn s ghost" data-a="grp-add" data-id="${esc(g.id)}">${icon('plus', 14)}Sumar amigos</button><button class="btn s ghost" data-a="grp-rename" data-id="${esc(g.id)}">Cambiar nombre</button><button class="btn s ghost" data-a="grp-leave" data-id="${esc(g.id)}" data-tone="halt">Salir</button></div></div>
    <div class="chat-log" id="chat-log">${msgs || '<div class="sm t3" style="text-align:center;padding:24px">Todavía no hay mensajes.</div>'}</div>
    <div class="xs t3" id="chat-typing" style="min-height:16px;padding:0 16px" aria-live="polite"></div>
    <form class="row chat-form" id="chat-form" style="gap:8px"><textarea class="input" id="chat-in" rows="1" maxlength="500" placeholder="Escribí al grupo (hasta 500 caracteres)" aria-label="Mensaje"></textarea><button class="btn pri" type="submit">Enviar</button></form>`;
}

/* «Está escribiendo»: lo muestra el chat abierto. Vence solo (6 s sin novedades). */
function paintTyping() {
  const el = $('#chat-typing'); if (!el) return;
  const v = ppl.view || {};
  let names = [];
  if (ppl.open?.startsWith('g:')) names = (v.groups || []).find((g) => `g:${g.id}` === ppl.open)?.typing || [];
  else { const f = (v.friends || []).find((x) => x.node === ppl.open); if (f?.typing) names = [f.name]; }
  el.textContent = names.length ? `${names.join(', ')} ${names.length > 1 ? 'están' : 'está'} escribiendo…` : '';
  clearTimeout(ppl.typingTimer);
  if (names.length) ppl.typingTimer = setTimeout(async () => { await loadPeople(); if (state.route === 'people') paintPeople(true); }, 6500);
}

function chatHTML(f) {
  const msgs = ppl.chat.map((m) => msgHTML(m, false)).join('');
  return `<div class="card-head"><div class="row" style="gap:10px">${pcAvatar('', 32, '', f.avatar)}<div><b>${esc(f.name)}</b><div class="xs t3">${f.online ? 'Conectado' : 'Desconectado'}${f.sharedGroups.length ? ` · Comparten: ${esc(f.sharedGroups.join(', '))}` : ''}</div></div></div>
      <div class="btns"><button class="btn s ghost" data-a="ppl-remove" data-node="${esc(f.node)}" data-name="${esc(f.name)}">Quitar</button><button class="btn s ghost" data-a="ppl-block" data-node="${esc(f.node)}" data-name="${esc(f.name)}" data-tone="halt">Bloquear</button></div></div>
    <div class="chat-log" id="chat-log">${msgs || '<div class="sm t3" style="text-align:center;padding:24px">Todavía no hay mensajes.</div>'}</div>
    <div class="xs t3" id="chat-typing" style="min-height:16px;padding:0 16px" aria-live="polite"></div>
    <form class="row chat-form" id="chat-form" style="gap:8px"><textarea class="input" id="chat-in" rows="1" maxlength="500" placeholder="Escribí un mensaje (hasta 500 caracteres)" aria-label="Mensaje"></textarea><button class="btn pri" type="submit">Enviar</button></form>`;
}

function wireChat(f) {
  const log = $('#chat-log'); if (log) log.scrollTop = log.scrollHeight;
  const form = $('#chat-form'), inp = $('#chat-in');
  const to = f.group || f.node;
  paintTyping();
  inp?.addEventListener('keydown', (e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); form.requestSubmit(); } });
  // El campo crece con el texto (hasta unas 5 líneas) en lugar de ocupar siempre ese alto.
  const grow = () => { inp.style.height = 'auto'; inp.style.height = `${Math.min(inp.scrollHeight + 2, 140)}px`; };
  // «Está escribiendo»: se avisa como mucho cada 3 s mientras hay texto, y se apaga al borrar,
  // al enviar o al salir del campo (y vence solo del otro lado si el aviso se pierde).
  let typingAt = 0, typingOn = false;
  const typing = (on) => {
    if (on && Date.now() - typingAt < 3000) return;
    if (!on && !typingOn) return;
    typingOn = on; typingAt = on ? Date.now() : 0;
    invoke('chat_typing', { to, on }).catch(() => {});
  };
  inp?.addEventListener('input', () => { grow(); typing(!!inp.value.trim()); });
  inp?.addEventListener('blur', () => typing(false));
  form?.addEventListener('submit', async (e) => {
    e.preventDefault();
    const text = inp.value.trim(); if (!text) return;
    inp.disabled = true;
    typing(false);
    try {
      if (f.group) { const n = await invoke('group_chat_send', { id: f.group, text }); inp.value = ''; if (n) toast(`No se pudo entregar a ${n} ${n === 1 ? 'persona' : 'personas'} todavía: se reintenta sola.`); }
      else { const ok = await invoke('friend_send', { node: f.node, text }); inp.value = ''; if (!ok) toast('Sin conexión con tu amigo: se entrega cuando aparezca.'); }
    } catch (err) { fail(err); }
    inp.disabled = false; inp.focus();
    ppl.chat = await (f.group ? invoke('group_chat_messages', { id: f.group }) : invoke('friend_chat', { node: f.node })).catch(() => ppl.chat);
    paintPeople();
  });
}

A['grp-open'] = async (el) => {
  ppl.open = `g:${el.dataset.id}`;
  ppl.chat = await invoke('group_chat_messages', { id: el.dataset.id }).catch(() => []);
  await loadPeople(); paintPeople(); $('#chat-in')?.focus();
};
A['grp-new'] = async () => {
  const fr = ppl.view?.friends || [];
  let name = '';
  const pick = new Set();
  const ok = await dialog({ title: 'Nuevo grupo', confirm: 'Crear',
    body: `<label class="field">Nombre<input class="input" id="gn-name" maxlength="40" placeholder="Por ejemplo: Los del survival"></label>
      <div class="sec">Amigos</div><div class="stack" style="gap:6px;max-height:260px;overflow:auto">${fr.map((f) => `<label class="row" style="gap:10px"><input type="checkbox" data-gn="${esc(f.node)}">${pcAvatar('', 28, '', f.avatar)}<span>${esc(f.name)}</span></label>`).join('')}</div>
      <p class="xs t3">Solo podés agregar a tus amigos. El grupo no da acceso a ningún servidor.</p>`,
    onMount: (d) => { const i = d.querySelector('#gn-name'); i.addEventListener('input', () => { name = i.value; }); d.querySelectorAll('[data-gn]').forEach((c) => c.addEventListener('change', () => { if (c.checked) pick.add(c.dataset.gn); else pick.delete(c.dataset.gn); })); i.focus(); } });
  if (!ok) return;
  if (!name.trim() || !pick.size) { toast('Poné un nombre y elegí al menos un amigo.', 'error'); return; }
  try { const id = await invoke('group_chat_create', { name, members: [...pick] }); ppl.open = `g:${id}`; ppl.chat = []; toast('Grupo creado'); } catch (e) { fail(e); }
  await loadPeople(); paintPeople();
};
A['grp-rename'] = async (el) => {
  const g = (ppl.view?.groups || []).find((x) => x.id === el.dataset.id); if (!g) return;
  let name = g.name;
  const ok = await dialog({ title: 'Cambiar el nombre del grupo', confirm: 'Guardar', body: `<label class="field">Nombre<input class="input" id="gr-name" maxlength="40" value="${esc(g.name)}"></label><p class="xs t3">Lo ven todas las personas del grupo.</p>`, onMount: (d) => { const i = d.querySelector('#gr-name'); i.addEventListener('input', () => { name = i.value; }); i.select(); } });
  if (!ok || !name.trim() || name.trim() === g.name) return;
  try { await invoke('group_chat_rename', { id: g.id, name }); toast('Nombre cambiado'); } catch (e) { fail(e); }
  await loadPeople(); paintPeople();
};
A['grp-add'] = async (el) => {
  const g = (ppl.view?.groups || []).find((x) => x.id === el.dataset.id); if (!g) return;
  const inside = new Set(g.members.map((m) => m.node));
  const fr = (ppl.view?.friends || []).filter((f) => !inside.has(f.node));
  if (!fr.length) { toast('Todos tus amigos ya están en este grupo. Agregá amigos nuevos desde «Invitar».'); return; }
  const pick = new Set();
  const ok = await dialog({ title: `Sumar amigos a «${esc(g.name)}»`, confirm: 'Sumar',
    body: `<div class="stack" style="gap:6px;max-height:260px;overflow:auto">${fr.map((f) => `<label class="row" style="gap:10px"><input type="checkbox" data-ga="${esc(f.node)}">${pcAvatar('', 28, '', f.avatar)}<span>${esc(f.name)}</span></label>`).join('')}</div><p class="xs t3">Ven los mensajes nuevos desde ahora. El grupo no da acceso a ningún servidor.</p>`,
    onMount: (d) => d.querySelectorAll('[data-ga]').forEach((c) => c.addEventListener('change', () => { if (c.checked) pick.add(c.dataset.ga); else pick.delete(c.dataset.ga); })) });
  if (!ok || !pick.size) return;
  try { const n = await invoke('group_chat_add', { id: g.id, members: [...pick] }); toast(n === 1 ? 'Se sumó 1 persona' : `Se sumaron ${n} personas`); } catch (e) { fail(e); }
  await loadPeople(); paintPeople();
};
A['grp-leave'] = async (el) => {
  const g = (ppl.view?.groups || []).find((x) => x.id === el.dataset.id); if (!g) return;
  if (!(await dialog({ title: `Salir de «${esc(g.name)}»`, body: '<p>Dejás de recibir los mensajes del grupo. Las demás personas siguen en el grupo.</p>', confirm: 'Salir' }))) return;
  try { await invoke('group_chat_leave', { id: g.id }); ppl.open = null; } catch (e) { fail(e); }
  await loadPeople(); paintPeople();
};

A['ppl-open'] = async (el) => {
  ppl.open = el.dataset.node;
  ppl.chat = await invoke('friend_chat', { node: ppl.open }).catch(() => []);
  await loadPeople(); paintPeople(); $('#chat-in')?.focus();
};
A['ppl-invite'] = async () => {
  let code;
  try { code = await invoke('friend_invite'); } catch (e) { fail(e); return; }
  await dialog({ title: 'Invitar a un amigo', body: `<p>Mandale este código por mensaje privado. Sirve una sola vez y vence en 48 horas. Cuando lo use, te llega una solicitud que podés aceptar o rechazar.</p><div class="addr"><span class="sm mono">${esc(code)}</span></div><p class="sm t2">Ser amigos no le da acceso a ningún servidor. Para jugar en uno, invitalo desde el servidor (Grupo → Invitar).</p>`, confirm: 'Copiar y cerrar', cancel: null });
  try { await navigator.clipboard.writeText(code); toast('Código copiado'); } catch { /* sin portapapeles */ }
};
A['ppl-add'] = async () => {
  let code = '';
  const ok = await dialog({ title: 'Usar un código de amistad', confirm: 'Enviar solicitud', body: '<label class="field">Código<input class="input mono" id="ppl-code" placeholder="amigo:…" spellcheck="false" autocomplete="off"></label>', onMount: (d) => { const i = d.querySelector('#ppl-code'); i.focus(); i.addEventListener('input', () => { code = i.value; }); } });
  if (!ok || !code.trim()) return;
  try { await invoke('friend_add', { code: code.trim() }); toast('Solicitud enviada'); } catch (e) { fail(e); }
};
A['ppl-answer'] = async (el) => {
  try { await invoke('friend_answer', { node: el.dataset.node, accept: el.dataset.v === 'si' }); toast(el.dataset.v === 'si' ? 'Ahora son amigos' : 'Solicitud rechazada'); } catch (e) { fail(e); }
  await loadPeople(); paintPeople();
};
A['ppl-remove'] = async (el) => {
  if (!(await dialog({ title: `Quitar a ${el.dataset.name}`, body: '<p>Dejan de verse conectados y de poder mandarse mensajes. Los mensajes guardados en esta PC se conservan. No cambia nada en los grupos que compartan.</p>', confirm: 'Quitar' }))) return;
  try { await invoke('friend_remove', { node: el.dataset.node, block: false }); ppl.open = null; } catch (e) { fail(e); }
  await loadPeople(); paintPeople();
};
A['ppl-block'] = async (el) => {
  if (!(await dialog({ title: `Bloquear a ${el.dataset.name}`, danger: true, body: '<p>No va a poder mandarte solicitudes ni mensajes, ni ver si estás conectado. No cambia nada en los grupos que compartan: para eso, quitalo desde el servidor.</p>', confirm: 'Bloquear' }))) return;
  try { await invoke('friend_remove', { node: el.dataset.node, block: true }); ppl.open = null; toast('Bloqueado'); } catch (e) { fail(e); }
  await loadPeople(); paintPeople();
};
A['ppl-unblock'] = async () => { try { await invoke('friend_unblock_all'); toast('Desbloqueados'); } catch (e) { fail(e); } await loadPeople(); paintPeople(); };

listen('friend-message', async (ev) => {
  const key = ev.payload.node;
  const isGroup = (ppl.view?.groups || []).some((g) => g.id === key) || ppl.open === `g:${key}`;
  if (state.route === 'people' && ppl.open === (isGroup ? `g:${key}` : key)) {
    ppl.chat = await (isGroup ? invoke('group_chat_messages', { id: key }) : invoke('friend_chat', { node: key })).catch(() => ppl.chat);
  }
  await loadPeople();
  // Sin repintar todo si se está escribiendo (no se pierde el texto del campo).
  if (state.route === 'people') {
    const draft = $('#chat-in')?.value || '';
    paintPeople();
    if (draft && $('#chat-in')) { $('#chat-in').value = draft; $('#chat-in').focus(); }
  } else {
    const g = (ppl.view?.groups || []).find((x) => x.id === key);
    const f = (ppl.view?.friends || []).find((x) => x.node === key);
    if (g) toast(`Mensaje nuevo en ${g.name}`); else toast(`Mensaje nuevo de ${f?.name || 'un amigo'}`);
  }
});
listen('friend-typing', async () => {
  if (state.route !== 'people') return;
  await loadPeople();
  paintPeople(true);
});
