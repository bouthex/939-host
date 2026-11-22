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
  catch (e) {
    PREP[id] = null; el.disabled = false; el.removeAttribute('aria-busy'); el.innerHTML = `${icon('play', 14)}Preparar para jugar`;
    // Sin Minecraft en el lugar de siempre: se puede elegir la carpeta (por ejemplo, otro disco).
    if (/No se encontr[oó] (el Launcher|la carpeta) de Minecraft/.test(errMsg(e))) { await chooseMinecraftDir(errMsg(e)); return; }
    fail(e); return;
  }
  PREP[id] = null;
  el.disabled = false; el.removeAttribute('aria-busy'); el.innerHTML = `${icon('check', 14)}Listo para jugar`;
  const buttons = [];
  if (r.prefsAvailable) buttons.push({ label: `Usar mis preferencias de «${r.prefsAvailable}»`, value: 'prefs' });
  if (r.previousMods) buttons.push({ label: 'Deshacer el cambio de mods', value: 'undo' });
  buttons.push({ label: 'Entendido', value: 'ok', kind: 'pri' });
  const choice = await dialog({ title: 'Listo para jugar', cancel: null, buttons,
    body: `<ul class="steps-done">
        <li>${icon('check', 14)}<span>${esc(r.label || r.versionId)}: ${r.installedVersion || r.installedLoader ? 'instalado' : 'ya estaba instalado'}</span></li>
        ${r.modsInstalled ? `<li>${icon('check', 14)}<span>${r.modsInstalled} ${r.modsInstalled === 1 ? 'mod instalado y verificado' : 'mods instalados y verificados'}</span></li>` : ''}
        ${r.serverOnly.length ? `<li>${icon('check', 14)}<span>${r.serverOnly.length} solo del servidor, no hace${r.serverOnly.length === 1 ? '' : 'n'} falta en tu PC</span></li>` : ''}
        ${r.packs?.length ? `<li>${icon('check', 14)}<span>${r.packs.length === 1 ? 'Resource pack opcional instalado y activado' : `${r.packs.length} resource packs opcionales instalados y activados`}</span></li>` : ''}
        ${r.seededFrom ? `<li>${icon('check', 14)}<span>Tus preferencias de <b>${esc(r.seededFrom)}</b> (idioma, teclas, accesibilidad, gráficos y sonido) copiadas al perfil nuevo</span></li>` : ''}
        <li>${icon('check', 14)}<span>Perfil <b>${esc(r.profileName)}</b> en el Launcher, con ${r.serversListed > 1 ? `tu lista de servidores (${r.serversListed}) y este` : 'el servidor'} en Multijugador</span></li>
      </ul>
      <p class="sm t2">Cada perfil tiene su propia carpeta: no se tocó ningún otro perfil. Tus mundos siguen en el perfil donde los creaste (no se copian ni se mueven: abrir un mundo con otra versión de Minecraft puede dañarlo).</p>
      ${r.prefsAvailable ? `<p class="sm t2">Este perfil ya existía y tiene sus propias opciones. Si querés, podés usar las de «${esc(r.prefsAvailable)}»: las actuales quedan guardadas como copia.</p>` : ''}
      ${r.previousMods ? `<p class="sm t2">La carpeta de mods que había quedó guardada como «${esc(r.previousMods)}». Podés deshacerlo.</p>` : ''}
      <p class="sm">Abrí el <b>Launcher de Minecraft</b>, elegí el perfil <b>${esc(r.profileName)}</b> y tocá <b>Jugar</b>. La primera vez el Launcher termina de bajar algunos archivos del juego.</p>
      <p class="xs t3 mono ellipsis" title="${esc(r.gameDir)}">${esc(r.gameDir)}</p>` });
  if (choice === 'prefs') {
    try { const d = await invoke('client_import_prefs', { id }); toast(d.length ? 'Preferencias copiadas al perfil' : 'El perfil ya tenía esas preferencias', 'ok', d.length ? 'Las opciones que tenía quedaron guardadas como copia (.antes-939host).' : undefined); } catch (e) { fail(e); }
  } else if (choice === 'undo') {
    try { const aside = await invoke('client_prepare_undo', { id, previous: r.previousMods }); toast('Volvió la carpeta de mods anterior', 'ok', `La que puso 939 Host quedó guardada como «${aside}». No se borró nada.`); } catch (e) { fail(e); }
  }
};

/* Carpeta de Minecraft fuera del lugar de siempre: se muestran las encontradas y se puede elegir otra. */
async function chooseMinecraftDir(why) {
  const d = await invoke('minecraft_dirs').catch(() => ({ found: [], current: null }));
  const r = await dialog({ title: 'No se encontró Minecraft', cancel: 'Cancelar',
    body: `<p>${esc(why)}</p>${d.found.length ? `<p class="sm">Encontradas en esta PC:</p><ul class="sm">${d.found.map((f) => `<li><span class="mono">${esc(f.path)}</span> · ${esc(f.source)}${f.hasLauncher ? '' : ' (sin perfiles del Launcher)'}</li>`).join('')}</ul>` : ''}
      <p class="sm t2">Si instalaste Minecraft en otra carpeta, elegila (normalmente se llama <span class="mono">.minecraft</span> y tiene <span class="mono">launcher_profiles.json</span>).</p>`,
    buttons: [{ label: 'Elegir la carpeta…', value: 'pick', kind: 'pri' }] });
  if (r !== 'pick') return;
  const p = await TAURI.dialog.open({ directory: true, title: 'Elegí la carpeta de Minecraft (.minecraft)' });
  if (!p) return;
  try { await invoke('minecraft_dir_set', { path: p }); toast('Carpeta de Minecraft elegida', 'ok', 'Volvé a tocar «Preparar para jugar».'); } catch (e) { fail(e); }
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
  const choice = await dialog({ title: 'Archivo .mrpack listo', cancel: 'Cerrar',
    body: `<p class="sm t2">Para el Launcher oficial usá «Preparar para jugar». Este archivo es para otros launchers.</p>
      <p class="sm t2 mono ellipsis" title="${esc(p.file)}">${esc(p.file)} · ${fmtBytes(p.sizeBytes)}</p>`,
    buttons: [{ label: 'Mostrar en la carpeta', value: 'reveal' }, ...(ln.length ? [{ label: `Abrir con ${ln[0]}`, value: 'open', kind: 'pri' }] : [])] });
  try {
    if (choice === 'open') await invoke('profile_open', { path: p.file, reveal: false });
    else if (choice === 'reveal') await invoke('profile_open', { path: p.file, reveal: true });
  } catch (e) { fail(e); }
};
