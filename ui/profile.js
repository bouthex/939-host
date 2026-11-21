/* 939 Host — «Preparar para jugar»: instala en el Minecraft de esta PC la versión, el cargador y
   los mods que necesita el servidor, en una carpeta propia, y crea el perfil en el Launcher.
   El .mrpack sigue disponible como exportación avanzada (servidores con mods). */

function profileButton(c, v, id) {
  const sid = id || state.server?.config?.id || '';
  const mrpack = c.loader !== 'vanilla' && !id ? `<button class="btn s ghost" data-a="profile-export" title="Archivo .mrpack para otros launchers (Modrinth App, Prism…)">${icon('download', 14)}.mrpack</button>` : '';
  return `<span class="row" style="gap:6px;justify-content:flex-end"><button class="btn s pri" data-a="client-prepare" data-id="${esc(sid)}">${icon('play', 14)}Preparar para jugar</button>${mrpack}</span>`;
}

const PREP = {};
listen('client-prepare-progress', (e) => {
  const p = e.payload || {};
  PREP[p.id] = p;
  document.querySelectorAll(`[data-a="client-prepare"][data-id="${CSS.escape(p.id || '')}"]`).forEach((b) => {
    b.setAttribute('aria-busy', 'true');
    b.title = p.message || '';
    const lab = b.querySelector('.prep-msg') || Object.assign(document.createElement('span'), { className: 'prep-msg' });
    lab.textContent = p.message || 'Preparando…';
    if (!lab.parentNode) { b.textContent = ''; b.appendChild(lab); }
  });
});

A['client-prepare'] = async (el) => {
  const id = el.dataset.id;
  if (!id || PREP[id]?.running) return;
  PREP[id] = { running: true };
  el.setAttribute('aria-busy', 'true');
  el.disabled = true;
  let r;
  try { r = await invoke('client_prepare', { id }); }
  catch (e) { PREP[id] = null; el.disabled = false; el.removeAttribute('aria-busy'); el.innerHTML = `${icon('play', 14)}Preparar para jugar`; fail(e); return; }
  PREP[id] = null;
  el.disabled = false; el.removeAttribute('aria-busy'); el.innerHTML = `${icon('check', 14)}Listo para jugar`;
  await dialog({ title: 'Listo para jugar', cancel: null, confirm: 'Entendido',
    body: `<ul class="steps-done">
        <li>${icon('check', 14)}Minecraft ${esc(r.versionId)} ${r.installedVersion || r.installedLoader ? 'instalado' : 'ya estaba instalado'}</li>
        ${r.modsInstalled ? `<li>${icon('check', 14)}${r.modsInstalled} ${r.modsInstalled === 1 ? 'mod instalado y verificado' : 'mods instalados y verificados'}</li>` : ''}
        ${r.serverOnly.length ? `<li>${icon('check', 14)}${r.serverOnly.length} solo del servidor, no hace${r.serverOnly.length === 1 ? '' : 'n'} falta en tu PC</li>` : ''}
        <li>${icon('check', 14)}Perfil <b>${esc(r.profileName)}</b> en el Launcher, con el servidor en Multijugador</li>
      </ul>
      ${r.previousMods ? `<p class="sm t2">La carpeta de mods que había quedó guardada como «${esc(r.previousMods)}».</p>` : ''}
      <p class="sm">Abrí el <b>Launcher de Minecraft</b>, elegí el perfil <b>${esc(r.profileName)}</b> y tocá <b>Jugar</b>. La primera vez el Launcher termina de bajar algunos archivos del juego.</p>
      <p class="xs t3 mono ellipsis" title="${esc(r.gameDir)}">${esc(r.gameDir)}</p>` });
};

A['profile-export'] = async (el) => {
  const id = state.server.config.id;
  el?.setAttribute('aria-busy', 'true');
  let r;
  try { r = await invoke('profile_export', { id, includeManual: false }); } catch (e) { el?.removeAttribute('aria-busy'); fail(e); return; }
  el?.removeAttribute('aria-busy');
  const rep = r.report;
  if (rep.manualExcluded.length) {
    const inc = await dialog({ title: 'Mods agregados a mano', confirm: 'Incluirlos',
      body: `<p>${rep.manualExcluded.length === 1 ? 'Este mod no viene' : 'Estos mods no vienen'} de Modrinth, así que 939 Host no puede verificarlos:</p><ul class="sm">${rep.manualExcluded.map((m) => `<li class="mono">${esc(m)}</li>`).join('')}</ul>
        <p class="sm t2">Si los incluís, van copiados dentro del perfil. Compartirlos depende de la licencia de cada mod.</p>`,
      cancel: 'Dejarlos afuera' });
    if (inc) { try { r = await invoke('profile_export', { id, includeManual: true }); } catch (e) { fail(e); return; } }
  }
  const p = r.report;
  const ln = r.launchers;
  const choice = await dialog({ title: 'Archivo .mrpack listo', cancel: 'Cerrar',
    body: `<p class="sm t2">Para el Launcher oficial usá «Preparar para jugar». Este archivo es para otros launchers.</p>
      <p class="sm t2 mono ellipsis" title="${esc(p.file)}">${esc(p.file)} · ${fmtBytes(p.sizeBytes)}</p>`,
    buttons: [{ label: 'Mostrar en la carpeta', value: 'reveal' }, ...(ln.length ? [{ label: `Abrir con ${ln[0]}`, value: 'open', kind: 'pri' }] : [])] });
  try {
    if (choice === 'open') await invoke('profile_open', { path: p.file, reveal: false });
    else if (choice === 'reveal') await invoke('profile_open', { path: p.file, reveal: true });
  } catch (e) { fail(e); }
};
