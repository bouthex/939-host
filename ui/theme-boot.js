// Aplica el tema antes de pintar para que no destelle. El valor real viene de la configuración
// de 939 Host; esto es solo la última elección recordada en esta ventana.
(function () {
  var t = 'system', m = '';
  try { t = localStorage.getItem('939-theme') || 'system'; m = localStorage.getItem('939-motion') || ''; } catch (e) { /* sin almacenamiento */ }
  var dark = t === 'dark' || (t === 'system' && window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  if (m === 'reduce') document.documentElement.dataset.motion = 'reduce';
})();
