/* 939 Host — Papelera: servidores borrados que se conservan 30 días. */

const fmtBytes = (b) => fmtMB((b || 0) / 1048576);
const daysText = (d) => (d <= 0 ? 'Se borra hoy' : d === 1 ? 'Queda 1 día' : `Quedan ${d} días`);

routes.trash = async function renderTrash() {
  main.innerHTML = `<div class="page"><div class="head"><div><h1>Papelera</h1><p class="sub">Los servidores borrados se guardan 30 días. Podés restaurarlos completos o borrarlos para siempre.</p></div></div><div id="trash-body">${vloadBlock('Leyendo la Papelera')}</div></div>`;
  await paintTrash();
};

function vloadBlock(text) { return `<div class="card pad row" style="gap:12px">${SHAPES.ring}<span class="t2">${esc(text)}…</span></div>`; }

async function paintTrash() {
  const body = $('#trash-body');
  if (!body) return;
  let items;
  try { items = await invoke('trash_list', {}); } catch (e) { fail(e); items = []; }
  state.trashCount = items.length; paintNav();
  if (!items.length) {
    body.innerHTML = `<div class="empty card"><span class="wi">${icon('trash', 22)}</span><h2>La Papelera está vacía</h2><p class="sm t2">Cuando borres un servidor, va a quedar acá 30 días por si cambiás de idea.</p></div>`;
    return;
  }
  const total = items.reduce((a, b) => a + b.sizeBytes, 0);
  body.innerHTML = `<div class="stack" style="gap:16px">
    <div class="spread"><span class="sm t2">${items.length === 1 ? '1 servidor' : `${items.length} servidores`} · ${fmtBytes(total)}</span>
      <button class="btn s" data-a="trash-empty" data-tone="danger">${icon('trash', 14)}Vaciar papelera</button></div>
    <div class="card list">${items.map((it) => `<div class="li trash-it">
      <span class="srv-ic">${icon('servers', 18)}</span>
      <div class="grow" style="min-width:0"><b class="ellipsis" style="display:block">${esc(it.meta.name)}</b>
        <span class="sm t2">${esc([it.meta.loader && it.meta.loader !== 'Vanilla' ? it.meta.loader : 'Vanilla', it.meta.mcVersion].filter(Boolean).join(' '))} · borrado el ${esc(fmtDate(it.meta.deletedAt))} · ${fmtBytes(it.sizeBytes)}</span>
        ${it.conflict ? '<span class="xs t3" style="display:block">Ya hay un servidor con ese nombre: se va a restaurar como «(restaurado)», sin tocar el otro.</span>' : ''}</div>
      <span class="days ${it.daysLeft <= 3 ? 'soon' : ''}" title="Se borra para siempre 30 días después de borrarlo">${daysText(it.daysLeft)}</span>
      <div class="btns"><button class="btn s" data-a="trash-restore" data-tone="go" data-key="${esc(it.key)}" data-name="${esc(it.meta.name)}">${icon('restart', 14)}Restaurar</button>
        <button class="btn s icon ghost" data-a="trash-purge" data-tone="danger" data-key="${esc(it.key)}" data-name="${esc(it.meta.name)}" data-size="${it.sizeBytes}" aria-label="Borrar para siempre">${icon('trash', 15)}</button></div>
    </div>`).join('')}</div>
    <p class="xs t3">Se restaura todo: mundo, mods, configuración, copias, puntos de restauración e identidad del servidor. Lo que lleva más de 30 días se borra solo y libera el espacio.</p></div>`;
}

A['trash-restore'] = async (el) => {
  el.setAttribute('aria-busy', 'true');
  try {
    const id = await invoke('trash_restore', { key: el.dataset.key });
    toast(`«${el.dataset.name}» restaurado`);
    state.servers = await invoke('list_servers').catch(() => state.servers);
    go('server', { id });
  } catch (e) { el.removeAttribute('aria-busy'); fail(e); }
};

A['trash-purge'] = async (el) => {
  const ok = await dialog({ title: 'Borrar para siempre', danger: true, confirm: 'Borrar para siempre',
    body: `<p>Se borra <b>${esc(el.dataset.name)}</b> (${fmtBytes(+el.dataset.size)}) con su mundo, mods, configuración y copias.</p><p><b>No se va a poder recuperar.</b></p>` });
  if (!ok) return;
  try { await invoke('trash_purge', { key: el.dataset.key }); toast('Borrado para siempre'); paintTrash(); } catch (e) { fail(e); }
};

A['trash-empty'] = async () => {
  let items = [];
  try { items = await invoke('trash_list', {}); } catch (e) { fail(e); return; }
  if (!items.length) return;
  const total = items.reduce((a, b) => a + b.sizeBytes, 0);
  const ok = await dialog({ title: 'Vaciar la Papelera', danger: true, confirm: `Borrar ${items.length === 1 ? '1 servidor' : `${items.length} servidores`}`,
    body: `<p>Se borran para siempre:</p><ul class="sm" style="margin:6px 0 10px;padding-left:18px">${items.map((i) => `<li>${esc(i.meta.name)} · ${fmtBytes(i.sizeBytes)}</li>`).join('')}</ul><p>Se liberan <b>${fmtBytes(total)}</b>. <b>No se van a poder recuperar.</b></p>` });
  if (!ok) return;
  try {
    const r = await invoke('trash_empty', {});
    if (r.errors.length) toast(`Se borraron ${r.removed}; ${r.errors.length} no se pudieron borrar`, 'error'); else toast(`Papelera vacía · ${fmtBytes(r.freedBytes)} liberados`);
    paintTrash();
  } catch (e) { fail(e); }
};
