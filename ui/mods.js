'use strict';
/* Mods: Instalados, Explorar Modrinth (categorías y lado), Actualizaciones, Archivos agregados a
   mano, página de detalle e importación arrastrando o eligiendo .jar. */

const MSUB = [['installed', 'Instalados'], ['explore', 'Explorar Modrinth'], ['curseforge', 'CurseForge'], ['modpacks', 'Modpacks'], ['updates', 'Actualizaciones']];
const CATS = [['', 'Todas'], ['optimization', 'Optimización'], ['utility', 'Utilidades'], ['library', 'Bibliotecas'], ['game-mechanics', 'Mecánicas'], ['adventure', 'Aventura'],
  ['worldgen', 'Generación de mundo'], ['mobs', 'Criaturas'], ['magic', 'Magia'], ['technology', 'Tecnología'], ['storage', 'Almacenamiento'], ['equipment', 'Equipamiento'],
  ['decoration', 'Decoración'], ['food', 'Comida'], ['transportation', 'Transporte'], ['management', 'Administración'], ['social', 'Social'], ['economy', 'Economía'], ['minigame', 'Minijuegos']];
const SIDE = { server: 'Solo servidor', both: 'Servidor y jugadores', client: 'Solo jugadores', unknown: 'Lado desconocido' };
const ISSUE = { duplicate: 'Duplicado', incompatible: 'Otra versión de Minecraft', client_only: 'Solo para jugadores', unknown: 'Local' };
const md = { req: null, auto: false, sub: 'installed', list: [], offline: false, updates: {}, perf: [], q: '', cat: '', side: 'server', sort: 'relevance', results: null, offset: 0, detail: null, busy: false, loadedFor: null };
const sideOf = (cl, sv) => (sv === 'unsupported' ? 'client' : cl === 'unsupported' ? 'server' : 'both');
const imgs = () => state.info.settings.modImagesEnabled !== false;
const modIcon = (url, title, cls = '') => `<span class="mod-ic ${cls}">${url && imgs() ? `<img src="${esc(url)}" alt="" loading="lazy" referrerpolicy="no-referrer">` : esc((title || '?')[0].toUpperCase())}</span>`;

tabDraw.mods = async (el) => {
  const c = state.server.config;
  if (c.loader === 'vanilla') {
    el.innerHTML = `<div class="empty">${icon('box', 28)}<h2>Este servidor es Vanilla</h2><p>Los mods necesitan Fabric, NeoForge o Forge. Creá un servidor con uno de ellos (podés usar una copia de este mismo mundo) para instalar mods.</p><button class="btn pri" data-a="new">${icon('plus', 16)}Crear servidor</button></div>`;
    return;
  }
  if (md.loadedFor !== c.id) { Object.assign(md, { sub: 'installed', req: null, results: null, q: '', cat: '', updates: {}, detail: null, offset: 0 }); md.loadedFor = c.id; }
  if (md.goSub) { md.sub = md.goSub; md.goSub = null; md.detail = null; }
  el.innerHTML = '<div id="mods-root" class="stack" style="gap:16px"><div class="card pad"><div class="bar indet"><i></i></div></div></div>';
  await loadMods();
  if (md.sub === 'files') md.sub = 'installed';
  paintMods();
  // «Buscar actualizaciones compatibles automáticamente»: se consulta en segundo plano.
  if (md.auto && !md.offline && !md.checked) {
    invoke('mods_updates', { id: c.id }).then((u) => { md.updates = u; md.checked = true; if (state.route === 'server' && state.tab === 'mods' && md.loadedFor === c.id && !md.detail) paintMods(); }).catch(() => {});
  }
  // Las solicitudes se cargan aparte: el análisis (y la consulta a Modrinth) no frena la pestaña.
  loadRequests().then(() => { if (state.route === 'server' && state.tab === 'mods' && md.loadedFor === c.id) paintMods(); });
};
async function loadMods() {
  const id = state.params.id;
  md.auto = (await invoke('mods_settings', { id }).catch(() => ({ autoUpdates: false }))).autoUpdates;
  try { md.list = await invoke('mods_list', { id, online: true }); md.offline = false; }
  catch { md.list = await invoke('mods_list', { id, online: false }).catch(() => []); md.offline = true; }
}
function paintMods() {
  const root = $('#mods-root');
  if (!root) return;
  if (md.detail) { paintDetail(root); return; }
  const counts = { installed: md.list.length, updates: Object.keys(md.updates).length, requests: (md.req?.list || []).filter((r) => !r.rejected).length };
  const subs = state.share?.enabled ? [['requests', 'Solicitudes'], ...MSUB] : MSUB;
  if (md.sub === 'requests' && !state.share?.enabled) md.sub = 'installed';
  root.innerHTML = `
    ${md.offline ? `<div class="note">${icon('alert')}<div class="grow"><b>Sin conexión con Modrinth</b><span class="sm t2">Se muestran los archivos locales. Para buscar, instalar o identificar mods hace falta internet.</span></div><button class="btn s" data-a="mods-reload">${icon('refresh', 14)}Reintentar</button></div>` : ''}
    <div class="spread" style="flex-wrap:wrap"><div class="seg" role="tablist">${subs.map(([k, l]) => `<button class="${md.sub === k ? 'on' : ''}" data-a="msub" data-v="${k}" role="tab" aria-selected="${md.sub === k}">${l}${counts[k] ? ` <span class="t3">${counts[k]}</span>` : ''}</button>`).join('')}</div>
      <div class="btns"><span class="chip soft">${versionLabel(state.server.config)}</span><button class="btn s ghost" data-a="folder" data-which="mods">${icon('folder', 14)}Carpeta de mods</button></div></div>
    <div id="mods-progress"></div>
    <div id="mods-sub"></div>`;
  const sub = $('#mods-sub');
  if (md.sub === 'requests') paintRequests(sub);
  else if (md.sub === 'installed') paintInstalled(sub);
  else if (md.sub === 'explore') paintExplore(sub);
  else if (md.sub === 'curseforge') paintCurseForge(sub);
  else paintUpdates(sub);
}
A.msub = (el) => { if (el.dataset.v === 'modpacks') { Object.assign(mp, { target: state.params.id, detail: null, results: null, src: 'modrinth' }); go('modpack'); return; } md.sub = el.dataset.v; paintMods(); if (md.sub === 'explore' && !md.results && !md.offline) runSearch(); };
A['mods-reload'] = async () => { await loadMods(); paintMods(); };

/* ── Solicitudes: mods que llegaron de otra PC y esperan la decisión de la propietaria ── */
const RSTATUS = {
  sin_alertas: 'Sin alertas conocidas', desconocido: 'Información insuficiente', precaucion: 'Precaución',
  sospechoso: 'Sospechoso', amenaza: 'Amenaza confirmada', incompleto: 'Análisis incompleto',
};
async function loadRequests() {
  if (!state.share?.enabled) { md.req = null; return; }
  const owner = state.share.group?.members?.find((m) => m.isMe)?.role === 'propietario';
  md.req = { ...(md.req || { list: [] }), loading: true, owner };
  try { md.req = { list: await invoke('mod_requests', { id: state.params.id }), owner, loading: false }; }
  catch (e) { md.req = { list: [], owner, loading: false, error: e?.message || String(e) }; }
}
function reqCard(r, owner) {
  const a = r.analysis;
  const title = a?.meta?.name || r.name;
  const st = a ? a.status : 'incompleto';
  const strong = st === 'sospechoso' || st === 'amenaza';
  const alerts = a ? a.findings.filter((f) => f.level !== 'info') : [];
  const when = r.at ? new Date(r.at).toLocaleString('es-AR', { dateStyle: 'short', timeStyle: 'short' }) : '';
  return `<div class="card pad stack${strong ? ' note strong' : ''}" style="gap:10px">
    <div class="spread" style="align-items:flex-start;gap:12px"><div class="grow" style="min-width:0">
      <b>${esc(title)}</b>${a?.meta?.version ? ` <span class="t2">${esc(a.meta.version)}</span>` : ''}
      <div class="sm t2">${esc(r.name)}${r.size ? ` · ${fmtBytes(r.size)}` : ''} · de ${esc(r.from)}${when ? ` · ${esc(when)}` : ''}</div>
    </div><span class="chip${strong ? ' fill' : ' soft'}">${esc(r.rejected ? 'Rechazado' : RSTATUS[st] || st)}</span></div>
    ${r.here ? `<p class="sm" style="margin:0">${esc(a ? a.summary : 'No se pudo analizar el archivo. Que no haya alertas no significa nada.')}</p>`
      : `<p class="sm" style="margin:0">Está apartado en ${esc(r.heldBy || 'otra PC')}. Se analiza cuando llegue a esta PC; un análisis hecho por otra PC no se puede verificar.</p>`}
    ${alerts.length ? `<ul class="sm" style="margin:0;padding-left:18px">${alerts.map((f) => `<li>${esc(f.title)}</li>`).join('')}</ul>` : ''}
    ${a?.modrinth ? `<div class="sm">Publicado en Modrinth: <a href="#" data-a="req-link" data-url="${esc(a.modrinth.url)}">${esc(a.modrinth.title || a.modrinth.project)}</a>${a.modrinth.version ? ` · versión ${esc(a.modrinth.version)}` : ''}</div>` : ''}
    ${a?.meta?.depends?.length ? `<div class="sm t2">Necesita: ${a.meta.depends.map(esc).join(', ')}</div>` : ''}
    <details><summary class="xs t3" style="cursor:pointer">Detalles técnicos</summary><div class="tech" style="margin-top:6px">${esc([
      `SHA-256: ${r.sha256}`,
      a ? `Tamaño: ${a.size} bytes · loader: ${a.meta.loader || 'no identificado'} · id: ${a.meta.id || '—'}` : '',
      ...(a ? a.findings.map((f) => `[${f.level}] ${f.title}${f.detail ? ` — ${f.detail}` : ''}`) : []),
      ...(a?.nested?.length ? [`Jars incluidos: ${a.nested.join(', ')}`] : []),
      ...(a ? a.checked.map((c) => `Revisado: ${c}`) : []),
      ...(a ? a.unchecked.map((c) => `No revisado: ${c}`) : []),
      'El análisis no ejecuta el mod. Solo busca indicadores conocidos: no detecta todo.',
    ].filter(Boolean).join('\n'))}</div></details>
    <div class="btns">
      ${owner ? `<button class="btn s${strong ? '' : ' pri'}" data-a="req-approve" data-h="${esc(r.sha256)}" data-st="${esc(st)}">${r.rejected ? 'Aprobar de todos modos' : 'Aprobar'}</button>` : ''}
      ${owner && !r.rejected ? `<button class="btn s" data-a="req-reject" data-h="${esc(r.sha256)}">Rechazar</button>` : ''}
      ${owner && r.here ? `<button class="btn s ghost" data-a="req-reject" data-h="${esc(r.sha256)}" data-discard="1">${r.rejected ? 'Borrar archivo' : 'Rechazar y borrar'}</button>` : ''}
      <button class="btn s ghost" data-a="req-link" data-url="https://www.virustotal.com/gui/file/${esc(r.sha256)}" title="Abre la página de VirusTotal de esta huella en tu navegador. No sube el archivo.">${icon('external', 14)}VirusTotal</button>
      <button class="btn s ghost" data-a="req-copy" data-h="${esc(r.sha256)}">${icon('copy', 14)}Copiar huella</button>
    </div></div>`;
}
function paintRequests(el) {
  const R = md.req;
  if (!R || (R.loading && !R.list.length)) { el.innerHTML = '<div class="card pad stack"><div class="bar indet"><i></i></div><p class="sm t2" style="margin:0">Analizando los mods recibidos (sin ejecutarlos)…</p></div>'; return; }
  const pend = R.list.filter((r) => !r.rejected);
  const rej = R.list.filter((r) => r.rejected);
  el.innerHTML = `<div class="stack" style="gap:12px">
    <div class="note">${icon('shield')}<div class="grow"><b>Los mods que llegan de otra PC no se cargan sin tu aprobación</b><span class="sm t2">Quedan apartados en una cuarentena. Un mod es un programa que corre con los permisos de Windows de la PC que aloja el servidor: aprobalo solo si sabés de dónde salió. El análisis ayuda, pero no garantiza nada.${R.owner ? '' : ' Solo la PC propietaria puede aprobar o rechazar.'}</span></div></div>
    ${R.error ? `<div class="note">${icon('alert')}<div class="grow"><b>No se pudieron cargar las solicitudes</b><span class="sm t2">${esc(R.error)}</span></div></div>` : ''}
    ${pend.length ? pend.map((r) => reqCard(r, R.owner)).join('') : '<div class="card pad"><p class="sm t2" style="margin:0">No hay mods esperando aprobación.</p></div>'}
    ${rej.length ? `<h3 style="margin:8px 0 0">Rechazados (${rej.length})</h3><p class="sm t2" style="margin:0">No se cargan nunca en ninguna PC del grupo.</p>${rej.map((r) => reqCard(r, R.owner)).join('')}` : ''}
    ${R.owner ? `<details id="req-log"><summary class="sm" style="cursor:pointer">Registro de decisiones</summary><div id="req-log-body" class="sm t2" style="margin-top:8px">Cargando…</div></details>` : ''}
  </div>`;
  const log = $('#req-log');
  if (log) log.addEventListener('toggle', async () => {
    if (!log.open) return;
    const v = await invoke('mod_decisions', { id: state.params.id }).catch(() => []);
    const L = { aprobado: 'Aprobado', rechazado: 'Rechazado', descartado: 'Archivo borrado', propio: 'Instalado en esta PC (aprobado solo)' };
    $('#req-log-body').innerHTML = v.length ? `<div>${v.slice(0, 100).map((d) => `<div class="kv"><span>${esc(L[d.decision] || d.decision)} · <b>${esc(d.name || d.sha256.slice(0, 12))}</b>${d.analysis ? ` <span class="t3">(${esc(RSTATUS[d.analysis] || d.analysis)})</span>` : ''}</span><span class="xs t3">${esc(new Date(d.at).toLocaleString('es-AR', { dateStyle: 'short', timeStyle: 'short' }))}</span></div>`).join('')}</div>` : 'Todavía no hay decisiones.';
  });
}
A['req-approve'] = async (el) => {
  const st = el.dataset.st;
  const danger = st === 'sospechoso' || st === 'amenaza';
  const body = danger
    ? `<p><b>El análisis encontró indicadores ${st === 'amenaza' ? 'de malware conocido' : 'que no son normales en un mod'}.</b> Si lo aprobás, se va a ejecutar en la PC que aloje el servidor con los permisos de su usuario de Windows.</p><p>Aprobalo solo si sabés exactamente por qué los necesita.</p>`
    : '<p>Se va a cargar en la PC que aloje el servidor, con los permisos de su usuario de Windows. La aprobación vale solo para este archivo exacto: si cambia un byte, hay que volver a aprobarlo.</p>';
  if (!(await dialog({ title: danger ? 'Aprobar un mod con alertas' : 'Aprobar mod', confirm: danger ? 'Aprobar de todos modos' : 'Aprobar', body }))) return;
  try { await invoke('mods_approve', { id: state.params.id, hashes: [el.dataset.h] }); toast('Mod aprobado: se carga en el próximo encendido'); } catch (e) { fail(e); }
  await loadRequests(); paintMods();
};
A['req-reject'] = async (el) => {
  const discard = !!el.dataset.discard;
  if (!(await dialog({ title: discard ? 'Rechazar y borrar' : 'Rechazar mod', confirm: discard ? 'Rechazar y borrar' : 'Rechazar', body: `<p>No se va a cargar nunca en ninguna PC del grupo${discard ? ', y el archivo se borra de esta PC' : ''}. Si llega otra vez el mismo archivo, sigue rechazado.</p>` }))) return;
  try { await invoke('mods_reject', { id: state.params.id, hashes: [el.dataset.h], discard }); toast(discard ? 'Mod rechazado y borrado' : 'Mod rechazado'); } catch (e) { fail(e); }
  await loadRequests(); paintMods();
};
A['req-link'] = async (el) => { try { await invoke('open_link', { url: el.dataset.url }); } catch (e) { fail(e); } };
A['req-copy'] = async (el) => { try { await navigator.clipboard.writeText(el.dataset.h); toast('Huella copiada'); } catch { /* sin portapapeles */ } };

function dropZone() {
  return `<div class="drop" id="mods-drop">${icon('upload', 22)}<div class="grow"><b>Agregar mods desde archivos</b><div class="sm t2">Desde cualquier carpeta de tu PC: arrastrá archivos .jar a esta ventana o elegilos. No hace falta que estén en Modrinth o CurseForge. Antes de copiarlos se revisa que sean para ${loaderName(state.server.config.loader)} ${esc(state.server.config.mcVersion)}; los mods solo para jugadores se omiten. Los originales no se tocan.</div></div>
    <div class="btns"><button class="btn s" data-a="mods-pick">Elegir archivos…</button><button class="btn s ghost" data-a="mods-import-folder">Importar carpeta…</button></div></div>`;
}
function modRow(x) {
  const local = !x.managed;
  const where = x.origin === 'optimizacion' ? 'Rendimiento' : local ? (x.issues.includes('unknown') || !x.projectId ? 'Mod local: no está en ningún catálogo; funciona igual, sin actualizaciones automáticas' : 'Agregado desde tu PC') : x.explicit ? 'Instalado por vos' : 'Dependencia';
  const issues = x.issues.filter((i) => !(local && i === 'unknown'));
  const removeBtn = local
    ? `<button class="btn s icon ghost" data-a="mod-aside" data-file="${esc(x.fileName)}" title="Quitar" aria-label="Quitar ${esc(x.title)}">${icon('trash', 15)}</button>`
    : `<button class="btn s icon ghost" data-a="mod-remove" data-project="${esc(x.projectId)}" data-title="${esc(x.title)}" ${x.requiredBy.length ? 'disabled title="Otro mod lo necesita"' : 'title="Quitar"'} aria-label="Quitar ${esc(x.title)}">${icon('trash', 15)}</button>`;
  return `<div class="li">${modIcon(null, x.title, 'sm')}
    <div class="mod-main"><span class="t">${x.projectId && !local ? `<button class="link" data-a="mod-open" data-project="${esc(x.projectId)}">${esc(x.title)}</button>` : esc(x.title)}${x.versionNumber ? `<span class="mono xs t3">${esc(x.versionNumber)}</span>` : ''}${local && !x.projectId ? '<span class="chip s">Local</span>' : ''}${issues.map((i) => `<span class="chip s">${ISSUE[i] || esc(i)}</span>`).join('')}</span>
    <span class="xs t3">${SIDE[x.side] || esc(x.side)} · ${where}${x.requiredBy.length ? ` de ${esc(x.requiredBy.join(', '))}` : ''}</span>${local ? `<span class="xs t3 mono ellipsis">${esc(x.fileName)}</span>` : ''}</div>
    <div class="btns">${!local && md.updates[x.projectId] ? `<button class="btn s" data-a="mod-update-one" data-project="${esc(x.projectId)}">${icon('download', 14)}${esc(md.updates[x.projectId])}</button>` : ''}${removeBtn}</div></div>`;
}
function paintInstalled(el) {
  const list = [...md.list].sort((a, b) => a.title.localeCompare(b.title, 'es'));
  const active = isActive(state.server);
  el.innerHTML = `<div class="grid2" style="grid-template-columns:minmax(0,1.6fr) minmax(0,1fr)">
    <div class="stack" style="gap:16px">
      <div class="card"><div class="card-head"><h2>Mods del servidor</h2><div class="btns"><span class="sm t3">${list.length}</span>${list.length ? `<button class="btn s ghost" data-a="mods-remove-all" ${active ? 'disabled title="Detené el servidor primero"' : ''}>${icon('trash', 14)}Eliminar todos los mods</button>` : ''}</div></div>
      ${list.length ? `<div class="list">${list.map(modRow).join('')}</div>`
        : '<div class="pad sm t2">Este servidor todavía no tiene mods. Buscalos en <button class="link" data-a="msub" data-v="explore">Explorar Modrinth</button> o agregá los que tengas en tu PC.</div>'}</div>
      ${dropZone()}
      ${active ? `<div class="note">${icon('info')}<div class="grow"><span class="sm t2">Los cambios en mods se aplican al reiniciar el servidor. Para agregar o quitar archivos, detenelo primero.</span></div></div>` : ''}
    </div>
    <div class="stack" style="gap:16px">
      <div class="card pad stack"><label class="switch" style="padding:0"><div class="grow"><b>Buscar actualizaciones compatibles automáticamente</b><span class="sm t2">Al abrir los mods, 939 Host busca versiones nuevas para ${loaderName(state.server.config.loader)} ${esc(state.server.config.mcVersion)}. Nada se instala sin que revises los cambios, y antes se crea un punto de recuperación.</span></div><input type="checkbox" id="mods-auto" ${md.auto ? 'checked' : ''}></label>
        ${Object.keys(md.updates).length ? `<button class="btn s" data-a="msub" data-v="updates">${icon('download', 14)}${Object.keys(md.updates).length} ${Object.keys(md.updates).length === 1 ? 'actualización' : 'actualizaciones'} para revisar</button>` : ''}</div>
      <div class="card pad stack"><div class="row" style="gap:8px">${icon('gauge')}<h2>Rendimiento</h2></div>
        <p class="sm t2" style="margin:0">939 Host consulta qué mods de optimización existen para ${loaderName(state.server.config.loader)} ${esc(state.server.config.mcVersion)} y te propone una selección estable. Antes de instalar se guarda una copia de seguridad.</p>
        ${(() => { const mine = list.filter((x) => x.origin === 'optimizacion'); return mine.length ? `<div class="sm">${mine.map((x) => esc(x.title)).join(' · ')}</div>` : ''; })()}
        <button class="btn" data-a="opt-open" data-tone="go" ${md.offline || active ? 'disabled' : ''}>${icon('gauge', 15)}${list.some((x) => x.origin === 'optimizacion') ? 'Revisar mods de rendimiento' : 'Elegir mods de rendimiento'}</button>
        ${active ? '<span class="xs t3">Detené el servidor para instalar.</span>' : ''}</div>
      ${hasChunky(list) ? `<div class="card pad stack"><h3>Pregenerar el mundo</h3><p class="sm t2" style="margin:0">Chunky está instalado. La pregeneración se maneja desde la pestaña Mundo.</p><button class="btn s" data-a="tab" data-v="world">Ir a Mundo</button></div>` : ''}
    </div>
  </div>`;
  el.querySelector('#mods-auto')?.addEventListener('change', (e) => autoUpdatesChanged(e.target));
}
async function autoUpdatesChanged(el) {
  try { await invoke('mods_set_auto_updates', { id: state.params.id, on: el.checked }); md.auto = el.checked; toast(el.checked ? 'Se buscarán actualizaciones compatibles al abrir los mods' : 'Búsqueda automática desactivada'); if (el.checked && !md.offline) A['mod-check']({}); } catch (e) { el.checked = !el.checked; fail(e); }
}
A['mods-remove-all'] = async () => {
  const list = md.list;
  const libs = list.filter((x) => x.managed && !x.explicit).length;
  const ok = await dialog({ title: `Eliminar los ${list.length} mods`, danger: true,
    body: `<p>Se quitan <b>${list.length} ${list.length === 1 ? 'mod' : 'mods'}</b> de este servidor${libs ? `, incluidas ${libs} dependencias` : ''}. Los archivos van a la Papelera de 939 Host y se pueden recuperar.</p>
      <p class="sm t2">Antes se crea un punto de recuperación. El mundo, las configuraciones, la cuarentena y las aprobaciones no se tocan.</p>
      <div class="note">${icon('alert')}<div class="grow"><span class="sm">Si encendés el servidor sin estos mods, los bloques, objetos y criaturas que agregaban desaparecen del mundo. Para volver atrás, usá el punto «Antes de quitar todos los mods».</span></div></div>`,
    confirm: 'Eliminar todos' });
  if (!ok) return;
  progressCard('Quitando todos los mods', 'Creando un punto de recuperación…');
  try { const r = await invoke('mods_remove_all', { id: state.params.id }); toast(`Se quitaron ${r.length} mods`); } catch (e) { fail(e); }
  await loadMods(); paintMods();
};
/** Indicador discreto de una operación en curso dentro de la pestaña. */
function progressCard(title, stage) {
  const prog = $('#mods-progress');
  if (prog) prog.innerHTML = `<div class="card pad row" style="gap:14px" role="status"><div class="grow stack" style="gap:6px"><b>${esc(title)}</b><span class="sm t2" id="mp-msg">${esc(stage)}</span><div class="bar indet"><i id="mp-bar"></i></div></div></div>`;
}

/* ───────── Explorar ───────── */
function paintExplore(el) {
  const r = md.results;
  el.innerHTML = `<div class="store">
    <nav class="cats" aria-label="Categorías">${CATS.map(([k, l]) => `<button class="${md.cat === k ? 'on' : ''}" data-a="mcat" data-v="${k}">${l}</button>`).join('')}</nav>
    <div class="stack">
      <div class="row wrap" style="gap:10px"><div class="search grow" style="min-width:240px">${icon('search', 16)}<input id="mod-q" placeholder="Buscar en Modrinth (por ejemplo: voice chat, mapas, rendimiento)" value="${esc(md.q)}" aria-label="Buscar mods" ${md.offline ? 'disabled' : ''}></div>
        <div class="seg" role="group" aria-label="Dónde se instala">${[['server', 'Servidor'], ['both', 'Servidor y jugadores'], ['', 'Todos']].map(([k, l]) => `<button class="${md.side === k ? 'on' : ''}" data-a="mside" data-v="${k}">${l}</button>`).join('')}</div>
        <select class="input" id="mod-sort" style="width:auto" aria-label="Ordenar">${[['relevance', 'Más relevantes'], ['downloads', 'Más descargados'], ['follows', 'Más seguidos'], ['updated', 'Actualizados'], ['newest', 'Nuevos']].map(([k, l]) => `<option value="${k}" ${md.sort === k ? 'selected' : ''}>${l}</option>`).join('')}</select></div>
      <p class="xs t3" style="margin:0">Solo aparecen mods para ${loaderName(state.server.config.loader)} ${esc(state.server.config.mcVersion)}: no se puede instalar algo incompatible por error.</p>
      <div class="card" id="mod-results">${!r ? `<div class="pad">${md.offline ? '<span class="sm t2">Sin conexión con Modrinth.</span>' : '<div class="bar indet"><i></i></div>'}</div>`
        : r.hits.length ? `<div class="list">${r.hits.map(hitRow).join('')}</div>${r.totalHits > r.hits.length ? `<div class="pad" style="text-align:center"><button class="btn s" data-a="mod-more">Ver más · ${r.hits.length} de ${fmtNum(r.totalHits)}</button></div>` : ''}`
          : '<div class="pad sm t2">No hay resultados con esos filtros.</div>'}</div>
    </div></div>`;
  const q = $('#mod-q');
  q?.addEventListener('input', debounce(() => { md.q = q.value; runSearch(); }, 350));
  $('#mod-sort')?.addEventListener('change', (e) => { md.sort = e.target.value; runSearch(); });
}
function hitRow(h) {
  const installed = md.list.some((x) => x.projectId === h.projectId);
  return `<div class="li">${modIcon(h.iconUrl, h.title)}
    <div class="mod-main"><span class="t"><button class="link" data-a="mod-open" data-project="${esc(h.projectId)}" style="text-decoration:none">${esc(h.title)}</button><span class="chip s">${SIDE[sideOf(h.clientSide, h.serverSide)]}</span></span>
      <span class="d">${esc(h.description)}</span><span class="xs t3">por ${esc(h.author)} · ${fmtNum(h.downloads)} descargas</span></div>
    ${installed ? '<span class="chip">Instalado</span>' : `<button class="btn s" data-a="mod-install" data-project="${esc(h.projectId)}" data-title="${esc(h.title)}">${icon('download', 14)}Instalar</button>`}</div>`;
}
async function runSearch(more = false) {
  if (md.offline) return;
  const offset = more ? md.offset + 20 : 0;
  const seq = (runSearch.seq = (runSearch.seq || 0) + 1);
  try {
    const r = await invoke('mods_explore', { id: state.params.id, query: md.q.trim(), categories: md.cat ? [md.cat] : [], side: md.side, sort: md.sort, offset });
    if (seq !== runSearch.seq) return;
    md.offset = offset;
    md.results = more && md.results ? { ...r, hits: [...md.results.hits, ...r.hits] } : r;
  } catch (e) { fail(e); md.results = md.results || { hits: [], totalHits: 0 }; }
  if (md.sub !== 'explore' || md.detail) return;
  const box = $('#mod-results');
  if (!box) return;
  const r = md.results;
  box.innerHTML = r.hits.length ? `<div class="list">${r.hits.map(hitRow).join('')}</div>${r.totalHits > r.hits.length ? `<div class="pad" style="text-align:center"><button class="btn s" data-a="mod-more">Ver más · ${r.hits.length} de ${fmtNum(r.totalHits)}</button></div>` : ''}` : '<div class="pad sm t2">No hay resultados con esos filtros.</div>';
}
A.mcat = (el) => { md.cat = el.dataset.v; document.querySelectorAll('.cats button').forEach((b) => b.classList.toggle('on', b === el)); md.results = null; runSearch(); };
A.mside = (el) => { md.side = el.dataset.v; el.parentElement.querySelectorAll('button').forEach((b) => b.classList.toggle('on', b === el)); runSearch(); };
A['mod-more'] = (el) => { el.disabled = true; runSearch(true); };

/* ───────── Actualizaciones ───────── */
function paintUpdates(el) {
  const ups = Object.entries(md.updates);
  const byId = Object.fromEntries(md.list.filter((x) => x.projectId).map((x) => [x.projectId, x]));
  el.innerHTML = `<div class="card"><div class="card-head"><h2>Actualizaciones</h2><div class="btns">${ups.length > 1 ? `<button class="btn s pri" data-a="mod-update-all">Revisar y actualizar todo</button>` : ''}<button class="btn s" data-a="mod-check" ${md.offline ? 'disabled' : ''}>${icon('refresh', 14)}Buscar ahora</button></div></div>
    ${ups.length ? `<div class="list">${ups.map(([pid, ver]) => `<div class="li">${modIcon(null, byId[pid]?.title)}<div class="mod-main"><span class="t">${esc(byId[pid]?.title || pid)}</span><span class="xs t3 mono">${esc(byId[pid]?.versionNumber || '')} → ${esc(ver)}</span></div><button class="btn s" data-a="mod-update-one" data-project="${esc(pid)}">${icon('download', 14)}Actualizar</button></div>`).join('')}</div>`
      : `<div class="pad sm t2">${md.checked ? 'Todo está al día.' : 'Tocá «Buscar ahora» para ver si hay versiones nuevas compatibles con este servidor.'}</div>`}</div>`;
}
A['mod-check'] = async (el) => {
  el.disabled = true;
  try { md.updates = await invoke('mods_updates', { id: state.params.id }); md.checked = true; const n = Object.keys(md.updates).length; toast(n ? `${n} ${n === 1 ? 'actualización disponible' : 'actualizaciones disponibles'}` : 'Todo está al día'); } catch (e) { fail(e); }
  paintMods();
};
/** Revisión antes de actualizar: qué cambia, punto de recuperación previo y sin versiones incompatibles. */
async function reviewUpdates(pids) {
  const byId = Object.fromEntries(md.list.filter((x) => x.projectId).map((x) => [x.projectId, x]));
  const rows = pids.map((pid) => `<div class="kv"><span style="text-align:left"><b>${esc(byId[pid]?.title || pid)}</b></span><span class="mono sm">${esc(byId[pid]?.versionNumber || '?')} → ${esc(md.updates[pid])}</span></div>`).join('');
  const ok = await dialog({ title: pids.length === 1 ? 'Revisar la actualización' : `Revisar ${pids.length} actualizaciones`,
    body: `<div>${rows}</div><p class="sm t2">Solo versiones para ${loaderName(state.server.config.loader)} ${esc(state.server.config.mcVersion)}. Antes de cambiar archivos se crea un punto de recuperación; las dependencias nuevas se muestran al instalar y cada archivo se verifica por hash.${state.share?.enabled ? ' En las demás PCs del grupo, un archivo distinto vuelve a pasar por la aprobación de la propietaria.' : ''}</p>`, confirm: 'Actualizar' });
  if (!ok) return;
  progressCard('Actualizando mods', 'Creando un punto de recuperación…');
  try {
    const r = await invoke('mods_update_apply', { id: state.params.id, projectIds: pids });
    for (const pid of pids) delete md.updates[pid];
    toast(`${r.length} ${r.length === 1 ? 'mod actualizado' : 'mods actualizados'}${isActive(state.server) ? '. Reiniciá el servidor para usarlos.' : ''}`);
  } catch (e) { fail(e); }
  await loadMods(); paintMods();
}
A['mod-update-all'] = () => reviewUpdates(Object.keys(md.updates));
A['mod-update-one'] = (el) => reviewUpdates([el.dataset.project]);

A['mod-aside'] = async (el) => {
  if (!(await dialog({ title: 'Apartar archivo', body: `<p><span class="mono sm">${esc(el.dataset.file)}</span> se mueve a la Papelera de 939 Host. Podés recuperarlo desde ahí.</p>`, confirm: 'Apartar' }))) return;
  try { await invoke('mods_set_aside', { id: state.params.id, fileName: el.dataset.file }); toast('Archivo apartado'); } catch (e) { fail(e); }
  await loadMods(); paintMods();
};

/* ───────── instalar, quitar, importar ───────── */
async function installMod(projectId, title, versionId = null) {
  let plan;
  try { plan = await invoke('mods_plan', { id: state.params.id, projectId, versionId }); } catch (e) { fail(e); return; }
  const rows = plan.items.map((i) => `<div class="kv"><span style="text-align:left"><b>${esc(i.title)}</b> <span class="mono xs">${esc(i.version.versionNumber)}</span><br><span class="sm t3">${esc(i.reason)}${i.replaces ? ` · reemplaza ${esc(i.replaces)}` : ''}</span></span><span class="chip s">${SIDE[i.side] || esc(i.side)}</span></div>`).join('');
  const warn = plan.warnings.length ? `<div class="note">${icon('info')}<div class="grow">${plan.warnings.map((w) => `<span class="sm t2">${esc(w)}</span>`).join('')}</div></div>` : '';
  const ok = await dialog({ title: plan.items.length > 1 ? `Instalar ${title} y ${plan.items.length - 1} ${plan.items.length === 2 ? 'dependencia' : 'dependencias'}` : `Instalar ${title}`, body: `<div>${rows}</div>${warn}<p class="sm">Descarga: ${fmtMB(plan.totalBytes / 1048576)}. Cada archivo se verifica por hash.</p>`, confirm: 'Instalar' });
  if (!ok) return;
  const prog = $('#mods-progress');
  if (prog) prog.innerHTML = `<div class="card pad row" style="gap:14px"><span id="mp-ring">${pring(0, 28)}</span><div class="grow stack" style="gap:6px"><b id="mp-msg">Preparando…</b><div class="bar"><i id="mp-bar" style="width:0%"></i></div></div></div>`;
  try { await invoke('mods_install', { id: state.params.id, projectId, versionId }); delete md.updates[projectId]; toast(isActive(state.server) ? 'Instalado. Reiniciá el servidor para usarlo.' : 'Mods instalados'); } catch (e) { fail(e); }
  await loadMods(); paintMods();
}
A['mod-install'] = (el) => installMod(el.dataset.project, el.dataset.title, el.dataset.version || null);
A['mod-remove'] = async (el) => {
  if (!(await dialog({ title: `Quitar ${el.dataset.title}`, body: '<p>Se borra el archivo del mod. Las dependencias que solo usaba este mod también se quitan.</p>', confirm: 'Quitar' }))) return;
  try { const r = await invoke('mods_remove', { id: state.params.id, projectId: el.dataset.project }); toast(`Quitado: ${r.join(', ')}`); } catch (e) { fail(e); }
  await loadMods(); paintMods();
};
function importReport(r) {
  const part = (label, list) => (list.length ? `<div class="kv"><span class="k">${label}</span><span class="sm" style="max-width:60%">${esc(list.join(', '))}</span></div>` : '');
  return dialog({ title: r.copied.length ? `${r.copied.length} ${r.copied.length === 1 ? 'mod agregado' : 'mods agregados'}` : 'No se agregó ningún mod', body: `<div>${part('Agregados', r.copied)}${part('Solo para jugadores (omitidos)', r.skippedClient)}${part('No son compatibles con este servidor', r.skippedIncompatible || [])}${part('Dependencias de cliente que el servidor ya no exige', r.clientDepsIgnored || [])}${part('Duplicados (se usó el más nuevo)', r.skippedDuplicate)}${part('Ya estaban', r.skippedExisting)}${part('No son mods', r.notMods)}</div><p class="sm">Los archivos originales no se modificaron.</p>`, confirm: 'Listo', cancel: null });
}
async function importPaths(paths) {
  if (isActive(state.server)) { toast('Detené el servidor antes de agregar mods.', 'error'); return; }
  const jars = paths.filter((p) => /\.jar$/i.test(p));
  if (!jars.length) { toast('Solo se pueden agregar archivos .jar.', 'error'); return; }
  progressCard(jars.length === 1 ? 'Agregando 1 mod' : `Agregando ${jars.length} mods`, 'Revisando compatibilidad y copiando…');
  try { const r = await invoke('mods_import_files', { id: state.params.id, paths: jars }); $('#mods-progress') && ($('#mods-progress').innerHTML = ''); await importReport(r); } catch (e) { fail(e); }
  md.sub = 'installed';
  await loadMods(); paintMods();
}
A['mods-pick'] = async () => {
  const sel = await TAURI.dialog.open({ multiple: true, title: 'Elegí archivos de mods (.jar)', filters: [{ name: 'Mods de Minecraft', extensions: ['jar'] }] });
  if (!sel) return;
  importPaths(Array.isArray(sel) ? sel : [sel]);
};
A['mods-import-folder'] = async () => {
  if (isActive(state.server)) { toast('Detené el servidor antes de importar mods.', 'error'); return; }
  const path = await TAURI.dialog.open({ directory: true, title: 'Elegí una carpeta de mods (por ejemplo, la de tu perfil de Minecraft)' });
  if (!path) return;
  try { await importReport(await invoke('mods_import_folder', { id: state.params.id, path })); } catch (e) { fail(e); }
  await loadMods(); paintMods();
};
/* Arrastrar y soltar (eventos nativos de la ventana: traen las rutas reales). */
function dropActive() { return !dialogOpen && state.route === 'server' && state.tab === 'mods' && state.server?.config.loader !== 'vanilla'; }
function showDropOverlay(on) {
  let el = $('#drop-full');
  if (on && !el) { el = document.createElement('div'); el.id = 'drop-full'; el.className = 'dropzone-full'; el.innerHTML = '<div>Soltá los archivos .jar para agregarlos a este servidor</div>'; document.body.appendChild(el); }
  if (!on && el) el.remove();
  $('#mods-drop')?.classList.toggle('over', on);
}
function setupDragDrop() {
  listen('tauri://drag-enter', () => { if (dropActive()) showDropOverlay(true); });
  listen('tauri://drag-leave', () => showDropOverlay(false));
  listen('tauri://drag-drop', (ev) => { showDropOverlay(false); if (dropActive() && ev.payload?.paths?.length) importPaths(ev.payload.paths); });
}

/* ───────── detalle de un mod ───────── */
A['mod-open'] = async (el) => {
  md.detail = { id: el.dataset.project, loading: true };
  paintMods();
  try {
    const [p, versions] = await Promise.all([invoke('mods_project', { project: el.dataset.project }), invoke('mods_versions', { id: state.params.id, projectId: el.dataset.project }).catch(() => [])]);
    if (md.detail?.id === el.dataset.project) md.detail = { id: el.dataset.project, p, versions };
  } catch (e) { fail(e); md.detail = null; }
  paintMods();
};
A['mod-back'] = () => { md.detail = null; paintMods(); };
const plainBody = (s) => String(s || '').replace(/<[^>]+>/g, '').replace(/!\[[^\]]*\]\([^)]*\)/g, '').replace(/\[([^\]]+)\]\([^)]*\)/g, '$1').replace(/^#+\s*/gm, '').replace(/\*\*|__|`/g, '').replace(/\n{3,}/g, '\n\n').trim();
function paintDetail(root) {
  const d = md.detail;
  if (d.loading) { root.innerHTML = `<button class="btn s ghost" data-a="mod-back" style="align-self:flex-start">${icon('back', 14)}Volver</button><div class="card pad"><div class="bar indet"><i></i></div></div>`; return; }
  const p = d.p;
  const inst = md.list.find((x) => x.projectId === p.id);
  const side = sideOf(p.clientSide, p.serverSide);
  const links = [['Modrinth', `https://modrinth.com/mod/${p.slug}`], ['Código fuente', p.sourceUrl], ['Reportar problemas', p.issuesUrl], ['Wiki', p.wikiUrl]].filter(([, u]) => u && /^https:\/\//.test(u));
  root.innerHTML = `<button class="btn s ghost" data-a="mod-back" style="align-self:flex-start">${icon('back', 14)}Volver</button>
    <div class="card pad"><div class="row" style="align-items:flex-start;gap:20px">${modIcon(p.iconUrl, p.title, 'lg')}
      <div class="grow stack" style="gap:6px"><h2 style="font-size:20px">${esc(p.title)}</h2><span class="t2">${esc(p.description)}</span>
        <div class="row wrap" style="gap:6px"><span class="chip">${SIDE[side]}</span>${p.categories.filter((c) => CATS.some(([k]) => k === c)).map((c) => `<span class="chip soft">${CATS.find(([k]) => k === c)[1]}</span>`).join('')}</div></div>
      <div class="stack" style="align-items:flex-end;gap:8px">${inst ? `<span class="chip fill">Instalado · ${esc(inst.versionNumber || '')}</span>` : `<button class="btn pri" data-a="mod-install" data-project="${esc(p.id)}" data-title="${esc(p.title)}" ${d.versions.length ? '' : `disabled title="No hay versión para ${esc(state.server.config.mcVersion)}"`}>${icon('download', 16)}Instalar</button>`}
        ${d.versions.length ? '' : `<span class="xs t3">Sin versión para ${loaderName(state.server.config.loader)} ${esc(state.server.config.mcVersion)}</span>`}</div></div></div>
    <div class="grid2" style="grid-template-columns:minmax(0,1.7fr) minmax(0,1fr)">
      <div class="stack" style="gap:16px">
        ${imgs() && p.gallery.length ? `<div class="gallery">${p.gallery.slice(0, 6).map((g) => `<img src="${esc(g.url)}" alt="${esc(g.title || '')}" loading="lazy" referrerpolicy="no-referrer">`).join('')}</div>` : ''}
        <div class="card pad stack"><h2>Descripción</h2><div class="mod-body">${esc(plainBody(p.body)) || '<span class="t3">Sin descripción.</span>'}</div></div></div>
      <div class="stack" style="gap:16px">
        <div class="card pad"><div class="kv"><span class="k">Descargas</span><span class="mono">${fmtNum(p.downloads)}</span></div><div class="kv"><span class="k">Seguidores</span><span class="mono">${fmtNum(p.followers)}</span></div>
          <div class="kv"><span class="k">Actualizado</span><span>${p.updated ? esc(ago(p.updated)) : '—'}</span></div><div class="kv"><span class="k">Licencia</span><span>${esc(p.license?.name || p.license?.id || '—')}</span></div></div>
        ${links.length ? `<div class="card list">${links.map(([l, u]) => `<button class="li" data-a="link" data-url="${esc(u)}"><span class="grow">${l}</span>${icon('external', 15)}</button>`).join('')}</div>` : ''}
        <div class="card"><div class="card-head"><h2>Versiones compatibles</h2><span class="sm t3">${d.versions.length}</span></div>
          ${d.versions.length ? `<div class="list">${d.versions.slice(0, 8).map((v) => `<div class="li" style="padding:10px 20px"><div class="grow"><div class="mono sm b ellipsis">${esc(v.versionNumber)}</div><div class="xs t3">${{ release: 'Estable', beta: 'Beta', alpha: 'Alfa' }[v.versionType] || esc(v.versionType)} · ${esc(ago(v.datePublished))}</div></div>
            ${inst && inst.versionNumber === v.versionNumber ? '<span class="xs t3">Instalada</span>' : `<button class="btn s ghost" data-a="mod-install" data-project="${esc(p.id)}" data-title="${esc(p.title)}" data-version="${esc(v.id)}">Instalar</button>`}</div>`).join('')}</div>` : '<div class="pad sm t2">Ninguna para este servidor.</div>'}</div>
      </div></div>`;
}
