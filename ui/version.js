/* 939 Host — Cambiar la versión de un servidor: elegir versión → análisis concreto → actualizar,
   copiar o empezar de cero → avance → resultado (con volver atrás). */

const verUi = { versions: null, snapshots: null };

A['ver-change'] = async () => {
  const cfg = state.server.config;
  if (isActive(state.server)) { toast('Detené el servidor para cambiar la versión.', 'error'); return; }
  if (!verUi.versions) { try { verUi.versions = await invoke('list_versions', { includeSnapshots: false }); } catch (e) { fail(e); return; } }
  let loader = cfg.loader, snaps = false, target = null;
  // Con el mismo loader no tiene sentido elegir la versión actual; con otro loader, sí.
  const opts = () => (snaps ? verUi.snapshots || verUi.versions : verUi.versions)
    .filter((v) => loader !== cfg.loader || v.id !== cfg.mcVersion)
    .map((v) => `<option value="${esc(v.id)}">${esc(v.id)}${v.id === cfg.mcVersion ? ' (la actual)' : ''}${v.kind === 'snapshot' ? ' · snapshot' : ''}</option>`).join('');
  const LOADERS = [['vanilla', 'Vanilla (sin mods)'], ['fabric', 'Fabric'], ['forge', 'Forge'], ['neoforge', 'NeoForge']];
  const ok = await dialog({ title: 'Cambiar versión o loader', confirm: 'Revisar el cambio',
    body: `<p>Ahora: <b>${loaderName(cfg.loader)} ${esc(cfg.mcVersion)}</b>. Antes de tocar nada, 939 Host revisa qué implica (Java, loader, cada mod y el mundo), hace una copia verificada y te pide confirmar.</p>
      <label class="field">Loader<select class="input" id="ver-ld">${LOADERS.map(([k, l]) => `<option value="${k}" ${k === cfg.loader ? 'selected' : ''}>${l}${k === cfg.loader ? ' (el actual)' : ''}</option>`).join('')}</select><span class="hint">Quilt no está disponible para servidores en 939 Host: sus mods se usan con Fabric cuando son compatibles.</span></label>
      <label class="field">Versión de Minecraft<select class="input" id="ver-sel">${opts()}</select></label>
      <label class="check"><input type="checkbox" id="ver-snap"><span>Mostrar snapshots<span class="hint">Versiones de prueba de Mojang.</span></span></label>`,
    onMount: (d) => {
      const sel = d.querySelector('#ver-sel'), ld = d.querySelector('#ver-ld');
      const refill = () => { const prev = target; sel.innerHTML = opts(); if (prev && [...sel.options].some((o) => o.value === prev)) sel.value = prev; target = sel.value; };
      target = sel.value;
      sel.addEventListener('change', () => { target = sel.value; });
      ld.addEventListener('change', () => { loader = ld.value; refill(); });
      d.querySelector('#ver-snap').addEventListener('change', async (ev) => {
        snaps = ev.target.checked;
        if (snaps && !verUi.snapshots) { try { verUi.snapshots = await invoke('list_versions', { includeSnapshots: true }); } catch (e) { fail(e); } }
        refill();
      });
    } });
  if (ok && target) await verAnalyze(target, loader);
};

async function verAnalyze(to, loader) {
  const id = state.server.config.id;
  const cfg = state.server.config;
  const root = document.getElementById('modal-root');
  root.innerHTML = `<div class="overlay"><div class="dialog"><h2>Revisando el cambio a ${esc(to)}</h2><div class="opstrip" data-tone="cycle"><span class="op-spin"></span><div class="grow"><b>Analizando el servidor</b><span class="sm t2">Versión, loader, Java, mods y mundo.</span></div><span class="op-bar"><i></i></span></div></div></div>`;
  let a;
  try { a = await invoke('version_analyze', { id, to, loader }); } catch (e) { root.innerHTML = ''; fail(e); return; }
  root.innerHTML = '';
  const dir = { upgrade: 'Actualización a una versión más nueva', downgrade: 'Cambio a una versión anterior', same: 'Misma versión', loader: `Cambio de loader en Minecraft ${a.toMc}` }[a.direction];
  const count = (st) => a.mods.filter((m) => m.status === st).length;
  const modsLine = a.mods.length ? `<div class="kv"><span class="k">Mods</span><span>${[[count('ok'), 'listo', 'listos'], [count('update'), 'se actualiza', 'se actualizan'], [count('missing'), 'sin versión', 'sin versión'], [count('unknown'), 'sin verificar', 'sin verificar']].filter(([n]) => n).map(([n, a1, an]) => `${n} ${n === 1 ? a1 : an}`).join(' · ')}</span></div>` : '';
  const missing = a.mods.filter((m) => m.status === 'missing');
  const choice = await dialog({ title: `${a.fromLoader} ${a.fromMc} → ${a.loader} ${a.toMc}`, wide: true, cancel: 'Cancelar',
    body: `<p class="t2" style="margin:0">${dir}${a.snapshotInvolved ? ' · incluye una snapshot' : ''}</p>
      <div><div class="kv"><span class="k">Java</span><span>${a.javaTo ? `Java ${a.javaTo}${a.javaTo !== a.javaFrom ? ` (ahora ${a.javaFrom}; se instala solo)` : ''}` : '—'}</span></div>
        ${loader !== 'vanilla' ? `<div class="kv"><span class="k">${esc(a.loader)}</span><span>${a.loaderSupported ? `versión ${esc(a.loaderVersion || '')}` : 'sin versión para este Minecraft'}</span></div>` : ''}${modsLine}</div>
      ${a.blockers.map((b) => `<div class="issue">${icon('alert', 18)}<div class="sm">${esc(b)}</div><span></span></div>`).join('')}
      ${a.warnings.length ? `<ul class="warnlist">${a.warnings.map((w) => `<li>${esc(w)}</li>`).join('')}</ul>` : ''}
      ${missing.length ? `<details class="sm"><summary>Mods sin versión para ${esc(a.toMc)} (${missing.length})</summary><ul>${missing.map((m) => `<li>${esc(m.title)} <span class="t3">${esc(m.current)}</span></li>`).join('')}</ul></details>` : ''}
      <div class="optcards">
        <button class="optcard" data-dv="0" ${a.canUpdateInPlace ? '' : 'disabled'}><b>Actualizar este servidor</b><span class="sm t2">${a.canUpdateInPlace ? 'Se crea una copia completa y verificada, se actualiza todo y se prueba que arranque. Podés volver atrás.' : 'No es seguro hacerlo sobre este servidor.'}</span></button>
        <button class="optcard" data-dv="1" ${a.loaderChanges ? 'disabled' : ''}><b>Crear una copia con ${esc(a.toMc)}</b><span class="sm t2">${a.loaderChanges ? 'Para cambiar de loader, usá «Actualizar» (con copia previa y vuelta atrás) o «Empezar de cero».' : 'Este servidor queda intacto. Elegís qué copiar y probás la copia antes de decidir.'}</span></button>
        <button class="optcard" data-dv="2"><b>Empezar de cero</b><span class="sm t2">Servidor nuevo en ${esc(a.toMc)}, sin el mundo ni configuraciones anteriores.</span></button>
      </div>`,
    buttons: [{ label: 'update', value: 'update' }, { label: 'copy', value: 'copy' }, { label: 'new', value: 'new' }],
    onMount: (d) => { d.querySelectorAll('.btns.end [data-dv]').forEach((b) => b.remove()); } });
  if (choice === 'update') await verUpdate(id, a, loader);
  else if (choice === 'copy') await verCopy(id, a);
  else if (choice === 'new') { state.prefill = { mcVersion: a.toMc, loader }; go('create'); }
}

async function verUpdate(id, a, loader) {
  const ok = await dialog({ title: `Actualizar a ${a.toMc}`, confirm: 'Actualizar',
    body: `<p>Se conserva:</p><ul class="sm" style="margin:0;padding-left:18px">${a.keeps.map((k) => `<li>${esc(k)}</li>`).join('')}</ul>
      ${a.hasWorld && a.direction === 'upgrade' ? `<p class="sm"><b>El mundo se convierte a ${esc(a.toMc)}.</b> Después no se puede abrir con ${esc(a.fromMc)} sin riesgo; por eso antes se guarda una copia completa para volver atrás.</p>` : ''}` });
  if (!ok) return;
  const root = document.getElementById('modal-root');
  const paint = (msg, f) => { root.innerHTML = `<div class="overlay"><div class="dialog"><h2>Actualizando a ${esc(a.toMc)}</h2><div class="opstrip" data-tone="cycle"><span class="op-spin"></span><div class="grow"><b>${esc(msg)}</b><span class="sm t2">No cierres 939 Host. Si algo falla, el servidor puede volver al estado anterior.</span></div></div><div class="bar"><i style="width:${Math.round(f * 100)}%"></i></div></div></div>`; };
  paint('Analizando el servidor', 0.02);
  const un = await TAURI.event.listen('version-progress', (ev) => { if (ev.payload.id === id) paint(ev.payload.message, ev.payload.fraction); });
  let r;
  try { r = await invoke('version_update', { id, to: a.toMc, loader }); } catch (e) { un(); root.innerHTML = ''; fail(e); return; }
  un(); root.innerHTML = '';
  const again = await dialog({ title: r.ok ? `Listo: Minecraft ${a.toMc}` : 'La actualización no terminó bien', cancel: r.ok ? null : 'Cerrar',
    body: `<ul class="steps-done">${r.steps.map((s) => `<li>${icon('check', 14)}${esc(s)}</li>`).join('')}</ul>${r.error ? `<div class="issue">${icon('alert', 18)}<div class="sm">${esc(r.error)}</div><span></span></div>` : ''}
      <p class="sm t2">Copia de seguridad: ${esc(r.backupFile)} (en Copias).</p>`,
    buttons: r.ok ? [{ label: 'Listo', value: 'ok', kind: 'pri' }, { label: 'Volver a la versión anterior', value: 'rollback' }] : [{ label: 'Volver a la versión anterior', value: 'rollback', kind: 'pri' }] });
  if (again === 'rollback') await verRollback(id, a.fromMc);
  refreshServer();
}

async function verRollback(id, fromMc) {
  if (!(await dialog({ title: `Volver a ${fromMc}`, confirm: 'Volver atrás', body: '<p>Se restaura la copia completa de antes del cambio (mundo incluido), los mods anteriores y la versión anterior. Lo jugado después del cambio se pierde.</p>' }))) return;
  const root = document.getElementById('modal-root');
  root.innerHTML = `<div class="overlay"><div class="dialog"><h2>Volviendo a ${esc(fromMc)}</h2><div class="opstrip" data-tone="cycle"><span class="op-spin"></span><div class="grow"><b>Restaurando la copia y la versión anterior</b><span class="sm t2">Puede tardar unos minutos.</span></div><span class="op-bar"><i></i></span></div></div></div>`;
  try { await invoke('version_rollback', { id }); root.innerHTML = ''; toast(`Volvió a Minecraft ${fromMc}`); } catch (e) { root.innerHTML = ''; fail(e); }
  refreshServer();
}

async function verCopy(id, a) {
  const cfg = state.server.config;
  const res = await dialog({ title: `Copia con Minecraft ${a.toMc}`, confirm: 'Crear la copia',
    body: `<label class="field">Nombre<input class="input" id="cp-name" value="${esc(`${cfg.name} ${a.toMc}`)}"></label>
      <div class="stack" style="gap:8px"><label class="check"><input type="checkbox" id="cp-world" ${a.direction === 'downgrade' ? '' : 'checked'}><span>Mundo<span class="hint">${a.direction === 'downgrade' ? 'No recomendado: Minecraft no convierte mundos hacia atrás.' : `Se convierte a ${esc(a.toMc)} en la copia; el original no cambia.`}</span></span></label>
      <label class="check"><input type="checkbox" id="cp-mods" ${cfg.loader === 'vanilla' ? 'disabled' : 'checked'}><span>Mods<span class="hint">Se actualizan los que tengan versión para ${esc(a.toMc)}.</span></span></label>
      <label class="check"><input type="checkbox" id="cp-config" checked><span>Configuración y jugadores<span class="hint">Ajustes, lista blanca, operadores y bloqueados.</span></span></label></div>`,
    onMount: (d) => { d.addEventListener('input', () => { state._cp = { name: d.querySelector('#cp-name').value, world: d.querySelector('#cp-world').checked, mods: d.querySelector('#cp-mods').checked, config: d.querySelector('#cp-config').checked }; }); state._cp = { name: `${cfg.name} ${a.toMc}`, world: a.direction !== 'downgrade', mods: cfg.loader !== 'vanilla', config: true }; } });
  if (!res) return;
  const p = state._cp;
  if (p.world && a.direction === 'downgrade') {
    if (!(await dialog({ title: 'Copiar el mundo a una versión anterior', danger: true, confirm: 'Copiar igual', body: '<p>Minecraft no convierte mundos hacia atrás. En la copia pueden desaparecer bloques, ítems o zonas. El servidor original no se toca.</p>' }))) return;
  }
  const root = document.getElementById('modal-root');
  root.innerHTML = `<div class="overlay"><div class="dialog"><h2>Creando la copia</h2><div class="opstrip" data-tone="cycle"><span class="op-spin"></span><div class="grow"><b id="cp-msg">Copiando archivos</b><span class="sm t2">El servidor original no se toca.</span></div><span class="op-bar"><i></i></span></div></div></div>`;
  const un = await TAURI.event.listen('version-progress', (ev) => { const m = document.getElementById('cp-msg'); if (m) m.textContent = ev.payload.message; });
  try {
    const nid = await invoke('server_duplicate', { id, name: p.name, parts: { world: p.world, mods: p.mods, config: p.config }, to: a.toMc });
    un(); root.innerHTML = ''; toast('Copia creada'); go('server', { id: nid });
  } catch (e) { un(); root.innerHTML = ''; fail(e); }
}
