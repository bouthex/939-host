# Cómo contribuir

¡Gracias por querer mejorar 939 Host! Estas pautas mantienen el proyecto seguro y fácil de mantener.

## Reportar errores o pedir funciones

- Buscá primero si ya existe un [issue](https://github.com/bouthex/939-host/issues).
- Usá las plantillas. Para errores, adjuntá el informe de diagnóstico (*Ajustes → Informe de diagnóstico*),
  la versión de 939 Host y de Windows, y los pasos para reproducirlo.
- **Vulnerabilidades: nunca en un issue público.** Ver [SECURITY.md](SECURITY.md).

## Cambios de código

1. Hacé un fork y una rama desde `main`.
2. Compilá y probá (ver [docs/COMPILAR.md](docs/COMPILAR.md)):
   ```bash
   cargo test --locked -p host939-core -p host939-p2p -p host939-cluster
   ```
3. Abrí un pull request explicando qué cambia y por qué. La CI tiene que pasar.
4. Todo cambio entra por pull request revisado por un mantenedor.

### Criterios

- **No romper lo que funciona.** Nada de funciones simuladas: si algo no se puede probar, decilo en el PR.
- **Nunca perder datos del usuario.** Lo que se reemplaza o borra va a la Papelera de la app con un punto
  de seguridad.
- **Sin telemetría.** Cualquier conexión de red nueva tiene que documentarse en [PRIVACY.md](PRIVACY.md)
  en el mismo PR y, si no es imprescindible, poder desactivarse.
- **Dependencias:** solo con licencias aceptadas en `about.toml` (permisivas y compatibles con MIT), y
  regenerando `THIRD_PARTY_LICENSES.html`. Sin binarios precompilados nuevos sin discutirlo antes.
- **Nada privado en el repositorio:** sin mundos, mods de terceros, registros, claves, rutas personales ni
  datos de jugadores.
- La interfaz está en español rioplatense (voseo).

## Licencia de las contribuciones

Al enviar un pull request aceptás que tu contribución se publica bajo la [licencia MIT](LICENSE) del
proyecto.

## Conducta

Ver [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
