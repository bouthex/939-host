/* 939 Host — Perfil para jugar: un .mrpack con lo que necesita el Minecraft de cada jugador.
   Se abre con Modrinth App, Prism Launcher o ATLauncher. No toca la instalación de nadie. */

function profileButton(c, v) {
  if (c.loader === 'vanilla') return '';
  return `<button class="btn s" data-a="profile-export" data-tone="cycle">${icon('download', 14)}Preparar perfil de mods</button>`;
}

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
  const choice = await dialog({ title: 'Perfil de mods listo', cancel: 'Cerrar',
    body: `<ul class="steps-done">
        <li>${icon('check', 14)}${p.fromModrinth.length} ${p.fromModrinth.length === 1 ? 'mod de Modrinth, verificado' : 'mods de Modrinth, verificados'} por hash</li>
        ${p.manualIncluded.length ? `<li>${icon('check', 14)}${p.manualIncluded.length} agregado${p.manualIncluded.length === 1 ? '' : 's'} a mano, copiado${p.manualIncluded.length === 1 ? '' : 's'} en el perfil</li>` : ''}
        ${p.serverOnlySkipped.length ? `<li>${icon('check', 14)}${p.serverOnlySkipped.length} solo del servidor, no hace${p.serverOnlySkipped.length === 1 ? '' : 'n'} falta en tu PC</li>` : ''}
      </ul>${p.manualExcluded.length && !p.manualIncluded.length ? `<p class="sm t2">Quedaron afuera ${p.manualExcluded.length} agregados a mano: sin ellos quizás no puedas entrar.</p>` : ''}
      <p class="sm t2 mono ellipsis" title="${esc(p.file)}">${esc(p.file)} · ${fmtBytes(p.sizeBytes)}</p>
      <p class="sm">${ln.length ? `Abrilo con <b>${esc(ln[0])}</b>: crea un perfil aparte con la versión, el loader y los mods, sin tocar tu Minecraft de siempre.` : 'Para usarlo hace falta un launcher que importe perfiles <b>.mrpack</b>, como Modrinth App o Prism Launcher (gratis). Crea un perfil aparte sin tocar tu Minecraft de siempre.'}</p>`,
    buttons: [{ label: 'Mostrar en la carpeta', value: 'reveal' }, ...(ln.length ? [{ label: `Abrir con ${ln[0]}`, value: 'open', kind: 'pri' }] : [])] });
  try {
    if (choice === 'open') await invoke('profile_open', { path: p.file, reveal: false });
    else if (choice === 'reveal') await invoke('profile_open', { path: p.file, reveal: true });
  } catch (e) { fail(e); }
};
