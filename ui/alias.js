/* 939 Host — Direcciones propias («supervivencia.939host»): internas de cada PC con 939 Host.
   No son públicas: se dice siempre con claridad. */

async function loadAliases() { try { state.aliases = await invoke('alias_status'); } catch { state.aliases = null; } return state.aliases; }

function aliasRow(id) {
  const st = state.aliases;
  if (!st) return '';
  const it = st.items.find((x) => x.id === id);
  const hint = '<span class="xs t3">Interna: funciona en las PCs con 939 Host abierto, aunque lo aloje otra PC del grupo. Lleva también el chat de voz (Simple Voice Chat). No es una dirección pública.</span>';
  if (!it) return `<div class="kv"><span class="k">Dirección propia</span><button class="btn s ghost" data-a="alias-edit" data-id="${esc(id)}">Crear una, como ${esc(st.suggestion[id] || 'servidor')}.939host${icon('chev', 14)}</button></div>`;
  const ready = it.inHosts && it.listening;
  const status = it.error ? `<span class="chk bad" title="${esc(it.error)}">No se pudo abrir</span>`
    : !it.inHosts ? `<button class="btn s" data-a="alias-hosts" data-tone="cycle">${icon('shield', 14)}Activar (pide permiso)</button>`
    : '<span class="chk ok">Activa</span>';
  return `<div class="kv"><span class="k">Dirección propia</span><span class="stack" style="gap:4px;align-items:flex-end"><span class="row" style="gap:6px">
    <span class="mono${ready ? '' : ' t3'}">${esc(it.address)}</span>${ready ? `<button class="btn s icon ghost" data-a="copy" data-text="${esc(it.address)}" data-msg="Dirección copiada" aria-label="Copiar">${icon('copy', 14)}</button>` : ''}
    <button class="btn s icon ghost" data-a="alias-edit" data-id="${esc(id)}" aria-label="Cambiar la dirección">${icon('edit', 14)}</button>${status}</span>${hint}</span></div>`;
}

A['alias-edit'] = async (el) => {
  const id = el.dataset.id;
  const st = state.aliases || (await loadAliases());
  const cur = st?.items.find((x) => x.id === id);
  let label = cur?.label || st?.suggestion[id] || '';
  const r = await dialog({ title: 'Dirección propia', confirm: 'Guardar',
    body: `<p>Una dirección fácil de recordar para entrar desde Minecraft, sin puerto.</p>
      <label class="field">Dirección<span class="affix"><input class="input mono" id="al-in" value="${esc(label)}" maxlength="40" spellcheck="false"><span class="mono t3">.939host</span></span><span class="hint" id="al-hint">Letras sin acentos, números y guiones.</span></label>
      <div class="note">${icon('info')}<div class="grow sm"><b>Es una dirección interna.</b> Funciona en esta PC y, si el servidor es de un grupo y sos la propietaria, llega sola a las demás PCs del grupo (cada una la activa una vez con el permiso de Windows). Quien no tenga 939 Host no puede usarla, y no reemplaza a un dominio público. Para activarla, Windows pide permiso de administrador una vez para «Windows PowerShell», que es el que agrega las líneas de 939 Host al archivo hosts (no toca nada más).</div></div>`,
    buttons: [...(cur ? [{ label: 'Quitar dirección', value: 'remove' }] : []), { label: 'Guardar', value: true, kind: 'pri' }],
    onMount: (d) => {
      const i = d.querySelector('#al-in');
      i.addEventListener('input', () => { i.value = i.value.toLowerCase().replace(/[^a-z0-9-]/g, ''); label = i.value; });
      i.focus(); i.select();
    } });
  if (!r) return;
  try {
    await invoke('alias_set', { id, label: r === 'remove' ? null : label });
    await loadAliases();
    if (state.aliases?.hostsPending) await A['alias-hosts']();
    else toast(r === 'remove' ? 'Dirección quitada' : 'Dirección guardada');
  } catch (e) { fail(e); }
  tabRefresh.summary?.();
};

A['alias-hosts'] = async (el) => {
  if (el) el.setAttribute('aria-busy', 'true');
  try {
    await invoke('alias_apply_hosts');
    toast('Dirección activada en esta PC');
  } catch (e) { fail(e); }
  await loadAliases();
  tabRefresh.summary?.();
};

/* Una dirección .939host que llegó del grupo y todavía no está activa en esta PC: se avisa una
   vez por dirección (sin esto, la persona tenía que encontrarla en la pantalla del servidor). */
async function offerGroupAliases() {
  const st = await loadAliases();
  const pend = (st?.items || []).filter((x) => !x.inHosts && !x.error);
  let seen = [];
  try { seen = JSON.parse(localStorage.getItem('alias-ofrecidas') || '[]'); } catch { seen = []; }
  const nuevas = pend.filter((x) => !seen.includes(x.address));
  if (!nuevas.length) return;
  try { localStorage.setItem('alias-ofrecidas', JSON.stringify([...seen, ...nuevas.map((x) => x.address)])); } catch { /* sin almacenamiento */ }
  const r = await dialog({ title: 'Dirección del grupo', confirm: 'Activar',
    body: `<p>Tu grupo usa ${nuevas.map((x) => `<b class="mono">${esc(x.address)}</b>`).join(', ')} para entrar desde Minecraft (lleve a la PC que aloje el servidor en ese momento, también con el chat de voz).</p><p class="sm t2">Para que funcione en esta PC, Windows pide permiso una vez para «Windows PowerShell», que agrega la dirección al archivo hosts. No cambia nada más.</p>` });
  if (r) await A['alias-hosts']();
}
