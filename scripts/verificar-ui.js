// Verifica los scripts de la interfaz: sintaxis y nombres globales repetidos entre archivos.
// Todos los scripts de ui/ comparten el ámbito global: un `const` repetido en dos archivos hace
// que el segundo no cargue entero (pasó con `OPS`). Uso: node scripts/verificar-ui.js
const fs = require('fs');
const path = require('path');
const dir = path.join(__dirname, '..', 'ui');
const seen = new Map();
let bad = 0;
for (const f of fs.readdirSync(dir).filter((x) => x.endsWith('.js'))) {
  const src = fs.readFileSync(path.join(dir, f), 'utf8');
  try { new Function(src); } catch (e) { console.error(`✘ ${f}: ${e.message}`); bad++; }
  for (const m of src.matchAll(/^(?:const|let|class|(?:async\s+)?function)\s+([A-Za-z_$][\w$]*)/gm)) {
    if (seen.has(m[1])) { console.error(`✘ «${m[1]}» está declarado en ${seen.get(m[1])} y en ${f}`); bad++; } else seen.set(m[1], f);
  }
}
if (bad) process.exit(1);
console.log(`✔ ${seen.size} nombres globales sin repetir; sintaxis correcta`);
