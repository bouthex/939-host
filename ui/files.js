/* 939 Host — Archivos del servidor: explorar, crear, copiar, mover, renombrar, borrar (a la
   Papelera), importar y exportar, buscar. Local o en la PC anfitriona del grupo (P2P).
   El mundo queda protegido mientras el servidor está encendido. */

const fm = { path: '', list: null, q: '', results: null, sel: new Set(), clip: null, busy: null, forId: null };
const FM_QUICK = [['', 'Servidor', 'folder'], ['world', 'Mundo', 'globe'], ['mods', 'Mods', 'box'], ['config', 'Configuración', 'settings'], ['world/datapacks', 'Datapacks', 'layers'], ['resourcepacks', 'Resource packs', 'image'], ['logs', 'Registros', 'file']];
const FM_KIND = { world: 'globe', mods: 'box', config: 'settings', datapacks: 'layers', resourcepacks: 'image', plugins: 'box', logs: 'file', jar: 'box' };
const fmJoin = (a, b) => (a ? `${a}/${b}` : b);
const fmParent = (p) => p.split('/').slice(0, -1).join('/');
const fmName = (p) => p.split('/').pop();

tabDraw.files = async (el) => {
  const id = state.server.config.id;
  if (fm.forId !== id) Object.assign(fm, { path: '', list: null, q: '', results: null, sel: new Set(), clip: null, forId: id });
  await fmLoad();
  fmPaint(el);
};
async function fmLoad() {
  try { fm.list = await invoke('files_list', { id: fm.forId, path: fm.path }); fm.error = null; }
  catch (e) { fm.error = errMsg(e); fm.list = fm.list || { entries: [], remote: false }; }
  fm.sel.clear();
}
function fmRow(e) {
  const on = fm.sel.has(e.path);
  return `<div class="fm-row${on ? ' on' : ''}${e.locked ? ' locked' : ''}" data-a="fm-row" data-p="${esc(e.path)}" data-dir="${e.dir ? 1 : 0}" draggable="false">
    <input type="checkbox" class="fm-chk" data-a="fm-sel" data-p="${esc(e.path)}" ${on ? 'checked' : ''} aria-label="Elegir ${esc(e.name)}">
    <span class="fm-ic">${icon(e.dir ? (FM_KIND[e.kind] && e.path.split('/').length === 1 ? FM_KIND[e.kind] : 'folder') : (FM_KIND[e.kind] || 'file'), 16)}</span>
    <span class="fm-name ellipsis">${esc(e.name)}${fm.results ? `<span class="xs t3 fm-loc">${esc(fmParent(e.path) || 'carpeta del servidor')}</span>` : ''}</span>
    ${e.locked ? `<span class="xs t3 fm-lock" title="Protegido mientras el servidor está encendido">${icon('shield', 13)}</span>` : '<span></span>'}
    <span class="xs t3 num fm-size">${e.dir ? '' : fmtBytes(e.size)}</span>
    <span class="xs t3 fm-date">${e.modified ? fmtDate(e.modified) : ''}</span>
  </div>`;
}
function fmPaint(el = $('#srv-tab')) {
  if (!el) return;
  const L = fm.list || { entries: [] };
  const rows = fm.results || L.entries;
  const crumbs = ['', ...fm.path.split('/').filter(Boolean)];
  const n = fm.sel.size;
  const one = n === 1 ? [...fm.sel][0] : null;
  const anyLocked = rows.some((r) => fm.sel.has(r.path) && r.locked);
  el.innerHTML = `<div class="stack" style="gap:14px">
    ${L.remote ? `<div class="note">${icon('friends')}<div class="grow"><b>Archivos en la PC de ${esc(L.hostName)}</b><span class="sm t2">Es la anfitriona ahora. Los cambios se hacen allá por la conexión cifrada del grupo y viajan a las demás PCs con la próxima sincronización.</span></div></div>` : ''}
    ${L.active ? `<div class="note">${icon('shield')}<div class="grow"><b>El mundo está protegido</b><span class="sm t2">Con el servidor encendido, Minecraft escribe el mundo en cualquier momento: sus archivos se pueden ver, pero no modificar ni descargar. Para una copia en caliente usá Copias.</span></div></div>` : ''}
    ${fm.error ? `<div class="issue">${icon('alert', 18)}<div class="sm">${esc(fm.error)}</div><button class="btn s" data-a="fm-reload">Reintentar</button></div>` : ''}
    <div class="fm-quick">${FM_QUICK.map(([p, t, ic]) => `<button class="chip-btn${fm.path === p && !fm.results ? ' on' : ''}" data-a="fm-go" data-p="${p}">${icon(ic, 14)}${t}</button>`).join('')}</div>
    <div class="card fm" id="fm-drop">
      <div class="fm-bar">
        <nav class="fm-crumbs" aria-label="Ubicación">${fm.results ? `<span>Resultados para «${esc(fm.q)}»</span><button class="link" data-a="fm-clear">Volver</button>` : crumbs.map((c, i) => `${i ? '<span class="t3">/</span>' : ''}<button class="link" data-a="fm-go" data-p="${esc(crumbs.slice(1, i + 1).join('/'))}">${i ? esc(c) : 'Servidor'}</button>`).join('')}</nav>
        <label class="fm-search">${icon('search', 14)}<input class="input" id="fm-q" placeholder="Buscar archivos" value="${esc(fm.q)}"></label>
      </div>
      <div class="fm-tools">
        ${n ? `<span class="sm">${n} ${n === 1 ? 'elegido' : 'elegidos'}</span>
          <button class="btn s" data-a="fm-export">${icon('download', 14)}Exportar</button>
          ${one ? `<button class="btn s" data-a="fm-rename" ${anyLocked ? 'disabled' : ''}>${icon('edit', 14)}Renombrar</button>` : ''}
          <button class="btn s" data-a="fm-clip" data-m="copy">${icon('copy', 14)}Copiar</button>
          <button class="btn s" data-a="fm-clip" data-m="move" ${anyLocked ? 'disabled' : ''}>${icon('scissors', 14)}Cortar</button>
          <button class="btn s" data-a="fm-delete" data-tone="halt" ${anyLocked ? 'disabled' : ''}>${icon('trash', 14)}Borrar</button>
          <button class="btn s ghost" data-a="fm-unsel">Cancelar</button>`
        : `<button class="btn s" data-a="fm-mkdir" ${fm.results ? 'disabled' : ''}>${icon('folderplus', 14)}Nueva carpeta</button>
          <button class="btn s" data-a="fm-import" data-dir="0" ${fm.results ? 'disabled' : ''}>${icon('upload', 14)}Importar archivos</button>
          <button class="btn s" data-a="fm-import" data-dir="1" ${fm.results ? 'disabled' : ''}>${icon('folder', 14)}Importar carpeta</button>
          ${fm.clip ? `<button class="btn s" data-a="fm-paste" data-tone="go" ${fm.results ? 'disabled' : ''}>${icon('paste', 14)}Pegar ${fm.clip.paths.length === 1 ? `«${esc(fmName(fm.clip.paths[0]))}»` : `${fm.clip.paths.length} elementos`} acá</button><button class="btn s icon ghost" data-a="fm-clipclear" aria-label="Vaciar portapapeles">${icon('x', 14)}</button>` : ''}
          <span class="grow"></span>${!L.remote ? `<button class="btn s ghost" data-a="fm-openlocal">${icon('folder', 14)}Abrir en Windows</button>` : ''}`}
      </div>
      <div class="fm-head xs t3"><span></span><span></span><span>Nombre</span><span></span><span class="num">Tamaño</span><span>Modificado</span></div>
      <div class="fm-list">${rows.length ? rows.map(fmRow).join('') : `<div class="fm-empty"><span class="wi">${icon(fm.results ? 'search' : 'folder', 22)}</span><b>${fm.results ? 'No se encontró nada con ese nombre' : 'Carpeta vacía'}</b><span class="sm t2">${fm.results ? 'Probá con otra parte del nombre.' : 'Arrastrá archivos o carpetas acá, o usá «Importar».'}</span></div>`}</div>
    </div>
    <p class="xs t3">Lo que borrás o reemplazás va a la Papelera de 939 Host y se puede restaurar durante 30 días. Arrastrá archivos desde Windows a esta lista para importarlos en la carpeta abierta.</p>
  </div>`;
  const q = $('#fm-q');
  q?.addEventListener('keydown', async (ev) => {
    if (ev.key !== 'Enter') return;
    fm.q = q.value.trim();
    if (fm.q.length < 2) { fm.results = null; fmPaint(); return; }
    try { fm.results = await invoke('files_search', { id: fm.forId, q: fm.q }); } catch (e) { fail(e); }
    fm.sel.clear(); fmPaint();
  });
}
const fmRefresh = async () => { if (fm.results && fm.q) { try { fm.results = await invoke('files_search', { id: fm.forId, q: fm.q }); } catch { fm.results = null; } } await fmLoad(); fmPaint(); };

A['fm-go'] = async (el) => { fm.path = el.dataset.p; fm.results = null; fm.q = ''; await fmLoad(); fmPaint(); };
A['fm-clear'] = async () => { fm.results = null; fm.q = ''; fmPaint(); };
A['fm-reload'] = fmRefresh;
A['fm-unsel'] = () => { fm.sel.clear(); fmPaint(); };
A['fm-sel'] = (el, ev) => { ev?.stopPropagation?.(); const p = el.dataset.p; fm.sel.has(p) ? fm.sel.delete(p) : fm.sel.add(p); fmPaint(); };
A['fm-row'] = async (el, ev) => {
  if (ev?.target?.closest?.('.fm-chk')) return;
  if (el.dataset.dir === '1') { fm.path = el.dataset.p; fm.results = null; fm.q = ''; await fmLoad(); fmPaint(); return; }
  const p = el.dataset.p; fm.sel.has(p) ? fm.sel.delete(p) : fm.sel.add(p); fmPaint();
};
A['fm-openlocal'] = async () => { try { await invoke('files_open_local', { id: fm.forId, path: fm.path }); } catch (e) { fail(e); } };

A['fm-mkdir'] = async () => {
  let name = '';
  const ok = await dialog({ title: 'Nueva carpeta', confirm: 'Crear', body: `<label class="field">Nombre<input class="input" id="fm-n" maxlength="80"></label>`, onMount: (d) => { const i = d.querySelector('#fm-n'); i.focus(); i.addEventListener('input', () => { name = i.value.trim(); }); } });
  if (!ok || !name) return;
  try { await invoke('files_mkdir', { id: fm.forId, path: fmJoin(fm.path, name) }); toast('Carpeta creada'); } catch (e) { fail(e); }
  fmRefresh();
};
A['fm-rename'] = async () => {
  const p = [...fm.sel][0];
  let name = fmName(p);
  const ok = await dialog({ title: 'Renombrar', confirm: 'Renombrar', body: `<label class="field">Nombre nuevo<input class="input" id="fm-n" value="${esc(name)}" maxlength="120"></label>`,
    onMount: (d) => { const i = d.querySelector('#fm-n'); i.focus(); const dot = name.lastIndexOf('.'); i.setSelectionRange(0, dot > 0 ? dot : name.length); i.addEventListener('input', () => { name = i.value.trim(); }); } });
  if (!ok || !name || name === fmName(p)) return;
  try { await invoke('files_transfer', { id: fm.forId, from: p, to: fmJoin(fmParent(p), name), mv: true, replace: false }); toast('Renombrado'); } catch (e) { fail(e); }
  fmRefresh();
};
A['fm-delete'] = async () => {
  const ps = [...fm.sel];
  if (!(await dialog({ title: ps.length === 1 ? `Borrar «${fmName(ps[0])}»` : `Borrar ${ps.length} elementos`, confirm: 'Mandar a la Papelera', danger: true,
    body: `<p>Va a la Papelera de 939 Host: se puede restaurar en su lugar durante 30 días.</p>${ps.length > 1 ? `<ul class="sm">${ps.slice(0, 8).map((p) => `<li class="mono">${esc(p)}</li>`).join('')}${ps.length > 8 ? `<li>y ${ps.length - 8} más</li>` : ''}</ul>` : ''}` }))) return;
  try { await invoke('files_delete', { id: fm.forId, paths: ps }); toast('Enviado a la Papelera'); refreshTrashCount(); } catch (e) { fail(e); }
  fmRefresh();
};
A['fm-clip'] = (el) => { fm.clip = { mode: el.dataset.m, paths: [...fm.sel] }; fm.sel.clear(); toast(el.dataset.m === 'copy' ? 'Copiado: abrí la carpeta de destino y tocá «Pegar»' : 'Cortado: abrí la carpeta de destino y tocá «Pegar»'); fmPaint(); };
A['fm-clipclear'] = () => { fm.clip = null; fmPaint(); };
A['fm-paste'] = async () => {
  const { mode, paths } = fm.clip;
  for (const p of paths) {
    const to = fmJoin(fm.path, fmName(p));
    if (to === p) { if (mode === 'move') continue; }
    let dest = to === p ? fmJoin(fm.path, `copia de ${fmName(p)}`) : to;
    try { await invoke('files_transfer', { id: fm.forId, from: p, to: dest, mv: mode === 'move', replace: false }); }
    catch (e) {
      if (e?.code === 'exists') {
        const r = await dialog({ title: `«${fmName(p)}» ya existe acá`, cancel: 'Omitir', body: '<p>Podés reemplazarlo (el actual va a la Papelera) o guardar los dos.</p>',
          buttons: [{ label: 'Guardar los dos', value: 'both' }, { label: 'Reemplazar', value: 'replace', kind: 'pri' }] });
        try {
          if (r === 'replace') await invoke('files_transfer', { id: fm.forId, from: p, to: dest, mv: mode === 'move', replace: true });
          else if (r === 'both') { const dot = fmName(p).lastIndexOf('.'); const nm = dot > 0 ? `${fmName(p).slice(0, dot)} (2)${fmName(p).slice(dot)}` : `${fmName(p)} (2)`; await invoke('files_transfer', { id: fm.forId, from: p, to: fmJoin(fm.path, nm), mv: mode === 'move', replace: false }); }
        } catch (e2) { fail(e2); }
      } else fail(e);
    }
  }
  if (mode === 'move') fm.clip = null;
  fmRefresh();
};

/* Transferencias con progreso y cancelación. */
async function fmWithProgress(title, fn) {
  const root = document.getElementById('modal-root');
  root.innerHTML = `<div class="overlay"><div class="dialog"><h2>${esc(title)}</h2><div class="opstrip" data-tone="cycle"><span class="op-spin"></span><div class="grow"><b id="fmp-name">Preparando…</b><span class="sm t2" id="fmp-n"></span></div></div><div class="bar"><i id="fmp-bar" style="width:0%"></i></div><div class="btns end"><button class="btn" id="fmp-cancel">Cancelar</button></div></div></div>`;
  $('#fmp-cancel').addEventListener('click', () => { invoke('files_cancel'); $('#fmp-cancel').disabled = true; $('#fmp-name').textContent = 'Cancelando…'; });
  const un = await TAURI.event.listen('files-progress', (ev) => {
    const p = ev.payload; if (p.id !== fm.forId) return;
    const pct = p.total ? Math.min(100, (p.done / p.total) * 100) : 0;
    const bar = $('#fmp-bar'); if (bar) bar.style.width = `${pct}%`;
    if (p.name) { const n = $('#fmp-name'); if (n) n.textContent = p.name; }
    const c = $('#fmp-n'); if (c && p.total > 1) c.textContent = `${fmtBytes(p.done)} de ${fmtBytes(p.total)}`;
  });
  try { return await fn(); } finally { un(); root.innerHTML = ''; }
}
async function fmImport(sources) {
  if (!sources?.length) return;
  // .jar en mods/: se valida antes de copiar.
  if (fm.path === 'mods') {
    for (const s of sources.filter((x) => x.toLowerCase().endsWith('.jar'))) {
      const c = await invoke('files_check_jar', { id: fm.forId, path: s }).catch(() => null);
      if (c && !c.ok && !(await dialog({ title: `«${s.split(/[\\/]/).pop()}» puede no servir`, confirm: 'Agregar igual', body: `<p>${esc(c.message)}</p>` }))) return;
    }
  }
  const go = (replace) => fmWithProgress('Importando', () => invoke('files_import', { id: fm.forId, sources, dest: fm.path, replace }));
  try { const r = await go(false); toast(r.length === 1 ? 'Importado' : `${r.length} elementos importados`); }
  catch (e) {
    if (e?.code === 'exists' || /ya existe/i.test(errMsg(e))) {
      if (await dialog({ title: 'Ya existe en esta carpeta', confirm: 'Reemplazar', body: `<p>${esc(errMsg(e))} Si reemplazás, lo actual va a la Papelera.</p>` })) { try { await go(true); toast('Importado'); } catch (e2) { fail(e2); } }
    } else fail(e);
  }
  fmRefresh();
}
A['fm-import'] = async (el) => {
  const dir = el.dataset.dir === '1';
  const picked = await TAURI.dialog.open({ multiple: !dir, directory: dir, title: dir ? 'Elegí la carpeta para importar' : 'Elegí los archivos para importar' });
  if (!picked) return;
  fmImport(Array.isArray(picked) ? picked : [picked]);
};
A['fm-export'] = async () => {
  const dest = await TAURI.dialog.open({ directory: true, title: 'Elegí dónde guardar' });
  if (!dest) return;
  const ps = [...fm.sel];
  try {
    let last;
    for (const p of ps) last = await fmWithProgress(`Exportando «${fmName(p)}»`, () => invoke('files_export', { id: fm.forId, path: p, destDir: dest }));
    toast(ps.length === 1 ? `Guardado en ${last}` : `${ps.length} elementos exportados`);
  } catch (e) { fail(e); }
  fm.sel.clear(); fmPaint();
};
/* Arrastrar desde Windows a la lista. */
listen('tauri://drag-drop', (ev) => { if (!dialogOpen && state.route === 'server' && state.tab === 'files' && !fm.results && ev.payload?.paths?.length) fmImport(ev.payload.paths); });
listen('tauri://drag-enter', () => { if (!dialogOpen && state.route === 'server' && state.tab === 'files') $('#fm-drop')?.classList.add('over'); });
listen('tauri://drag-leave', () => $('#fm-drop')?.classList.remove('over'));
