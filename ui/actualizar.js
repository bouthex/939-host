'use strict';
/* Pantalla de actualización: misma estética que la presentación (cubos blanco y negro que se arman,
   giran mientras se trabaja y se desarman al terminar). Cada texto corresponde a un estado real que
   informa el programa (`update-status`); la barra solo aparece cuando hay un porcentaje real. Cada
   texto queda a la vista al menos un instante para que nada parpadee si la descarga es muy rápida. */

const UPD_STAGE = document.querySelector('#splash .sp-stage')?.outerHTML || '';
const UPD = { el: null, auto: false, installing: false, said: '', saidAt: 0, queue: [], timer: null, v: null };
const UPD_MIN_MS = 520;

/* Los estados se muestran en orden y cada uno queda a la vista un instante (aunque la descarga
   haya sido instantánea, «Verificando…» se llega a leer). El reinicio y los errores se muestran ya. */
function updSay(text, now) {
  if (!UPD.el) return;
  const put = (t) => { const s = UPD.el?.querySelector('#upd-status'); if (s) s.textContent = t; UPD.said = t; UPD.saidAt = performance.now(); };
  if (now) { clearTimeout(UPD.timer); UPD.timer = null; UPD.queue = []; if (text !== UPD.said) put(text); return; }
  const last = UPD.queue.length ? UPD.queue[UPD.queue.length - 1] : UPD.said;
  if (text === last) return;
  UPD.queue.push(text);
  const pump = () => {
    UPD.timer = null;
    if (!UPD.queue.length) return;
    const left = UPD_MIN_MS - (performance.now() - UPD.saidAt);
    if (left > 0) { UPD.timer = setTimeout(pump, left); return; }
    put(UPD.queue.shift());
    if (UPD.queue.length) UPD.timer = setTimeout(pump, UPD_MIN_MS);
  };
  if (!UPD.timer) pump();
}

function updBar(p) {
  const wrap = UPD.el?.querySelector('.upd-bar');
  if (!wrap) return;
  const bar = wrap.querySelector('.bar'), fill = wrap.querySelector('i'), pct = wrap.querySelector('.upd-pct');
  if (p == null) { wrap.hidden = true; return; }
  wrap.hidden = false;
  if (p === 'indet') { bar.classList.add('indet'); pct.textContent = ''; return; }
  bar.classList.remove('indet');
  fill.style.width = `${Math.round(p * 100)}%`;
  pct.textContent = `${Math.round(p * 100)} %`;
}

function updActs(html) { const a = UPD.el?.querySelector('#upd-acts'); if (a) a.innerHTML = html; }

/* Abre la pantalla. `auto`: al abrir la app, sin que nadie lo pida (sigue sola hasta reiniciar). */
function openUpdateScreen(v, auto) {
  if (UPD.el) { updOnStatus(v); return; }
  const el = document.createElement('div');
  el.className = 'splash upd';
  el.id = 'upd-screen';
  el.setAttribute('role', 'dialog');
  el.setAttribute('aria-modal', 'true');
  el.setAttribute('aria-label', 'Actualización de 939 Host');
  el.innerHTML = `${UPD_STAGE}
    <div class="upd-title" id="upd-title">Nueva versión disponible</div>
    <div class="upd-ver mono">${esc(v.current)} → ${esc(v.available || '')}</div>
    <div class="sp-status upd-status" id="upd-status" aria-live="polite">&nbsp;</div>
    <div class="upd-bar" hidden><div class="bar"><i></i></div><span class="xs t3 mono upd-pct"></span></div>
    <div class="btns upd-acts" id="upd-acts"></div>`;
  document.body.appendChild(el);
  UPD.el = el; UPD.auto = !!auto; UPD.installing = false; UPD.said = ''; UPD.saidAt = 0; UPD.queue = [];
  updSay('Nueva versión disponible');
  updOnStatus(v);
}

function closeUpdateScreen() {
  const el = UPD.el;
  if (!el) return;
  UPD.el = null;
  clearTimeout(UPD.timer);
  el.classList.add('out');
  setTimeout(() => el.remove(), 520);
}

/* Cada cambio de estado del programa. */
function updOnStatus(v) {
  UPD.v = v;
  if (!UPD.el) return;
  const st = v.status;
  UPD.el.classList.toggle('err', ['error', 'bloqueada', 'sin-conexion'].includes(st));
  if (st === 'descargando') {
    updSay('Descargando actualización…');
    updBar(v.progress ?? 'indet');
    updActs('<button class="btn s ghost" data-a="upd-later">Seguir usando 939 Host</button>');
  } else if (st === 'verificando') {
    updSay('Verificando archivos…');
    updBar(1);
    updActs('');
  } else if (st === 'lista') {
    if (UPD.installing) {
      // El programa volvió a «lista» con un motivo: la instalación no pudo empezar (no se cambió nada).
      UPD.installing = false;
      updFail(v.message || 'No se pudo instalar.');
      return;
    }
    updSay('Verificando archivos…');
    updBar(1);
    updInstall();
  } else if (st === 'instalando') {
    updSay('Instalando actualización…');
    updBar('indet');
    updActs('');
  } else if (st === 'reiniciando') {
    UPD.el.classList.add('done');
    updSay('Reiniciando 939 Host…', true);
    updBar(null);
    updActs('');
  } else if (st === 'buscando') {
    updSay('Buscando actualizaciones…');
    updBar('indet');
  } else if (['error', 'bloqueada', 'sin-conexion'].includes(st)) {
    updFail(v.message || 'No se pudo actualizar.');
  } else if (st === 'al-dia') {
    updSay('939 Host está al día', true);
    updBar(null);
    setTimeout(closeUpdateScreen, 900);
  }
}

function updTitle(t) { const e = UPD.el?.querySelector('#upd-title'); if (e) e.textContent = t; }

function updFail(msg) {
  UPD.el?.classList.add('err');
  updTitle('No se pudo actualizar');
  msg = String(msg);
  updSay(msg.charAt(0).toUpperCase() + msg.slice(1), true);
  updBar(null);
  updActs('<button class="btn s pri" data-a="upd-retry">Reintentar</button><button class="btn s" data-a="upd-later">Continuar sin actualizar</button>');
}

async function updInstall() {
  if (UPD.installing) return;
  UPD.installing = true;
  try {
    const busy = await invoke('update_install');
    if (busy.length) {
      UPD.installing = false;
      updFail(`Para no interrumpir nada, la actualización espera a que termine: ${busy.join(', ')}.`);
    }
  } catch (e) {
    UPD.installing = false;
    updFail(String(e?.message || e));
  }
}

A['upd-later'] = () => { closeUpdateScreen(); toast('La actualización sigue en segundo plano', 'ok', 'Cuando esté lista, se instala desde el aviso de arriba o desde Configuración → Actualizaciones.'); };
A['upd-retry'] = async () => {
  UPD.el?.classList.remove('err');
  updTitle('Nueva versión disponible');
  updActs('');
  const v = UPD.v || {};
  if (v.status === 'lista') { updInstall(); return; }
  updSay('Buscando actualizaciones…');
  updBar('indet');
  try { updOnStatus(await invoke('update_check')); } catch (e) { updFail(String(e?.message || e)); }
};
A['upd-now'] = async () => {
  let v = null;
  try { v = await invoke('update_status'); } catch (e) { fail(e); return; }
  openUpdateScreen(v, false);
};

/* Aviso arriba cuando hay una versión lista y la app ya está en uso (no se instala sola en medio
   del trabajo). */
function paintUpdateBar(v) {
  const root = document.getElementById('updbar-root');
  if (!root) return;
  const show = v && v.status === 'lista' && !UPD.el && state.updBarHidden !== v.available;
  root.innerHTML = show ? `<div class="updbar"><span class="grow sm"><b>939 Host ${esc(v.available)}</b> está lista para instalar. Se reinicia en unos segundos; tus servidores y datos no se tocan.</span><button class="btn s pri" data-a="upd-now">Actualizar ahora</button><button class="btn s icon ghost" data-a="upd-bar-hide" aria-label="Más tarde" title="Más tarde">${icon('x', 14)}</button></div>` : '';
}
A['upd-bar-hide'] = () => { state.updBarHidden = UPD.v?.available || state.updBarHidden || 'x'; paintUpdateBar(null); };

/* Resultado de la última actualización (una sola vez). */
function showUpdateNotice(n, v) {
  if (!n) return;
  invoke('update_notice_seen').catch(() => {});
  if (n.kind === 'ok') { toast(`939 Host se actualizó a ${n.version}`, 'ok', 'Tus servidores, mundos y configuración siguen como estaban.'); return; }
  const canRetry = v && v.status === 'lista';
  dialog({
    title: 'La actualización no se completó',
    body: `<p>${esc(n.text)}</p>`,
    buttons: canRetry ? [{ label: 'Más tarde', value: 'no' }, { label: 'Reintentar', value: 'retry', kind: 'pri' }] : [{ label: 'Entendido', value: 'ok', kind: 'pri' }],
  }).then((r) => { if (r === 'retry') openUpdateScreen(v, false); });
}
