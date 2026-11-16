/* 939 Host — CurseForge (API oficial, con clave) y servidores desde modpacks (Modrinth,
   CurseForge o un archivo). Nada se descarga si el autor no lo permite. */

const cfx = { status: null, q: '', results: null, busy: false, error: null };
async function cfStatus() { if (!cfx.status) { try { cfx.status = await invoke('cf_status'); } catch { cfx.status = { available: false }; } } return cfx.status; }

function cfKeyHelpHTML() {
  return `<ol class="next-steps">
    <li><b>Pedí acceso</b><span class="sm t2">Entrá a <button class="link" data-a="link" data-url="https://console.curseforge.com/">console.curseforge.com</button> (CurseForge for Studios), creá una cuenta y completá el formulario para una clave de API, aceptando sus términos para terceros.</span></li>
    <li><b>Esperá la aprobación</b><span class="sm t2">CurseForge revisa el pedido y te avisa por correo. La clave es personal: no la compartas ni la publiques.</span></li>
    <li><b>Pegala acá</b><span class="sm t2">Se guarda solo en esta PC. Para que venga en las versiones de 939 Host, se agrega como secreto del repositorio (no en el código).</span></li></ol>`;
}
A['cf-key'] = async () => {
  let key = '';
  const st = await cfStatus();
  const r = await dialog({ title: 'Clave de la API de CurseForge', wide: true, confirm: 'Guardar y comprobar',
    body: `<p class="sm t2" style="margin:0">CurseForge exige una clave para que otras apps busquen y descarguen desde su plataforma.</p>${cfKeyHelpHTML()}
      <label class="field">Clave<input class="input mono" id="cf-k" type="password" autocomplete="off" spellcheck="false" placeholder="$2a$10$…"></label>`,
    buttons: [...(st.local ? [{ label: 'Borrar la clave de esta PC', value: 'clear' }] : []), { label: 'Guardar y comprobar', value: true, kind: 'pri' }],
    onMount: (d) => { const i = d.querySelector('#cf-k'); i.focus(); i.addEventListener('input', () => { key = i.value.trim(); }); } });
  if (!r) return;
  try { cfx.status = await invoke('cf_set_key', { key: r === 'clear' ? null : key }); toast(r === 'clear' ? 'Clave borrada' : 'Clave comprobada y guardada'); } catch (e) { fail(e); }
  if (state.route === 'server' && state.tab === 'mods') paintMods();
};

async function paintCurseForge(el) {
  const st = await cfStatus();
  if (!st.available) {
    el.innerHTML = `<div class="card pad stack" style="max-width:720px;gap:14px"><div class="row" style="gap:10px">${icon('box')}<h2>CurseForge necesita una clave</h2></div>
      <p class="sm t2" style="margin:0">La integración usa la API oficial de CurseForge, que pide una clave por app. Mientras tanto, Modrinth funciona sin nada.</p>${cfKeyHelpHTML()}
      <div class="row" style="gap:8px"><button class="btn pri" data-a="cf-key">Pegar la clave</button><button class="btn" data-a="msub" data-v="explore">Usar Modrinth</button></div></div>`;
    return;
  }
  const c = state.server.config;
  el.innerHTML = `<div class="stack" style="gap:14px">
    <div class="row" style="gap:8px"><label class="fm-search grow" style="width:auto">${icon('search', 14)}<input class="input" id="cf-q" placeholder="Buscar mods en CurseForge para ${loaderName(c.loader)} ${esc(c.mcVersion)}" value="${esc(cfx.q)}"></label><button class="btn s ghost" data-a="cf-key">${icon('settings', 14)}Clave</button></div>
    ${cfx.error ? `<div class="issue">${icon('alert', 18)}<div class="sm">${esc(cfx.error)}</div><span></span></div>` : ''}
    <div class="card list" id="cf-res">${cfx.busy ? '<div class="pad sm t2">Buscando…</div>' : !cfx.results ? '<div class="pad sm t2">Escribí y apretá Enter. Se muestran solo los que tienen versión para este servidor.</div>' : !cfx.results.length ? '<div class="pad sm t2">No hay resultados para esta versión.</div>'
      : cfx.results.map((m) => `<div class="li">${m.logo?.thumbnailUrl ? `<img class="mod-ic" src="${esc(m.logo.thumbnailUrl)}" alt="" loading="lazy">` : `<span class="srv-ic">${icon('box', 16)}</span>`}
        <div class="grow" style="min-width:0"><b>${esc(m.name)}</b><div class="sm t2 ellipsis">${esc(m.summary)}</div><div class="xs t3">${fmtNum(m.downloadCount)} descargas${m.allowModDistribution === false ? ' · el autor no permite descargarlo desde otras apps' : ''}</div></div>
        ${m.allowModDistribution === false ? `<button class="btn s" data-a="link" data-url="${esc(m.links?.websiteUrl || 'https://www.curseforge.com')}">Abrir en CurseForge</button>` : `<button class="btn s" data-a="cf-install" data-id="${m.id}" data-name="${esc(m.name)}" ${isActive(state.server) ? 'disabled title="Detené el servidor"' : ''}>${icon('download', 14)}Instalar</button>`}</div>`).join('')}</div>
    <p class="xs t3">Datos y descargas de la API oficial de CurseForge. Cada archivo se verifica con su SHA-1.</p></div>`;
  $('#cf-q')?.addEventListener('keydown', async (ev) => {
    if (ev.key !== 'Enter') return;
    cfx.q = ev.target.value.trim(); cfx.busy = true; cfx.error = null; paintCurseForge(el);
    try { cfx.results = await invoke('cf_search', { query: cfx.q, mc: c.mcVersion, loader: c.loader, kind: 'mods', index: 0 }); } catch (e) { cfx.error = errMsg(e); cfx.results = null; }
    cfx.busy = false; paintCurseForge(el);
  });
}
A['cf-install'] = async (el) => {
  const id = state.server.config.id;
  el.setAttribute('aria-busy', 'true');
  let plan;
  try { plan = await invoke('cf_plan', { id, modId: Number(el.dataset.id) }); } catch (e) { el.removeAttribute('aria-busy'); fail(e); return; }
  el.removeAttribute('aria-busy');
  const ok = await dialog({ title: `Instalar ${el.dataset.name}`, confirm: plan.blocked.length ? null : 'Instalar', cancel: plan.blocked.length ? 'Cerrar' : 'Cancelar',
    body: `<ul class="steps-done">${plan.items.map((i) => `<li>${icon('check', 14)}<span><b>${esc(i.title)}</b> <span class="t3 xs">${esc(i.file.displayName)} · ${esc(i.reason)}</span></span></li>`).join('')}</ul>
      ${plan.blocked.length ? `<div class="issue">${icon('alert', 18)}<div class="sm">El autor de ${plan.blocked.map((b) => `<b>${esc(b.title)}</b>`).join(', ')} no permite descargarlo desde otras apps. Bajalo desde CurseForge y agregalo en Archivos → mods.</div><span></span></div>` : ''}
      ${plan.incompatible.map((t) => `<div class="issue">${icon('alert', 18)}<div class="sm">${esc(t)}</div><span></span></div>`).join('')}
      <p class="sm t2">${fmtBytes(plan.totalBytes)} en total.</p>`,
    buttons: plan.blocked.length ? plan.blocked.map((b) => ({ label: `Abrir ${b.title}`, value: `link:${b.page || ''}` })) : undefined });
  if (typeof ok === 'string' && ok.startsWith('link:')) { invoke('open_link', { url: ok.slice(5) }).catch(() => {}); return; }
  if (!ok) return;
  try { const r = await invoke('cf_install', { id, modId: Number(el.dataset.id) }); toast(`Instalado: ${r.join(', ')}`); } catch (e) { fail(e); }
  paintMods();
};

/* ── Servidor desde un modpack ── */
const mp = { src: 'modrinth', q: '', results: null, busy: false, error: null };
routes.modpack = async function renderModpack() {
  const st = await cfStatus();
  main.innerHTML = `<div class="page">
    <div class="crumbs"><button data-a="nav" data-route="servers">Servidores</button><span>/</span><span>Desde un modpack</span></div>
    <div class="head"><h1>Crear un servidor desde un modpack</h1></div>
    <p class="t2" style="margin-top:-8px">939 Host lee el modpack, crea el servidor con su versión de Minecraft y su loader, e instala solo lo que el servidor necesita.</p>
    <div class="seg" role="tablist">${[['modrinth', 'Modrinth'], ['curseforge', 'CurseForge'], ['file', 'Desde un archivo']].map(([k, t]) => `<button class="${mp.src === k ? 'on' : ''}" data-a="mp-src" data-v="${k}">${t}</button>`).join('')}</div>
    <div id="mp-body" class="stack" style="gap:14px;margin-top:14px"></div></div>`;
  const body = $('#mp-body');
  if (mp.src === 'file') {
    body.innerHTML = `<div class="card pad stack" style="max-width:640px"><p class="sm t2" style="margin:0">Elegí un <b>.mrpack</b> (Modrinth) o el <b>.zip</b> de un modpack de CurseForge que hayas bajado.</p><div><button class="btn pri" data-a="mp-file">${icon('folder')}Elegir archivo…</button></div></div>`;
    return;
  }
  if (mp.src === 'curseforge' && !st.available) {
    body.innerHTML = `<div class="card pad stack" style="max-width:720px"><h2>CurseForge necesita una clave</h2>${cfKeyHelpHTML()}<div><button class="btn pri" data-a="cf-key">Pegar la clave</button></div></div>`;
    return;
  }
  body.innerHTML = `<label class="fm-search" style="width:min(520px,100%)">${icon('search', 14)}<input class="input" id="mp-q" placeholder="Buscar modpacks en ${mp.src === 'modrinth' ? 'Modrinth' : 'CurseForge'}" value="${esc(mp.q)}"></label>
    ${mp.error ? `<div class="issue">${icon('alert', 18)}<div class="sm">${esc(mp.error)}</div><span></span></div>` : ''}
    <div class="card list">${mp.busy ? '<div class="pad sm t2">Buscando…</div>' : !mp.results ? '<div class="pad sm t2">Escribí el nombre y apretá Enter.</div>' : !mp.results.length ? '<div class="pad sm t2">Sin resultados.</div>'
      : mp.results.map((m) => `<div class="li">${m.icon ? `<img class="mod-ic" src="${esc(m.icon)}" alt="" loading="lazy">` : `<span class="srv-ic">${icon('layers', 16)}</span>`}
        <div class="grow" style="min-width:0"><b>${esc(m.title)}</b><div class="sm t2 ellipsis">${esc(m.desc)}</div><div class="xs t3">${fmtNum(m.downloads)} descargas${m.blocked ? ' · el autor no permite descargarlo desde otras apps' : ''}</div></div>
        ${m.blocked ? `<button class="btn s" data-a="link" data-url="${esc(m.page)}">Abrir en CurseForge</button>` : `<button class="btn s pri" data-a="mp-pick" data-id="${esc(m.id)}">Elegir</button>`}</div>`).join('')}</div>`;
  $('#mp-q')?.addEventListener('keydown', async (ev) => {
    if (ev.key !== 'Enter') return;
    mp.q = ev.target.value.trim(); mp.busy = true; mp.error = null; routes.modpack();
    try {
      if (mp.src === 'modrinth') { const r = await invoke('modrinth_modpacks', { query: mp.q, offset: 0 }); mp.results = r.hits.map((h) => ({ id: h.projectId, title: h.title, desc: h.description, downloads: h.downloads, icon: h.iconUrl })); }
      else { const r = await invoke('cf_search', { query: mp.q, mc: null, loader: null, kind: 'modpacks', index: 0 }); mp.results = r.map((m) => ({ id: String(m.id), title: m.name, desc: m.summary, downloads: m.downloadCount, icon: m.logo?.thumbnailUrl, blocked: m.allowModDistribution === false, page: m.links?.websiteUrl })); }
    } catch (e) { mp.error = errMsg(e); mp.results = null; }
    mp.busy = false; routes.modpack();
  });
};
A['mp-src'] = (el) => { mp.src = el.dataset.v; mp.results = null; mp.error = null; routes.modpack(); };
A['mp-file'] = async () => { const p = await TAURI.dialog.open({ multiple: false, filters: [{ name: 'Modpack', extensions: ['mrpack', 'zip'] }] }); if (p) mpPrepare({ path: p }); };
A['mp-pick'] = (el) => mpPrepare(mp.src === 'modrinth' ? { modrinthProject: el.dataset.id } : { cfMod: Number(el.dataset.id) });

async function mpPrepare(args) {
  const root = document.getElementById('modal-root');
  root.innerHTML = `<div class="overlay"><div class="dialog"><h2>Leyendo el modpack</h2><div class="opstrip" data-tone="cycle"><span class="op-spin"></span><div class="grow"><b>Descargando y revisando</b><span class="sm t2">Versión de Minecraft, loader y qué archivos necesita el servidor.</span></div><span class="op-bar"><i></i></span></div></div></div>`;
  let path, info;
  try { [path, info] = await invoke('modpack_prepare', { path: args.path || null, modrinthProject: args.modrinthProject || null, cfMod: args.cfMod || null }); } catch (e) { root.innerHTML = ''; fail(e); return; }
  root.innerHTML = '';
  const hw = state.hw || (state.hw = await invoke('hardware_snapshot'));
  let name = info.name, mem = Math.max(hw.assessment.recommendedServerRamMb, 4096), eula = false;
  const port = await invoke('suggest_port');
  const ok = await dialog({ title: info.name, wide: true, confirm: 'Crear servidor',
    body: `<div><div class="kv"><span class="k">Minecraft</span><span>${esc(info.mcVersion)}</span></div><div class="kv"><span class="k">Loader</span><span>${loaderName(info.loader)} ${esc(info.loaderVersion || '')}</span></div>
      <div class="kv"><span class="k">Archivos para el servidor</span><span>${info.serverFiles}${info.clientOnly ? ` · ${info.clientOnly} solo para jugadores (no se instalan)` : ''}</span></div></div>
      ${info.warnings.map((w) => `<div class="note">${icon('info')}<div class="grow sm">${esc(w)}</div></div>`).join('')}
      <label class="field">Nombre del servidor<input class="input" id="mp-n" value="${esc(name)}" maxlength="40"></label>
      <div class="field"><div class="range-head"><span>Memoria (RAM)</span><span class="mono" id="mp-mv">${fmtMB(mem)}</span></div><input type="range" id="mp-m" min="2048" max="${hw.assessment.maxServerRamMb}" step="512" value="${Math.min(mem, hw.assessment.maxServerRamMb)}"><span class="hint">Los modpacks grandes suelen necesitar 6 GB o más.</span></div>
      <label class="check"><input type="checkbox" id="mp-e"><span>Acepto el <button class="link" data-a="link" data-url="https://aka.ms/MinecraftEULA">EULA de Minecraft</button><span class="hint">Mojang exige aceptarlo para crear un servidor.</span></span></label>`,
    onMount: (d) => {
      const pri = d.querySelector('.btns .pri'); if (pri) pri.disabled = true;
      d.querySelector('#mp-n').addEventListener('input', (e) => { name = e.target.value; });
      d.querySelector('#mp-m').addEventListener('input', (e) => { mem = +e.target.value; d.querySelector('#mp-mv').textContent = fmtMB(mem); });
      d.querySelector('#mp-e').addEventListener('change', (e) => { eula = e.target.checked; if (pri) pri.disabled = !eula; });
    } });
  if (!ok || !eula) return;
  try {
    const cfg = await invoke('modpack_create', { path, name: name.trim() || info.name, memoryMb: mem, port, acceptEula: true });
    toast('Servidor creado. Descargando el modpack…');
    state.tab = 'summary'; go('server', { id: cfg.id });
  } catch (e) { fail(e); }
}
