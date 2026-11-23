'use strict';
/* Asistente de reparación: diagnóstico en palabras simples, reparación automática con punto de
   restauración previo, verificación real (el servidor tiene que llegar a «Done») y Deshacer.
   Todo corre en esta PC con reglas locales: sin IA ni servicios pagos. */

const rp = { id: null, diag: null, sel: new Set(), verify: true, phase: 'idle', progress: [], result: null };
A.repair = (el) => go('repair', { id: el.dataset.id || state.params.id });

routes.repair = async function renderRepair() {
  const id = state.params.id;
  if (rp.id !== id || rp.phase === 'idle' || rp.phase === 'done') Object.assign(rp, { id, diag: null, sel: new Set(), phase: 'analyzing', progress: [], result: null });
  state.server = await invoke('get_server', { id });
  paintRepair();
  if (rp.phase === 'analyzing') analyze();
};
async function analyze() {
  rp.phase = 'analyzing'; paintRepair();
  try {
    rp.diag = await invoke('repair_analyze', { id: rp.id });
    rp.sel = new Set(rp.diag.problems.filter((p) => p.fix && !p.fix.needsAuthorization).map((p) => p.id));
    rp.phase = 'diagnosis';
  } catch (e) { fail(e); rp.phase = 'error'; rp.error = errMsg(e); }
  if (state.route === 'repair') paintRepair();
}
function shell(inner) {
  const c = state.server.config;
  return `<div class="page" style="max-width:920px">
    <div class="crumbs"><button data-a="nav" data-route="servers">Servidores</button><span>/</span><button data-a="open" data-id="${esc(c.id)}">${esc(c.name)}</button><span>/</span><span>Reparar</span></div>
    <div class="head"><div><h1>Asistente de reparación</h1><p class="sub">Busca por qué «${esc(c.name)}» no arranca y lo soluciona sin que tengas que tocar archivos.</p></div></div>
    <div class="card">${inner}</div></div>`;
}
function paintRepair() {
  if (state.route !== 'repair') return;
  if (rp.phase === 'analyzing') { main.innerHTML = shell(`<div class="pad stack" style="padding:32px"><div class="row">${SHAPES.ring}<b>Analizando el registro, los mods y la configuración…</b></div><div class="bar indet"><i></i></div><span class="sm t3">No se cambia nada mientras se analiza.</span></div>`); return; }
  if (rp.phase === 'error') { main.innerHTML = shell(`<div class="pad stack" style="padding:32px"><div class="note strong">${icon('alert')}<div class="grow"><b>No se pudo analizar</b><span class="sm t2">${esc(rp.error)}</span></div></div><div class="btns"><button class="btn pri" data-a="rp-again">Reintentar</button></div></div>`); return; }
  if (rp.phase === 'applying') { main.innerHTML = shell(progressHTML()); return; }
  if (rp.phase === 'done') { main.innerHTML = shell(resultHTML()); return; }
  main.innerHTML = shell(diagnosisHTML());
}
function diagnosisHTML() {
  const d = rp.diag;
  if (!d.problems.length) return `<div class="pad stack" style="padding:28px">
    <div class="row" style="align-items:flex-start"><span class="ok-badge">${icon('check', 22, 2)}</span><div><h2>No encontramos un problema conocido</h2><p class="t2" style="margin:4px 0 0">El último arranque no terminó con un error. Si el servidor igual no funciona, revisá la consola.</p></div></div>
    <div class="btns"><button class="btn" data-a="rp-console">Ver consola</button><button class="btn" data-a="rp-diag">Generar informe para pedir ayuda</button><button class="btn pri" data-a="rp-again">${icon('refresh', 16)}Volver a analizar</button></div></div>`;
  return `<div style="padding:24px 28px;display:flex;gap:16px;border-bottom:1px solid var(--line);align-items:flex-start">${icon('wrench', 22)}<div><h2>${esc(d.summary)}</h2><p class="sm t2" style="margin:4px 0 0">${d.fixable ? 'Elegí qué solucionar. Solo se cambia lo necesario.' : 'Estos problemas necesitan que intervengas vos.'}</p></div></div>
    <div style="padding:4px 28px">${d.problems.map((p, i) => `<div class="rep-problem">
      ${p.fix ? `<input type="checkbox" class="rp-chk" data-pid="${esc(p.id)}" ${rp.sel.has(p.id) ? 'checked' : ''} aria-label="Solucionar: ${esc(p.title)}" style="width:18px;height:18px;margin-top:4px;accent-color:var(--text);flex:none">` : `<span class="rep-n">${i + 1}</span>`}
      <div class="grow stack" style="gap:6px"><div class="row wrap" style="gap:8px"><b>${esc(p.title)}</b>${p.fix?.needsAuthorization ? '<span class="chip s">Necesita tu autorización</span>' : ''}</div>
        <span class="sm t2">${esc(p.explanation)}</span>
        ${p.fix ? `<div class="sm"><b>Solución:</b> ${esc(p.fix.summary)}</div><ul class="sm t2" style="margin:0;padding-left:18px">${p.fix.changes.map((c) => `<li>${esc(c)}</li>`).join('')}</ul>` : '<span class="sm t3">No hay una solución automática segura para esto.</span>'}
        ${p.technical.length ? `<details ${p.kind === 'unknown' ? 'open' : ''}><summary class="xs t3" style="cursor:pointer">Detalle técnico</summary><div class="tech" style="margin-top:6px">${esc(p.technical.join('\n'))}</div></details>` : ''}</div></div>`).join('')}</div>
    ${d.fixable ? `<div style="padding:18px 28px;background:var(--surface);border-top:1px solid var(--line);border-radius:0 0 var(--r-card) var(--r-card);display:flex;align-items:center;gap:16px;flex-wrap:wrap">
      <div class="grow stack" style="gap:6px"><div class="row sm" style="gap:8px">${icon('shield', 15)}<span>Antes de cambiar nada se crea el punto de restauración «Antes de reparar». Todo se puede deshacer.</span></div>
        <label class="check sm"><input type="checkbox" id="rp-verify" ${rp.verify ? 'checked' : ''}><span>Después, arrancar el servidor para comprobar que funciona</span></label></div>
      <div class="btns"><button class="btn" data-a="open" data-id="${esc(rp.id)}">Ahora no</button><button class="btn pri" data-a="rp-apply" ${rp.sel.size ? '' : 'disabled'}>${icon('wrench', 16)}Solucionar automáticamente</button></div></div>` : `<div style="padding:18px 28px;border-top:1px solid var(--line)" class="btns"><button class="btn" data-a="rp-console">Ver consola</button><button class="btn" data-a="rp-again">Volver a analizar</button></div>`}`;
}
document.addEventListener('change', (e) => {
  if (state.route !== 'repair') return;
  if (e.target.classList.contains('rp-chk')) { e.target.checked ? rp.sel.add(e.target.dataset.pid) : rp.sel.delete(e.target.dataset.pid); const b = $('[data-a="rp-apply"]'); if (b) b.disabled = !rp.sel.size; }
  if (e.target.id === 'rp-verify') rp.verify = e.target.checked;
});
A['rp-again'] = () => analyze();
A['rp-console'] = () => { state.tab = 'console'; go('server', { id: rp.id }); };
A['rp-diag'] = async (el) => { el.disabled = true; try { await invoke('diagnostics_report', { id: rp.id }); toast('Informe guardado en la carpeta de registros'); await invoke('open_folder', { id: null, which: 'applogs' }); } catch (e) { fail(e); } el.disabled = false; };
A['rp-apply'] = async () => {
  const chosen = rp.diag.problems.filter((p) => p.fix && rp.sel.has(p.id));
  const needAuth = chosen.filter((p) => p.fix.needsAuthorization);
  if (isActive(state.server)) { toast('Detené el servidor antes de reparar.', 'error'); return; }
  if (needAuth.length) {
    const ok = await dialog({ title: 'Autorizar cambios importantes', body: `<p>Estas reparaciones afectan ${needAuth.some((p) => p.kind === 'world') ? 'al mundo' : needAuth.some((p) => p.kind === 'eula') ? 'a un acuerdo legal' : 'a la configuración o a los mods'}. Revisá exactamente qué se va a hacer:</p>
      <div>${needAuth.map((p) => `<div class="kv" style="align-items:flex-start"><span style="text-align:left"><b>${esc(p.fix.summary)}</b>${p.fix.changes.map((c) => `<br><span class="sm t2">${esc(c)}</span>`).join('')}</span></div>`).join('')}</div>
      ${needAuth.some((p) => p.kind === 'eula') ? '<p class="sm">Leé el <button class="link" data-a="link" data-url="https://aka.ms/MinecraftEULA">EULA de Minecraft</button> antes de aceptarlo.</p>' : ''}
      <p class="sm">Igual se crea el punto «Antes de reparar»: lo podés deshacer.</p>`, confirm: 'Autorizo estos cambios' });
    if (!ok) return;
  }
  rp.phase = 'applying';
  rp.progress = [{ msg: 'Creando punto de restauración «Antes de reparar»', st: 'run' }];
  paintRepair();
  try {
    rp.result = await invoke('repair_apply', { id: rp.id, problemIds: chosen.map((p) => p.id), authorized: needAuth.length > 0, verify: rp.verify });
  } catch (e) { rp.result = { verification: 'error', applied: [], canUndo: true, error: errMsg(e) }; }
  rp.phase = 'done';
  state.server = await invoke('get_server', { id: rp.id }).catch(() => state.server);
  paintRepair();
};
function onRepairProgress(p) {
  if (p.id !== rp.id || rp.phase !== 'applying') return;
  const last = rp.progress[rp.progress.length - 1];
  if (last && last.msg === p.message) return;
  if (last) last.st = 'done';
  rp.progress.push({ msg: p.message, st: p.fraction >= 1 ? 'done' : 'run' });
  paintRepair();
}
function progressHTML() {
  const ic = { done: `<span class="ok-badge" style="width:26px;height:26px;border-radius:50%">${icon('check', 14, 2)}</span>`, run: SHAPES.ring.replace('class="vload"', 'class="vload lg"') };
  return `<div class="pad stack" style="padding:28px"><h2>Reparando…</h2><p class="sm t2" style="margin:0">No cierres 939 Host. ${rp.verify ? 'Al final se arranca el servidor para comprobar que funciona (puede tardar un par de minutos).' : ''}</p>
    <div>${rp.progress.map((s) => `<div class="rep-step"><span class="ico">${ic[s.st]}</span><div class="${s.st === 'run' ? 'b' : ''}">${esc(s.msg)}</div></div>`).join('')}</div></div>`;
}
function resultHTML() {
  const r = rp.result;
  const applied = r.applied?.length ? `<div><div class="sec" style="margin-bottom:6px">Qué se cambió</div>${r.applied.map((a) => `<div class="kv"><span>${esc(a)}</span>${icon('check', 14)}</div>`).join('')}</div>` : '';
  const undo = r.canUndo ? `<button class="btn" data-a="rp-undo">${icon('restart', 16)}Deshacer</button>` : '';
  if (r.verification === 'ok') return `<div class="pad stack" style="padding:28px;gap:18px"><div class="row" style="align-items:flex-start"><span class="ok-badge pop okdraw">${icon('check', 22, 2)}</span><div><h2>Listo: el servidor arrancó bien</h2><p class="t2" style="margin:4px 0 0">Se comprobó que Minecraft terminó de cargar. Tus amigos ya pueden entrar.</p></div></div>${applied}
    <div class="btns">${undo}<button class="btn pri" data-a="open" data-id="${esc(rp.id)}">Ir al servidor</button></div></div>`;
  if (r.verification === 'skipped') return `<div class="pad stack" style="padding:28px;gap:18px"><div class="row" style="align-items:flex-start"><span class="ok-badge pop okdraw">${icon('check', 22, 2)}</span><div><h2>Cambios aplicados</h2><p class="t2" style="margin:4px 0 0">No se arrancó el servidor para comprobarlo.</p></div></div>${applied}
    <div class="btns">${undo}<button class="btn pri" data-a="rp-start">${icon('play', 16)}Iniciar y comprobar</button></div></div>`;
  return `<div class="pad stack" style="padding:28px;gap:18px"><div class="note strong">${icon('alert')}<div class="grow"><b>${r.verification === 'error' ? 'No se pudo completar la reparación' : 'El servidor todavía no arranca'}</b><span class="sm t2">${esc(r.error || 'Puede haber otro problema además del que se resolvió.')}</span></div></div>${applied}
    ${r.remaining?.problems?.length ? `<p class="sm t2" style="margin:0">El nuevo análisis encontró ${r.remaining.problems.length === 1 ? 'otro problema' : `${r.remaining.problems.length} problemas`}.</p>` : ''}
    <div class="btns">${undo}<button class="btn" data-a="rp-console">Ver consola</button><button class="btn pri" data-a="rp-again">${icon('refresh', 16)}Analizar de nuevo</button></div></div>`;
}
A['rp-start'] = async () => { try { await invoke('start_server', { id: rp.id }); toast('Iniciando servidor…'); } catch (e) { fail(e); } rp.phase = 'idle'; go('server', { id: rp.id }); };
A['rp-undo'] = async () => {
  if (!(await dialog({ title: 'Deshacer la reparación', body: '<p>El servidor vuelve exactamente al punto «Antes de reparar»: mundo, mods y configuración. Si está encendido, se detiene primero.</p>', confirm: 'Deshacer' }))) return;
  try {
    const v = await invoke('get_server', { id: rp.id });
    if (isActive(v)) { await invoke('stop_server', { id: rp.id }); await waitStopped(rp.id); }
    await invoke('repair_undo', { id: rp.id });
    toast('Reparación deshecha');
  } catch (e) { fail(e); return; }
  rp.phase = 'idle';
  go('server', { id: rp.id });
};
async function waitStopped(id) {
  for (let i = 0; i < 200; i++) { const v = await invoke('get_server', { id }); if (!isActive(v)) return; await new Promise((r) => setTimeout(r, 500)); }
  throw new Error('El servidor tardó demasiado en detenerse.');
}
