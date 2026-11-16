/* 939 Host — Mods de rendimiento: selección según versión y loader (consultada en Modrinth),
   presets, vista previa, instalación con el gestor de mods y pregeneración guiada con Chunky. */

const optCache = {};
async function optLoad(mc, loader, id) {
  const k = `${mc}|${loader}|${id || ''}`;
  if (!optCache[k]) optCache[k] = invoke('optimize_options', { mc, loader, id: id || null }).catch((e) => { delete optCache[k]; throw e; });
  return optCache[k];
}
const PRESET_TEXT = {
  recommended: 'Los mods más estables: menos memoria, arranque más rápido y lógica del juego más eficiente, sin cambiar cómo se juega.',
  max: 'Suma optimizaciones más agresivas (generación de chunks en paralelo y ajustes para servidores). Algunas cambian detalles, como la actividad de mobs lejanos.',
  custom: 'Elegís uno por uno. Los experimentales pueden cambiar el comportamiento del juego.',
};
const sideText = (c) => c.client === 'required' ? 'También en los jugadores' : c.client === 'unsupported' ? 'Solo servidor' : 'Solo servidor (opcional en jugadores)';

/* Lista de mods con selección. `sel` = Set de slugs elegidos. */
function optListHTML(o, sel, editable) {
  return `<div class="optlist">${o.candidates.map((c) => {
    const on = sel.has(c.slug);
    return `<label class="optmod${c.available ? '' : ' off'}">
      ${editable ? `<input type="checkbox" data-om="${esc(c.slug)}" ${on ? 'checked' : ''} ${c.available && !c.installed ? '' : 'disabled'}>` : ''}
      ${c.iconUrl ? `<img src="${esc(c.iconUrl)}" alt="" loading="lazy">` : `<span class="ph">${icon('cube', 16)}</span>`}
      <span class="grow"><span class="row" style="gap:8px;flex-wrap:wrap"><b>${esc(c.title)}</b>${c.versionNumber ? `<span class="xs t3 mono">${esc(c.versionNumber)}</span>` : ''}
        ${c.experimental ? '<span class="tag hold">Experimental</span>' : c.tier === 'max' ? '<span class="tag">Máximo</span>' : '<span class="tag go">Recomendado</span>'}
        ${c.installed ? '<span class="tag">Instalado</span>' : ''}</span>
        <span class="sm t2">${esc(c.what)}</span>
        <span class="xs t3">${c.available ? `${sideText(c)}${c.dependencies.length ? ` · necesita ${esc(c.dependencies.join(', '))} (se instala solo)` : ''}${c.avoidWith.length ? ` · no combinar con ${esc(c.avoidWith.join(', '))}` : ''}` : esc(c.unavailableReason || 'No disponible')}</span></span>
    </label>`;
  }).join('')}</div>`;
}

/* Diálogo de elección. Devuelve los slugs o null. */
async function optDialog({ mc, loader, id, preset = 'recommended', current }) {
  let o;
  try { o = await optLoad(mc, loader, id); } catch (e) { fail(e); return null; }
  if (!o.candidates.some((c) => c.available)) {
    await dialog({ title: 'Mods de rendimiento', cancel: null, buttons: [{ label: 'Entendido', value: true, kind: 'pri' }], body: `<p>Ninguno de los mods de rendimiento que 939 Host considera confiables tiene versión para ${loaderName(loader)} ${esc(mc)} todavía. No se instala nada en su lugar.</p>` });
    return null;
  }
  let mode = preset;
  const sel = new Set(current || o[preset] || o.recommended);
  const chosen = await dialog({ title: `Mods de rendimiento · ${loaderName(loader)} ${esc(mc)}`, wide: true, confirm: id ? 'Instalar' : 'Usar esta selección',
    body: `<div class="seg" role="tablist">${[['recommended', 'Recomendado'], ['max', 'Máximo rendimiento'], ['custom', 'Personalizado']].map(([k, t]) => `<button data-op="${k}" class="${k === mode ? 'on' : ''}">${t}</button>`).join('')}</div>
      <p class="sm t2" id="op-text">${PRESET_TEXT[mode]}</p><div id="op-list">${optListHTML(o, sel, mode === 'custom')}</div><p class="sm" id="op-warn"></p>
      <p class="xs t3">Disponibilidad consultada ahora en Modrinth para esta versión exacta. Las mejoras dependen de cada servidor: no hay una cifra garantizada.</p>`,
    onMount: (d) => {
      const paint = () => {
        d.querySelector('#op-text').textContent = PRESET_TEXT[mode];
        d.querySelector('#op-list').innerHTML = optListHTML(o, sel, mode === 'custom');
        const bad = o.candidates.filter((c) => sel.has(c.slug) && c.avoidWith.some((a) => sel.has(a)));
        d.querySelector('#op-warn').textContent = bad.length ? `${bad.map((c) => c.title).join(' y ')} no conviene instalarlos juntos: elegí uno.` : '';
        const pri = d.querySelector('.btns .pri'); if (pri) pri.disabled = !!bad.length || !sel.size;
      };
      d.querySelectorAll('[data-op]').forEach((b) => b.addEventListener('click', () => {
        mode = b.dataset.op; d.querySelectorAll('[data-op]').forEach((x) => x.classList.toggle('on', x === b));
        if (mode !== 'custom') { sel.clear(); o[mode].forEach((s) => sel.add(s)); }
        paint();
      }));
      d.addEventListener('change', (e) => { const s = e.target.dataset?.om; if (s) { e.target.checked ? sel.add(s) : sel.delete(s); paint(); } });
      paint();
    } });
  return chosen ? { slugs: [...sel].filter((s) => o.candidates.find((c) => c.slug === s && c.available && !c.installed)), mode } : null;
}

/* ── Asistente de creación ── */
function wizardOptHTML(w) {
  if (w.loader === 'vanilla' || !(w.support && w.support.supported)) return '';
  const n = w.optSlugs ? w.optSlugs.length : 0;
  return `<div class="stack" style="gap:10px"><h3>Optimización del servidor <span class="hint">Opcional</span></h3><div class="card pad stack" style="gap:12px">
    <label class="switch"><div class="grow"><b>Instalar mods de rendimiento</b><span class="sm t2">Mods recomendados para mejorar el rendimiento, reducir el consumo de memoria y optimizar el funcionamiento del servidor. Se eligen para esta versión exacta.</span></div><input type="checkbox" data-a="wopt-toggle" ${w.optOn ? 'checked' : ''}></label>
    ${w.optOn ? `<div class="row" style="gap:10px;flex-wrap:wrap"><span class="sm">${w.optLoading ? 'Consultando qué mods sirven para esta versión…' : n ? `${n} mods · ${w.optMode === 'max' ? 'Máximo rendimiento' : w.optMode === 'custom' ? 'Personalizado' : 'Recomendado'}` : 'Ninguno disponible para esta versión'}</span>
      <button class="btn s" data-a="wopt-pick">Ver y elegir qué se instala</button></div>` : ''}
  </div></div>`;
}
A['wopt-toggle'] = async (el) => {
  const w = state.wizard;
  w.optOn = el.checked;
  if (w.optOn && !w.optSlugs) {
    w.optLoading = true; drawCreate();
    try { const o = await optLoad(w.mcVersion, w.loader); w.optSlugs = o.recommended; w.optMode = 'recommended'; } catch (e) { w.optOn = false; fail(e); }
    w.optLoading = false;
  }
  drawCreate();
};
A['wopt-pick'] = async () => {
  const w = state.wizard;
  const r = await optDialog({ mc: w.mcVersion, loader: w.loader, preset: w.optMode || 'recommended', current: w.optSlugs });
  if (r) { w.optSlugs = r.slugs; w.optMode = r.mode; drawCreate(); }
};

/* ── Servidor existente (pestaña Mods) ── */
A['opt-open'] = async () => {
  const c = state.server.config;
  if (isActive(state.server)) { toast('Detené el servidor para instalar mods: se cargan al arrancar.', 'error'); return; }
  const r = await optDialog({ mc: c.mcVersion, loader: c.loader, id: c.id });
  if (!r || !r.slugs.length) return;
  const root = document.getElementById('modal-root');
  const paint = (m) => { root.innerHTML = `<div class="overlay"><div class="dialog"><h2>Mods de rendimiento</h2><div class="opstrip" data-tone="cycle"><span class="op-spin"></span><div class="grow"><b>${esc(m)}</b><span class="sm t2">Antes se guardó una copia de seguridad del servidor.</span></div><span class="op-bar"><i></i></span></div></div></div>`; };
  paint('Creando una copia de seguridad');
  const un = await TAURI.event.listen('version-progress', (ev) => { if (ev.payload.id === c.id) paint(ev.payload.message); });
  let rep;
  try { rep = await invoke('optimize_install', { id: c.id, slugs: r.slugs }); } catch (e) { un(); root.innerHTML = ''; fail(e); return; }
  un(); root.innerHTML = '';
  for (const k of Object.keys(optCache)) delete optCache[k];
  await dialog({ title: rep.installed.length ? 'Mods de rendimiento instalados' : 'No se instaló nada', cancel: null, buttons: [{ label: 'Listo', value: true, kind: 'pri' }],
    body: `${rep.installed.length ? `<ul class="steps-done">${rep.installed.map((m) => `<li>${icon('check', 14)}${esc(m)}</li>`).join('')}</ul>` : ''}
      ${rep.skipped.length ? `<p class="sm"><b>No se instalaron:</b></p><ul class="sm">${rep.skipped.map(([m, why]) => `<li>${esc(m)}: ${esc(why)}</li>`).join('')}</ul>` : ''}
      ${rep.clientNeeded.length ? `<div class="note">${icon('info')}<div class="grow sm"><b>${esc(rep.clientNeeded.join(', '))}</b> también ${rep.clientNeeded.length === 1 ? 'tiene' : 'tienen'} que estar en el Minecraft de cada jugador. Ya ${rep.clientNeeded.length === 1 ? 'va' : 'van'} en «Preparar perfil de mods».</div></div>` : ''}
      <p class="sm t2">Aparecen en la lista de mods con la marca «Rendimiento»; se pueden quitar uno por uno. Se cargan la próxima vez que inicies el servidor.</p>` });
  refreshServer();
};

/* ── Chunky: pregeneración guiada ── */
const chunky = {};
TAURI.event.listen('console-line', (ev) => {
  const l = ev.payload.line;
  if (!l.includes('[Chunky]')) return;
  const m = l.match(/Task (running|finished|stopped|cancelled|paused) for ([\w:./-]+)\.\s*Processed: (\d+) chunks \(([\d.]+)%\)(?:, ETA: ([\d:]+))?(?:, Rate: ([\d.]+) cps)?/);
  if (!m) return;
  chunky[ev.payload.id] = { state: m[1], world: m[2], chunks: +m[3], pct: +m[4], eta: m[5] || null, rate: m[6] ? +m[6] : null, at: Date.now() };
  const box = document.getElementById('chunky-box');
  if (box && state.params.id === ev.payload.id) box.outerHTML = chunkyHTML(ev.payload.id);
});
const hasChunky = (mods) => (mods || []).some((m) => /chunky/i.test(m.title));
function chunkyHTML(id) {
  const t = chunky[id];
  const run = t && t.state === 'running' && Date.now() - t.at < 120000;
  const paused = t && t.state === 'paused';
  return `<div class="card pad stack" id="chunky-box" style="gap:12px"><div class="spread"><div><h3>Pregenerar el mundo (Chunky)</h3>
      <span class="sm t2">Genera el terreno por adelantado para que explorar no trabe al servidor. Usa bastante CPU mientras trabaja: conviene hacerlo con pocos jugadores.</span></div>
      ${run ? `<div class="row" style="gap:8px"><button class="btn s" data-a="chunky-cmd" data-c="pause" data-tone="hold">Pausar</button><button class="btn s" data-a="chunky-cancel" data-tone="halt">Cancelar</button></div>`
        : paused ? `<div class="row" style="gap:8px"><button class="btn s" data-a="chunky-cmd" data-c="continue" data-tone="go">Seguir</button><button class="btn s" data-a="chunky-cancel" data-tone="halt">Cancelar</button></div>`
        : `<button class="btn s" data-a="chunky-start" data-tone="go" ${state.server.status === 'running' ? '' : 'disabled title="Iniciá el servidor primero"'}>Pregenerar…</button>`}</div>
    ${t ? `<div class="stack" style="gap:6px"><div class="spread sm"><span>${run ? 'Generando' : paused ? 'En pausa' : t.state === 'finished' ? 'Terminado' : 'Detenido'} · ${t.chunks.toLocaleString('es-AR')} chunks</span><span class="mono">${t.pct.toFixed(1)} %${t.eta && run ? ` · faltan ${t.eta}` : ''}</span></div>
      <div class="bar"><i style="width:${Math.min(100, t.pct)}%"></i></div></div>` : ''}</div>`;
}
A['chunky-start'] = async () => {
  const id = state.server.config.id;
  let radius = 1000;
  const ok = await dialog({ title: 'Pregenerar el mundo', confirm: 'Empezar',
    body: `<p>Se genera un cuadrado alrededor del punto de aparición. Más grande = más tiempo y más espacio en disco.</p>
      <div class="seg" id="ck-seg">${[[500, 'Chico · 500'], [1000, 'Mediano · 1.000'], [2500, 'Grande · 2.500'], [5000, 'Enorme · 5.000']].map(([r, t]) => `<button data-r="${r}" class="${r === radius ? 'on' : ''}">${t}</button>`).join('')}</div>
      <label class="field">Radio en bloques<input class="input mono" type="number" id="ck-r" min="100" max="30000" step="100" value="${radius}"></label>
      <p class="sm t2" id="ck-est"></p>`,
    onMount: (d) => {
      const est = () => { const n = Math.pow(Math.ceil((radius * 2) / 16), 2); d.querySelector('#ck-est').textContent = `≈ ${n.toLocaleString('es-AR')} chunks en el mundo principal. Se puede pausar o cancelar cuando quieras.`; };
      d.querySelectorAll('[data-r]').forEach((b) => b.addEventListener('click', () => { radius = +b.dataset.r; d.querySelector('#ck-r').value = radius; d.querySelectorAll('[data-r]').forEach((x) => x.classList.toggle('on', x === b)); est(); }));
      d.querySelector('#ck-r').addEventListener('input', (e) => { radius = Math.max(100, Math.min(30000, parseInt(e.target.value, 10) || 100)); d.querySelectorAll('[data-r]').forEach((x) => x.classList.toggle('on', +x.dataset.r === radius)); est(); });
      est();
    } });
  if (!ok) return;
  try {
    for (const cmd of ['chunky world minecraft:overworld', 'chunky shape square', 'chunky spawn', `chunky radius ${radius}`, 'chunky start']) await invoke('send_command', { id, command: cmd });
    chunky[id] = { state: 'running', chunks: 0, pct: 0, eta: null, at: Date.now() };
    toast('Pregeneración iniciada');
    const box = document.getElementById('chunky-box'); if (box) box.outerHTML = chunkyHTML(id);
  } catch (e) { fail(e); }
};
A['chunky-cmd'] = async (el) => { try { await invoke('send_command', { id: state.server.config.id, command: `chunky ${el.dataset.c}` }); } catch (e) { fail(e); } };
A['chunky-cancel'] = async () => {
  if (!(await dialog({ title: 'Cancelar la pregeneración', confirm: 'Cancelar', danger: true, cancel: 'Seguir', body: '<p>Lo ya generado queda en el mundo. Para continuar después hay que empezar de nuevo.</p>' }))) return;
  const id = state.server.config.id;
  try { await invoke('send_command', { id, command: 'chunky cancel' }); await invoke('send_command', { id, command: 'chunky confirm' }); chunky[id] = { ...(chunky[id] || {}), state: 'cancelled', at: Date.now() }; } catch (e) { fail(e); }
};
