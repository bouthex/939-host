'use strict';
/* Configuración de 939 Host: General, Apariencia, Esta PC, Red, Privacidad y conexiones,
   Reconexión, Datos y Acerca de 939 Host. */

const SSECS = [['general', 'General'], ['look', 'Apariencia'], ['pc', 'Esta PC'], ['net', 'Red'], ['privacy', 'Privacidad y conexiones'], ['reconnect', 'Reconexión'], ['data', 'Datos'], ['about', 'Acerca de 939 Host']];
const REPO = 'https://github.com/bouthex/939-host';
let settingsTarget = null;
function goSettings(sec) { settingsTarget = sec; if (state.route === 'settings') scrollToSection(); else go('settings'); }
A['settings-section'] = (el) => goSettings(el.dataset.sec);
function scrollToSection() {
  if (!settingsTarget) return;
  const el = document.getElementById(`set-${settingsTarget}`);
  settingsTarget = null;
  el?.scrollIntoView({ block: 'start' });
}
const sw = (key, title, desc, extra = '') => `<label class="switch"><div class="grow"><b>${title}</b><span class="sm t2">${desc}</span>${extra}</div><input type="checkbox" data-s="${key}" ${state.info.settings[key] !== false && state.info.settings[key] ? 'checked' : ''}></label>`;

routes.settings = async function renderSettings() {
  const s = state.info.settings;
  const [hw, runtimes, profiles] = await Promise.all([
    state.hw ? Promise.resolve(state.hw) : invoke('hardware_snapshot').catch(() => null),
    invoke('list_runtimes').catch(() => []),
    invoke('client_profiles').catch(() => []),
  ]);
  state.hw = hw; state.clientProfiles = profiles;
  const a = hw?.assessment;
  main.innerHTML = `<div class="page">
    <div class="head"><div><h1>Configuración</h1><p class="sub">Ajustes de 939 Host en esta PC. Cada servidor tiene los suyos en su pestaña Ajustes.</p></div></div>
    <div class="settings">
      <nav class="toc" aria-label="Secciones">${SSECS.map(([k, l]) => `<button data-a="set-toc" data-v="${k}">${l}</button>`).join('')}</nav>
      <div class="stack" style="gap:32px">
        <section class="section" id="set-general"><h2>General</h2><div class="card">
          <div class="switch" style="cursor:default"><div class="grow"><b>Nombre de esta PC</b><span class="sm t2">Así la ven tus amigos en el grupo.</span></div><input class="input" data-s="nodeName" maxlength="40" value="${esc(s.nodeName)}" style="width:240px" aria-label="Nombre de esta PC"></div>
          ${sw('startWithWindows', 'Iniciar con Windows', 'Arranca en la bandeja, sin abrir la ventana, para que tus servidores compartidos estén disponibles.')}
          ${sw('closeToTray', 'Seguir en la bandeja al cerrar la ventana', 'Los servidores siguen funcionando y la interfaz se cierra por completo para liberar memoria.')}
          <div class="switch" style="cursor:default"><div class="grow"><b>Salir de 939 Host</b><span class="sm t2">Detiene todos los servidores de forma ordenada y deja de participar en el grupo.</span></div><button class="btn" data-a="quit">${icon('logout', 16)}Salir</button></div>
        </div></section>

        <section class="section" id="set-look"><h2>Apariencia</h2><div class="card">
          <div class="switch" style="cursor:default"><div class="grow"><b>Tema</b><span class="sm t2">Se recuerda en esta PC.</span></div>
            <div class="seg" role="group" aria-label="Tema">${[['light', 'sun', 'Claro'], ['dark', 'moon', 'Oscuro'], ['system', 'monitor', 'Según Windows']].map(([k, ic, l]) => `<button class="${(s.theme || 'system') === k ? 'on' : ''}" data-a="set-theme" data-v="${k}" aria-pressed="${(s.theme || 'system') === k}">${icon(ic, 15)}${l}</button>`).join('')}</div></div>
          ${sw('reduceMotion', 'Reducir animaciones', 'Quita las transiciones y los indicadores animados.')}
          <div class="switch" style="cursor:default"><div class="grow"><b>Tamaño de letra de la consola</b><span class="sm t2">Entre 11 y 18 px.</span></div>
            <div class="row" style="gap:8px"><input type="range" min="11" max="18" step="1" value="${s.consoleFontSize || 13}" data-s="consoleFontSize" data-type="int" style="width:140px" aria-label="Tamaño de letra de la consola"><span class="mono sm" id="cfs">${s.consoleFontSize || 13} px</span></div></div>
        </div></section>

        <section class="section" id="set-pc"><div class="spread"><h2>Esta PC</h2><button class="btn s" data-a="bench" id="bench-btn">${icon('gauge', 14)}Evaluar ahora (~2 s)</button></div>
          ${hw ? `<div class="kpis">
            <div class="card kpi"><span class="l">Uso del procesador</span><span class="v" id="live-cpu">—</span></div>
            <div class="card kpi"><span class="l">Memoria libre</span><span class="v" id="live-ram">—</span></div>
            <div class="card kpi"><span class="l">Rendimiento por núcleo</span><span class="v">${hw.benchmark ? hw.benchmark.cpuSingleScore : '—'}<small>${hw.benchmark ? 'puntos' : 'sin medir'}</small></span></div>
            <div class="card kpi"><span class="l">Escritura en disco</span><span class="v">${hw.benchmark?.diskWriteMbS ?? '—'}<small>MB/s</small></span></div></div>
          <div class="note ${a.rating.startsWith('Buena') ? '' : 'strong'}">${icon(a.rating.startsWith('Buena') ? 'check' : 'info')}<div class="grow"><b>${esc(a.rating)}</b>${a.reasons.map((r) => `<span class="sm t2">${esc(r)}</span>`).join('')}<span class="sm t3" id="live-reasons"></span></div></div>
          <div class="card pad"><div class="kv"><span class="k">Procesador</span><span>${esc(hw.cpuModel)}</span></div>
            <div class="kv"><span class="k">Núcleos</span><span class="mono">${hw.physicalCores ?? '—'} físicos · ${hw.logicalCores} hilos</span></div>
            <div class="kv"><span class="k">Memoria total</span><span class="mono">${fmtMB(hw.ramTotalMb)}</span></div>
            <div class="kv"><span class="k">Memoria recomendada por servidor</span><span class="mono">${fmtMB(a.recommendedServerRamMb)} · máximo ${fmtMB(a.maxServerRamMb)}</span></div>
            <div class="kv"><span class="k">Sistema</span><span>${esc(hw.os)}</span></div>
            ${hw.disks.map((d) => `<div class="kv"><span class="k">Disco ${esc(d.mount)}${d.isDataDisk ? ' · datos de 939 Host' : ''}</span><span class="mono">${esc(d.kind)} · ${d.freeGb.toFixed(0)} de ${d.totalGb.toFixed(0)} GB libres</span></div>`).join('')}</div>
          <p class="xs t3" style="margin:0">Las métricas en vivo se leen cada 5 s solo mientras esta pantalla está abierta. La evaluación corre una vez o cuando la pedís; nunca en segundo plano.</p>` : '<div class="card pad sm t2">No se pudo leer el hardware de esta PC.</div>'}
        </section>

        <section class="section" id="set-net"><h2>Red</h2><div class="card">
          ${sw('allowHosting', 'Esta PC puede ser anfitriona', 'Si lo desactivás, nunca recibe un servidor por traspaso ni por recuperación automática (igual podés jugar).')}
          ${sw('allowRelay', 'Puente para otras PCs del grupo', 'Si un amigo no llega directo al anfitrión pero sí a esta PC, el tráfico pasa por acá.')}
          </div><div class="card pad stack"><b>Orden de conexión</b><ol class="sm t2" style="margin:0;padding-left:18px;display:flex;flex-direction:column;gap:4px">
            <li>Directa: misma red, IPv6 o puertos abiertos solos por UPnP/NAT-PMP.</li><li>Atravesando los routers (el relay público solo coordina el primer contacto).</li><li>Otra PC del grupo como puente.</li><li>Relay público gratuito para los datos, solo si nada de lo anterior funciona (se configura en Privacidad).</li></ol></div></section>

        <section class="section" id="set-privacy"><h2>Privacidad y conexiones</h2>
          <p class="sm t2" style="margin:0">939 Host no tiene cuentas, publicidad ni telemetría. Estas son las únicas conexiones opcionales que hace, y podés apagar cada una.</p><div class="card">
          ${sw('skinsEnabled', 'Skins reales de los jugadores', 'Consulta a Mojang (api.mojang.com, sessionserver.mojang.com y textures.minecraft.net) solo el nombre del jugador. Se guardan 24 h en esta PC. Apagado: se muestran las iniciales.')}
          ${sw('gameAssetsEnabled', 'Imágenes originales del juego', 'Descarga desde Mojang (piston-data.mojang.com) el cliente oficial de la versión del servidor, verifica su firma SHA-1, extrae solo los íconos de ítems, corazones y efectos, y borra el resto. Quedan en esta PC; no se comparten. Apagado: se muestran nombres y formas simples.', `<span><button class="btn s ghost" data-a="assets-clear" style="margin-top:6px">${icon('trash', 14)}Borrar imágenes descargadas</button></span>`)}
          ${sw('modImagesEnabled', 'Íconos y capturas de los mods', 'Carga las imágenes de cdn.modrinth.com en la tienda de mods. Apagado: se muestra la inicial de cada mod.')}
          ${sw('allowPublicRelays', 'Relays públicos gratuitos', 'Servidores de iroh que ayudan a conectar PCs detrás de routers difíciles. Los datos van cifrados de punta a punta. Se aplica al reiniciar 939 Host.')}
          </div><button class="link sm" data-a="link" data-url="${REPO}/blob/main/PRIVACY.md" style="align-self:flex-start">Leer la política de privacidad</button></section>

        <section class="section" id="set-reconnect"><h2>Reconexión automática al jugar</h2>
          <p class="sm t2" style="margin:0">Cuando el servidor cambia de PC, los jugadores se desconectan unos segundos. Con esto activado, tu Minecraft vuelve a entrar solo: 939 Host agrega el mod AutoReconnect al perfil que elijas. Solo agrega archivos; nunca borra ni reemplaza los tuyos.</p>
          <div class="card">${profiles.length ? `<div class="list">${profiles.map((p, i) => `<div class="li"><div class="grow"><b>${esc(p.name)}</b><div class="sm t3 mono">${esc(loaderName(p.loader))} ${esc(p.mcVersion)}</div></div>
            ${p.enabled ? `<span class="chip">Activada</span><button class="btn s ghost" data-a="reconnect-off" data-i="${i}">Desactivar</button>` : `<button class="btn s" data-a="reconnect-on" data-i="${i}">Activar</button>`}</div>`).join('')}</div>`
            : '<div class="pad sm t2">No se encontraron perfiles Fabric o NeoForge en el launcher oficial de esta PC.</div>'}</div></section>

        <section class="section" id="set-data"><h2>Datos</h2><div class="card pad stack">
          <div class="kv"><span class="k">Carpeta de datos</span><span class="mono xs" style="overflow-wrap:anywhere">${esc(state.info.dataDir)}</span></div>
          <div class="btns"><button class="btn s" data-a="folder" data-global="1" data-which="data">${icon('folder', 14)}Abrir carpeta de datos</button><button class="btn s" data-a="folder" data-global="1" data-which="trash">${icon('trash', 14)}Abrir Papelera</button><button class="btn s ghost" data-a="folder" data-global="1" data-which="applogs">Registros de 939 Host</button></div>
          <div><div class="sec" style="margin:8px 0 4px">Java instalado por 939 Host</div>${runtimes.length ? runtimes.map((r) => `<div class="kv"><span class="k">Java ${r.major}</span><span class="mono sm">${esc(r.version)} · ${fmtMB(r.sizeMb)}</span></div>`).join('') : '<span class="sm t3">Todavía ninguno. Se descarga solo al crear el primer servidor.</span>'}</div>
          <p class="xs t3" style="margin:0">939 Host no modifica tu instalación de Minecraft: los mundos importados se copian. La única excepción es la reconexión automática, y solo en el perfil que actives.</p></div></section>

        <section class="section" id="set-about"><h2>Acerca de 939 Host</h2><div class="card about">
          ${CINTA(112)}
          <div class="stack" style="gap:4px"><span style="font-size:22px;font-weight:650;letter-spacing:-0.02em">939 Host</span><span class="t2" style="font-size:15px">Advanced hosting. Simplified.</span><span class="sm t3">Versión ${esc(state.info.version)} · licencia MIT</span></div>
          <div style="width:40px;height:1px;background:var(--line2)"></div>
          <span class="sm">Creado por <b>Archimak</b> · <b>Bout</b></span>
          <span class="xs t3" style="max-width:560px">939 Host no es un producto oficial de Minecraft. No está aprobado por Mojang ni Microsoft, ni asociado con ellos. Minecraft es una marca de Mojang AB.</span></div>
          <div class="card list">${[['Buscar actualizaciones', 'Página de versiones en GitHub', `${REPO}/releases`], ['Código fuente', 'GitHub', REPO], ['Licencias de terceros', '', `${REPO}/blob/main/THIRD_PARTY_LICENSES.html`], ['Política de privacidad', '', `${REPO}/blob/main/PRIVACY.md`], ['Política de firma de código', '', `${REPO}/blob/main/CODE_SIGNING_POLICY.md`]]
            .map(([t, d, u]) => `<button class="li" data-a="link" data-url="${u}"><span class="grow">${t}</span><span class="sm t3">${d}</span>${icon('external', 15)}</button>`).join('')}</div></section>
      </div></div></div>`;
  scrollToSection();
  const tick = async () => {
    if (state.route !== 'settings' || !a) return;
    try {
      const l = await invoke('hardware_live', { needMb: a.recommendedServerRamMb });
      $('#live-cpu').innerHTML = `${l.cpuUsage.toFixed(0)}<small>%</small>`;
      $('#live-ram').innerHTML = `${fmtMB(l.ramAvailableMb)}<small>de ${fmtMB(l.ramTotalMb)}</small>`;
      $('#live-reasons').textContent = l.hostReady ? 'Ahora mismo: disponible para hostear.' : `Ahora mismo: ${l.reasons.join(' ')}`;
    } catch { /* ignorar */ }
  };
  setTimeout(tick, 300);
  every(5000, tick);
};
A['set-toc'] = (el) => { el.parentElement.querySelectorAll('button').forEach((b) => b.classList.toggle('on', b === el)); document.getElementById(`set-${el.dataset.v}`)?.scrollIntoView({ behavior: 'smooth', block: 'start' }); };
A['set-theme'] = async (el) => {
  el.parentElement.querySelectorAll('button').forEach((b) => { b.classList.toggle('on', b === el); b.setAttribute('aria-pressed', b === el); });
  await saveSetting({ theme: el.dataset.v }, null);
};
main.addEventListener('change', (e) => {
  const el = e.target;
  if (!el.dataset.s || state.route !== 'settings') return;
  const val = el.type === 'checkbox' ? el.checked : el.dataset.type === 'int' ? parseInt(el.value, 10) || 0 : el.value;
  if (el.dataset.s === 'nodeName' && !String(val).trim()) { toast('El nombre no puede quedar vacío.', 'error'); el.value = state.info.settings.nodeName; return; }
  saveSetting({ [el.dataset.s]: val }, el.dataset.s === 'consoleFontSize' ? null : 'Ajuste guardado').then((ok) => {
    if (!ok && el.type === 'checkbox') el.checked = !val;
    if (el.dataset.s === 'skinsEnabled') { media.skins = {}; }
    if (el.dataset.s === 'gameAssetsEnabled') { media.assets = null; media.requested.clear(); media.sprites = {}; }
  });
});
main.addEventListener('input', (e) => { if (e.target.dataset.s === 'consoleFontSize') { $('#cfs').textContent = `${e.target.value} px`; document.documentElement.style.setProperty('--console-size', `${e.target.value}px`); } });
A.bench = async (el) => {
  el.disabled = true; el.textContent = 'Evaluando…';
  try { await invoke('run_benchmark'); toast('Evaluación completada'); } catch (e) { fail(e); }
  state.hw = null; settingsTarget = 'pc'; routes.settings();
};
A['assets-clear'] = async (el) => {
  el.disabled = true;
  try { await invoke('game_assets_clear'); media.sprites = {}; media.requested.clear(); media.assets = null; toast('Imágenes del juego borradas'); } catch (e) { fail(e); }
  el.disabled = false;
};
A['reconnect-on'] = async (el) => {
  const p = state.clientProfiles[+el.dataset.i]; if (!p) return;
  el.disabled = true; toast('Preparando la reconexión automática…');
  try { const { name, gameDir, mcVersion, loader } = p; const r = await invoke('client_reconnect_enable', { profile: { name, gameDir, mcVersion, loader } }); toast(r.message); } catch (e) { fail(e); }
  settingsTarget = 'reconnect'; routes.settings();
};
A['reconnect-off'] = async (el) => {
  const p = state.clientProfiles[+el.dataset.i]; if (!p) return;
  if (!(await dialog({ title: 'Desactivar la reconexión automática', body: '<p>Se quitan solo los archivos que agregó 939 Host (van a la Papelera de 939 Host). Lo demás de tu perfil no se toca.</p>', confirm: 'Desactivar' }))) return;
  try { const { name, gameDir, mcVersion, loader } = p; await invoke('client_reconnect_disable', { profile: { name, gameDir, mcVersion, loader } }); toast('Reconexión automática desactivada'); } catch (e) { fail(e); }
  settingsTarget = 'reconnect'; routes.settings();
};
A.quit = async () => {
  const running = (await invoke('list_servers').catch(() => [])).filter(isActive);
  if (running.length && !(await dialog({ title: 'Salir de 939 Host', body: `<p>${running.length === 1 ? 'Hay un servidor encendido' : `Hay ${running.length} servidores encendidos`}. Los compartidos pasan a otra PC del grupo que esté conectada (si está activado); si no hay ninguna, se guardan, se copian a quien se pueda y se detienen. Los demás se guardan y se detienen.</p>`, confirm: 'Salir' }))) return;
  invoke('quit_app');
};
