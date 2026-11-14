# Política de seguridad

## Versiones con soporte

| Versión | Soporte |
|---|---|
| 0.9.x | ✅ |
| < 0.9 | ❌ |

## Reportar una vulnerabilidad

**No abras un issue público.** Usá el reporte privado de GitHub:
[Security → Report a vulnerability](https://github.com/bouthex/939-host/security/advisories/new).

Incluí, si podés: versión afectada, pasos para reproducir, impacto y una prueba de concepto mínima.
No incluyas datos personales de terceros.

Qué esperar:

- Confirmación de recepción en un plazo de 7 días.
- Evaluación y plan de corrección; te mantenemos al tanto en el mismo aviso privado.
- Publicación coordinada del arreglo y del aviso, con crédito si lo querés.

## Alcance

Nos interesan especialmente:

- Acceso al nodo P2P sin una invitación válida, o uso de una invitación ya usada o revocada.
- Que dos PCs puedan escribir el mismo servidor a la vez (fallas del permiso de anfitrión).
- Sincronización que acepte datos alterados (los bloques se verifican por SHA-256).
- Escrituras fuera de `%LOCALAPPDATA%\939Host` o del perfil de Minecraft elegido para la reconexión.
- Filtración de claves, invitaciones o identidad en registros o informes de diagnóstico.
- Problemas en el instalador, el desinstalador o el proceso de compilación y firma.

Fuera de alcance: vulnerabilidades del propio Minecraft, de mods de terceros o de servicios externos
(Mojang, Modrinth, Adoptium); reportalas a sus responsables.

## Integridad de las descargas

Las releases se compilan en GitHub Actions desde este repositorio, con `SHA256SUMS.txt` y atestación de
procedencia. Ver [docs/INSTALACION.md](docs/INSTALACION.md#verificar-la-descarga-opcional) y
[CODE_SIGNING_POLICY.md](CODE_SIGNING_POLICY.md).
