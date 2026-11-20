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
  const cur = fr.find((f) => f.node === ppl.open);
  if (soft && $('#ppl-list')) { $('#ppl-list').innerHTML = fr.length ? fr.map(friendRow).join('') : ''; return; }
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
        ${v.blocked ? `<button class="btn s ghost" data-a="ppl-unblock" style="align-self:flex-start">Desbloquear a ${v.blocked} ${v.blocked === 1 ? 'persona' : 'personas'}</button>` : ''}
      </div>
      <div class="card stack" id="ppl-chat" style="min-height:420px">${cur ? chatHTML(cur) : `<div class="empty" style="padding:48px 16px">${icon('user', 28)}<h2>Elegí un amigo</h2><p class="sm t2">Los mensajes viajan cifrados directo entre las dos PCs y se guardan solo en ellas. Si tu amigo no está conectado, se entregan cuando aparezca (con tu 939 Host abierto).</p></div>`}</div>
    </div></div>`;
  $('#ppl-av')?.addEventListener('change', (e) => invoke('friend_set_available', { on: e.target.checked }).catch(fail));
  if (cur) wireChat(cur);
}

function chatHTML(f) {
  const msgs = ppl.chat.map((m) => `<div class="msg ${m.fromMe ? 'me' : ''}"><div class="bubble">${esc(m.text).replace(/\n/g, '<br>')}</div><span class="xs t3">${esc(new Date(m.at).toLocaleString('es-AR', { dateStyle: 'short', timeStyle: 'short' }))}${m.fromMe && !m.delivered ? ' · pendiente' : ''}</span></div>`).join('');
  return `<div class="card-head"><div class="row" style="gap:10px">${pcAvatar('', 32, '', f.avatar)}<div><b>${esc(f.name)}</b><div class="xs t3">${f.online ? 'Conectado' : 'Desconectado'}${f.sharedGroups.length ? ` · Comparten: ${esc(f.sharedGroups.join(', '))}` : ''}</div></div></div>
      <div class="btns"><button class="btn s ghost" data-a="ppl-remove" data-node="${esc(f.node)}" data-name="${esc(f.name)}">Quitar</button><button class="btn s ghost" data-a="ppl-block" data-node="${esc(f.node)}" data-name="${esc(f.name)}" data-tone="halt">Bloquear</button></div></div>
    <div class="chat-log" id="chat-log">${msgs || '<div class="sm t3" style="text-align:center;padding:24px">Todavía no hay mensajes.</div>'}</div>
    <form class="row chat-form" id="chat-form" style="gap:8px"><textarea class="input" id="chat-in" rows="1" maxlength="500" placeholder="Escribí un mensaje (hasta 500 caracteres)" aria-label="Mensaje"></textarea><button class="btn pri" type="submit">Enviar</button></form>`;
}

function wireChat(f) {
  const log = $('#chat-log'); if (log) log.scrollTop = log.scrollHeight;
  const form = $('#chat-form'), inp = $('#chat-in');
  inp?.addEventListener('keydown', (e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); form.requestSubmit(); } });
  // El campo crece con el texto (hasta unas 5 líneas) en lugar de ocupar siempre ese alto.
  const grow = () => { inp.style.height = 'auto'; inp.style.height = `${Math.min(inp.scrollHeight + 2, 140)}px`; };
  inp?.addEventListener('input', grow);
  form?.addEventListener('submit', async (e) => {
    e.preventDefault();
    const text = inp.value.trim(); if (!text) return;
    inp.disabled = true;
    try { const ok = await invoke('friend_send', { node: f.node, text }); inp.value = ''; if (!ok) toast('Sin conexión con tu amigo: se entrega cuando aparezca.'); } catch (err) { fail(err); }
    inp.disabled = false; inp.focus();
    ppl.chat = await invoke('friend_chat', { node: f.node }).catch(() => ppl.chat);
    paintPeople();
  });
}

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
  if (state.route === 'people' && ppl.open === ev.payload.node) { ppl.chat = await invoke('friend_chat', { node: ppl.open }).catch(() => ppl.chat); }
  await loadPeople();
  if (state.route === 'people') paintPeople();
  else { const f = (ppl.view?.friends || []).find((x) => x.node === ev.payload.node); toast(`Mensaje nuevo de ${f?.name || 'un amigo'}`); }
});
