'use strict';
/* 939 Host — Resource packs del servidor: instalados (obligatorio u opcional, activar, reemplazar,
   quitar), búsqueda en Modrinth y archivos propios. Lo obligatorio lo envía el propio Minecraft al
   entrar; lo opcional lo instala «Preparar para jugar» en el Minecraft de cada jugador. */

const rpS = { forId: null, sub: 'installed', lib: null, q: '', all: false, results: null, offset: 0, busy: {}, error: null };

tabDraw.packs = async (el) => {
  const id = state.server.config.id;
  if (rpS.forId !== id) Object.assign(rpS, { forId: id, sub: 'installed', lib: null, q: '', all: false, results: null, offset: 0, busy: {}, error: null });
  el.innerHTML = '<div id="rp-root" class="stack" style="gap:16px"><div class="card pad"><div class="bar indet"><i></i></div></div></div>';
  await rpLoad();
  rpPaint();
};

async function rpLoad() {
  try { rpS.lib = await invoke('rp_library', { id: rpS.forId }); rpS.error = null; }
  catch (e) { rpS.error = errMsg(e); }
}

const RP_COMPAT = { incompatible: 'Otra versión', desconocida: 'Versión sin comprobar', invalido: 'Archivo inválido' };

function rpRow(p, L) {
  const off = !p.active;
  const where = off ? 'Desactivado: no lo recibe nadie.'
    : p.mode === 'obligatorio' ? 'Lo envía el servidor al entrar. Quien lo rechaza no puede jugar.'
    : 'Lo recibe cada jugador con «Preparar para jugar». Cada uno lo puede desactivar.';
  const dis = L.editable ? '' : 'disabled';
  return `<div class="li rp-li${off ? ' off' : ''}">${modIcon(p.icon, p.title)}
    <div class="mod-main"><span class="t">${esc(p.title)}${p.versionName ? `<span class="mono xs t3">${esc(p.versionName)}</span>` : ''}
      ${p.active ? `<span class="chip s${p.mode === 'obligatorio' ? ' fill' : ''}">${p.mode === 'obligatorio' ? 'Obligatorio' : 'Opcional'}</span>` : '<span class="chip s">Desactivado</span>'}
      ${RP_COMPAT[p.compat] ? `<span class="chip s" title="${esc(p.compatDetail)}">${RP_COMPAT[p.compat]}</span>` : ''}</span>
      ${p.description ? `<span class="d">${esc(p.description)}</span>` : ''}
      <span class="xs t3">${p.source === 'modrinth' ? 'Modrinth' : 'Archivo propio'} · ${fmtBytes(p.size)} · ${esc(where)}</span>
      ${p.compat !== 'ok' ? `<span class="xs t3">${esc(p.compatDetail)}</span>` : ''}
      ${p.warnings.map((w) => `<span class="xs rp-warn">${icon('alert', 12)}${esc(w)}</span>`).join('')}</div>
    <div class="btns rp-acts">
      <div class="seg s" role="group" aria-label="Para los jugadores">${[['obligatorio', 'Obligatorio'], ['opcional', 'Opcional']].map(([k, l]) => `<button class="${p.mode === k ? 'on' : ''}" data-a="rp-mode" data-f="${esc(p.file)}" data-v="${k}" ${dis || (p.compat === 'invalido' ? 'disabled' : '')}>${l}</button>`).join('')}</div>
      <label class="switch rp-sw" title="${p.active ? 'Desactivar' : 'Activar'}"><input type="checkbox" data-rp-active="${esc(p.file)}" ${p.active ? 'checked' : ''} ${dis} aria-label="Activar ${esc(p.title)}"></label>
      ${p.source !== 'modrinth' ? `<button class="btn s icon ghost" data-a="rp-replace" data-f="${esc(p.file)}" title="Reemplazar por otro archivo" aria-label="Reemplazar ${esc(p.title)}" ${dis}>${icon('refresh', 15)}</button>` : ''}
      <button class="btn s icon ghost" data-a="rp-remove" data-f="${esc(p.file)}" data-t="${esc(p.title)}" title="Quitar" aria-label="Quitar ${esc(p.title)}" ${dis}>${icon('trash', 15)}</button>
    </div></div>`;
}

function rpPaint() {
  const root = $('#rp-root');
  if (!root) return;
  if (rpS.error) { root.innerHTML = `<div class="issue">${icon('alert', 18)}<div class="sm">${esc(rpS.error)}</div><button class="btn s" data-a="rp-reload">Reintentar</button></div>`; return; }
  const L = rpS.lib;
  const req = L.packs.find((p) => p.active && p.mode === 'obligatorio');
  root.innerHTML = `
    ${L.readOnlyReason ? `<div class="note">${icon('info')}<div class="grow"><span class="sm t2">${esc(L.readOnlyReason)}</span></div></div>` : ''}
    ${L.localProblem && req && req.source !== 'modrinth' ? `<div class="note">${icon('alert')}<div class="grow"><b>Esta PC no puede entregar el pack propio</b><span class="sm t2">${esc(L.localProblem)} Cerrá el otro programa y volvé a abrir 939 Host.</span></div></div>` : ''}
    ${L.manualPack ? `<div class="note">${icon('info')}<div class="grow"><b>El servidor ya tiene un resource pack puesto a mano</b><span class="sm t2 mono ellipsis">${esc(L.manualPack)}</span><span class="sm t2">Si marcás un pack como obligatorio, ese se reemplaza mientras lo sea; al quitarlo, vuelve el anterior.</span></div></div>` : ''}
    ${L.running ? `<div class="note">${icon('info')}<div class="grow"><span class="sm t2">El servidor está encendido: lo que envía al entrar cambia al reiniciarlo.</span></div></div>` : ''}
    <div class="spread" style="flex-wrap:wrap"><div class="seg" role="tablist">${[['installed', 'Instalados'], ['explore', 'Buscar en Modrinth']].map(([k, l]) => `<button class="${rpS.sub === k ? 'on' : ''}" data-a="rp-sub" data-v="${k}" role="tab" aria-selected="${rpS.sub === k}">${l}${k === 'installed' && L.packs.length ? ` <span class="t3">${L.packs.length}</span>` : ''}</button>`).join('')}</div>
      <div class="btns"><span class="chip soft">Minecraft ${esc(L.mcVersion)}${L.serverFormat ? ` · formato ${L.serverFormat}` : ''}</span>
        <button class="btn s" data-a="rp-add" ${L.editable ? '' : 'disabled'}>${icon('plus', 14)}Agregar archivo .zip</button>
        <button class="btn s ghost" data-a="rp-folder" ${L.editable ? '' : 'disabled'}>${icon('folder', 14)}Carpeta</button></div></div>
    <div id="rp-sub"></div>`;
  const sub = $('#rp-sub');
  if (rpS.sub === 'explore') { rpPaintExplore(sub); return; }
  sub.innerHTML = `<div class="grid2" style="grid-template-columns:minmax(0,1.6fr) minmax(0,1fr)">
    <div class="card"><div class="card-head"><h2>Resource packs del servidor</h2><span class="sm t3">${L.packs.length}</span></div>
      ${L.packs.length ? `<div class="list">${L.packs.map((p) => rpRow(p, L)).join('')}</div>`
        : `<div class="pad sm t2">Todavía no hay resource packs. Buscalos en <button class="link" data-a="rp-sub" data-v="explore">Modrinth</button> o agregá un .zip propio.</div>`}</div>
    <div class="stack" style="gap:16px">
      <div class="card pad stack" style="gap:10px"><h3>Qué reciben los jugadores</h3>
        <div class="sm"><b>Obligatorio</b> <span class="t2">— lo envía el servidor al entrar (Minecraft admite uno solo). Quien lo rechaza no puede jugar. Si el pack cambia, cada jugador baja la versión nueva; si no cambió, no se vuelve a bajar.</span></div>
        <div class="sm"><b>Opcional</b> <span class="t2">— no se exige. «Preparar para jugar» lo instala y lo activa en el Minecraft de cada jugador, que lo puede desactivar.</span></div>
        <div class="xs t3">Un pack de Modrinth se baja de Modrinth. Un archivo propio no se publica en ningún lado: cada 939 Host lo trae de la PC anfitriona por el grupo. Quien entra por IP sin 939 Host solo puede recibir packs de Modrinth.</div>
      </div>
      <div class="card pad stack" style="gap:6px"><h3>En el servidor</h3><span class="sm t2">${req ? `Envía <b>${esc(req.title)}</b> como obligatorio.` : 'No envía ningún pack al entrar.'}</span>
        <span class="xs t3">Los archivos están en la carpeta <span class="mono">resourcepacks</span> del servidor (también en Archivos).</span></div>
    </div></div>`;
  sub.querySelectorAll('[data-rp-active]').forEach((i) => i.addEventListener('change', () => rpSet(i.dataset.rpActive, { active: i.checked })));
}

function rpPaintExplore(el) {
  const r = rpS.results;
  el.innerHTML = `<div class="stack">
    <div class="row wrap" style="gap:10px"><div class="search grow" style="min-width:240px">${icon('search', 16)}<input id="rp-q" placeholder="Buscar resource packs en Modrinth (por ejemplo: faithful, interfaz oscura)" value="${esc(rpS.q)}" aria-label="Buscar resource packs"></div>
      <label class="switch rp-sw" style="gap:8px;align-items:center"><span class="sm">Incluir otras versiones</span><input type="checkbox" id="rp-all" ${rpS.all ? 'checked' : ''}></label></div>
    <p class="xs t3" style="margin:0">${rpS.all ? 'Se muestran también packs hechos para otras versiones: se pueden usar, pero pueden verse mal.' : `Solo packs que declaran Minecraft ${esc(rpS.lib.mcVersion)}.`}</p>
    <div class="card" id="rp-results">${!r ? '<div class="pad"><div class="bar indet"><i></i></div></div>'
      : r.error ? `<div class="pad sm t2">${esc(r.error)}</div>`
      : r.hits.length ? `<div class="list">${r.hits.map(rpHit).join('')}</div>${r.total > r.hits.length ? `<div class="pad" style="text-align:center"><button class="btn s" data-a="rp-more">Ver más · ${r.hits.length} de ${fmtNum(r.total)}</button></div>` : ''}`
        : '<div class="pad sm t2">No hay resultados.</div>'}</div></div>`;
  const q = $('#rp-q');
  let t = null;
  q?.addEventListener('input', () => { clearTimeout(t); t = setTimeout(() => { rpS.q = q.value; rpSearch(); }, 350); });
  $('#rp-all')?.addEventListener('change', (e) => { rpS.all = e.target.checked; rpSearch(); });
}

function rpHit(h) {
  const inst = rpS.lib.packs.some((p) => p.source === 'modrinth' && p.title === h.title);
  const busy = rpS.busy[h.projectId];
  return `<div class="li">${modIcon(h.iconUrl, h.title)}
    <div class="mod-main"><span class="t">${esc(h.title)}${h.compatible ? '' : '<span class="chip s">Otra versión</span>'}</span>
      <span class="d">${esc(h.description)}</span><span class="xs t3">${fmtNum(h.downloads)} descargas · ${esc(h.versions.length > 3 ? `${h.versions[0]} … ${h.versions[h.versions.length - 1]}` : h.versions.join(', '))}</span></div>
    ${busy ? '<span class="chip">Descargando…</span>' : inst ? '<span class="chip">Instalado</span>' : `<button class="btn s" data-a="rp-install" data-p="${esc(h.projectId)}" data-t="${esc(h.title)}" data-c="${h.compatible ? 1 : 0}" ${rpS.lib.editable ? '' : 'disabled'}>${icon('download', 14)}Instalar</button>`}</div>`;
}

async function rpSearch(more = false) {
  const offset = more ? rpS.offset + 20 : 0;
  const seq = (rpSearch.seq = (rpSearch.seq || 0) + 1);
  if (!more) { rpS.results = null; if (rpS.sub === 'explore') rpPaintExplore($('#rp-sub')); }
  try {
    const r = await invoke('rp_search', { id: rpS.forId, query: rpS.q.trim(), all: rpS.all, offset });
    if (seq !== rpSearch.seq) return;
    rpS.offset = offset;
    rpS.results = more && rpS.results ? { ...r, hits: [...rpS.results.hits, ...r.hits] } : r;
  } catch (e) { rpS.results = { hits: [], total: 0, error: `No se pudo buscar en Modrinth: ${errMsg(e)}` }; }
  if (rpS.sub === 'explore' && $('#rp-sub')) { const q = $('#rp-q'); const pos = q?.selectionStart; rpPaintExplore($('#rp-sub')); const q2 = $('#rp-q'); if (q && q2 && document.activeElement !== q2 && rpS.q) { q2.focus(); q2.setSelectionRange(pos, pos); } }
}

function rpDone(r, what) {
  toast(what, 'ok', r.restartNeeded ? 'El servidor está encendido: lo que envía al entrar cambia al reiniciarlo.' : (r.demoted ? `«${r.demoted}» pasó a opcional: solo puede haber un pack obligatorio.` : undefined));
}

async function rpSet(file, change) {
  const L = rpS.lib;
  const p = L.packs.find((x) => x.file === file);
  if (change.mode === 'obligatorio' && L.manualPack) {
    if (!(await dialog({ title: 'Reemplazar el pack puesto a mano', confirm: 'Usar este pack', body: `<p>El servidor ya envía un resource pack configurado a mano:</p><p class="mono sm">${esc(L.manualPack)}</p><p>Mientras <b>${esc(p.title)}</b> sea obligatorio, se envía este. Si después lo quitás o lo dejás opcional, vuelve el anterior.</p>` }))) { rpPaint(); return; }
  }
  if (change.mode === 'obligatorio' && p.source !== 'modrinth') {
    if (!(await dialog({ title: 'Pack propio obligatorio', confirm: 'Hacerlo obligatorio', body: `<p>Cada jugador que entra con 939 Host lo recibe por el grupo, sin publicarlo en internet.</p><p>Quien entre por IP <b>sin</b> 939 Host no puede bajarlo y no va a poder entrar. Si necesitás que entren sin 939 Host, usá un pack de Modrinth.</p>` }))) { rpPaint(); return; }
  }
  try {
    const r = await invoke('rp_set', { id: rpS.forId, file, mode: change.mode || null, active: change.active ?? null });
    await rpLoad(); rpPaint();
    rpDone(r, change.mode ? `${p.title}: ${change.mode}` : `${p.title}: ${change.active ? 'activado' : 'desactivado'}`);
  } catch (e) { fail(e); await rpLoad(); rpPaint(); }
}

async function rpAddPath(path, onConflict = '') {
  try {
    const r = await invoke('rp_add_file', { id: rpS.forId, path, onConflict });
    await rpLoad(); rpS.sub = 'installed'; rpPaint();
    rpDone(r, `Agregado: ${r.file}`);
  } catch (e) {
    if (e?.code === 'exists') {
      const c = await dialog({ title: 'Ya hay un pack con ese nombre', cancel: 'Cancelar', body: `<p>${esc(errMsg(e))}</p><p>Podés reemplazarlo (el actual va a la Papelera y conserva si es obligatorio u opcional) o guardar los dos.</p>`,
        buttons: [{ label: 'Guardar los dos', value: 'ambos' }, { label: 'Reemplazar', value: 'reemplazar', kind: 'pri' }] });
      if (c === 'ambos' || c === 'reemplazar') return rpAddPath(path, c);
      return;
    }
    fail(e);
  }
}

A['rp-reload'] = async () => { await rpLoad(); rpPaint(); };
A['rp-sub'] = (el) => { rpS.sub = el.dataset.v; rpPaint(); if (rpS.sub === 'explore' && !rpS.results) rpSearch(); };
A['rp-more'] = () => rpSearch(true);
A['rp-mode'] = (el) => { const p = rpS.lib.packs.find((x) => x.file === el.dataset.f); if (p && p.mode !== el.dataset.v) rpSet(el.dataset.f, { mode: el.dataset.v }); };
A['rp-folder'] = async () => { try { await invoke('rp_open_folder', { id: rpS.forId }); await rpLoad(); rpPaint(); } catch (e) { fail(e); } };
A['rp-add'] = async () => {
  const p = await TAURI.dialog.open({ multiple: false, title: 'Elegí un resource pack (.zip)', filters: [{ name: 'Resource pack', extensions: ['zip'] }] });
  if (p) rpAddPath(p);
};
A['rp-replace'] = async (el) => {
  const old = rpS.lib.packs.find((x) => x.file === el.dataset.f);
  const p = await TAURI.dialog.open({ multiple: false, title: `Elegí el archivo que reemplaza a «${old.title}»`, filters: [{ name: 'Resource pack', extensions: ['zip'] }] });
  if (!p) return;
  if (!(await dialog({ title: `Reemplazar «${old.title}»`, confirm: 'Reemplazar', body: '<p>El archivo actual va a la Papelera de 939 Host (se puede restaurar). El nuevo conserva si es obligatorio u opcional. Si es obligatorio, cada jugador baja el nuevo al entrar.</p>' }))) return;
  try {
    // El anterior va a la Papelera; el nuevo hereda si era obligatorio u opcional y si estaba activo.
    const r = await invoke('rp_remove', { id: rpS.forId, file: old.file });
    const added = await invoke('rp_add_file', { id: rpS.forId, path: p, onConflict: 'ambos' });
    if (old.mode === 'obligatorio' || !old.active) await invoke('rp_set', { id: rpS.forId, file: added.file, mode: old.mode, active: old.active });
    await rpLoad(); rpPaint();
    rpDone({ ...added, restartNeeded: r.restartNeeded || added.restartNeeded }, `Reemplazado: ${added.file}`);
  } catch (e) { fail(e); await rpLoad(); rpPaint(); }
};
A['rp-remove'] = async (el) => {
  const p = rpS.lib.packs.find((x) => x.file === el.dataset.f);
  if (!(await dialog({ title: `Quitar «${p.title}»`, danger: true, confirm: 'Quitar', body: `<p>El archivo va a la Papelera de 939 Host (se puede restaurar durante 30 días).${p.mode === 'obligatorio' && p.active ? ' El servidor deja de enviarlo al entrar.' : ''}</p>` }))) return;
  try { const r = await invoke('rp_remove', { id: rpS.forId, file: p.file }); await rpLoad(); rpPaint(); rpDone(r, `Quitado: ${p.title}`); } catch (e) { fail(e); }
};
A['rp-install'] = async (el) => {
  const project = el.dataset.p;
  let allow = false;
  if (el.dataset.c !== '1') {
    if (!(await dialog({ title: 'Hecho para otra versión', confirm: 'Instalar igual', body: `<p><b>${esc(el.dataset.t)}</b> no declara Minecraft ${esc(rpS.lib.mcVersion)}. Minecraft lo marca como incompatible pero deja usarlo; algunas texturas pueden verse mal.</p>` }))) return;
    allow = true;
  }
  rpS.busy[project] = true; rpPaintExplore($('#rp-sub'));
  try {
    const r = await invoke('rp_install', { id: rpS.forId, project, allowIncompatible: allow });
    delete rpS.busy[project];
    await rpLoad(); rpPaintExplore($('#rp-sub'));
    rpDone(r, `Instalado: ${el.dataset.t}`);
  } catch (e) {
    delete rpS.busy[project];
    if (e?.code === 'incompatible' && !allow) {
      if (await dialog({ title: 'No hay versión para este Minecraft', confirm: 'Instalar la más nueva', body: `<p>${esc(errMsg(e))}</p><p>Se puede instalar igual; Minecraft lo marca como incompatible y puede verse mal.</p>` })) { el.dataset.c = '1'; rpS.busy[project] = true; try { const r = await invoke('rp_install', { id: rpS.forId, project, allowIncompatible: true }); rpDone(r, `Instalado: ${el.dataset.t}`); } catch (e2) { fail(e2); } delete rpS.busy[project]; await rpLoad(); }
    } else fail(e);
    rpPaintExplore($('#rp-sub'));
  }
};
