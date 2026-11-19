# Instalar la versión de prueba en las tres PCs

Versión de desarrollo de la rama `rediseno-v3`. **No es una release publicada.** Sale de la ejecución
«Pruebas en Windows» de GitHub Actions, trabajo «Versión de prueba (compilación de entrega)», artefacto
**`939Host-version-de-prueba`**: se compila igual que una release (sin las herramientas de prueba de la
CI), se revisa que no tenga secretos, datos personales ni el puerto de depuración, y en Windows se
instala, se abre y se desinstala automáticamente antes de dártela.

El artefacto trae `VERSION-DE-PRUEBA.txt` con el identificador (`0.9.0+prueba.<commit>`), la rama, el
commit exacto, el enlace a la ejecución y los SHA-256. **Si el commit no coincide con el que te pasaron
en el informe, no lo instales.** (El artefacto `939Host-windows-prueba` es otro: es la compilación con
herramientas de prueba que usa la CI; no es para instalar en sus PCs.)

## Antes de empezar

- **No hace falta cerrar Minecraft ni tocar `.minecraft`.** 939 Host usa su propia carpeta
  (`%LOCALAPPDATA%\939Host`) y su propio Java. No lee ni modifica la instalación del juego.
- **No uses el mundo 939 Z para probar.** Creá un servidor nuevo, o importá una *copia* del mundo
  (939 Host siempre copia; el original no se modifica).
- Si alguien tiene instalada la 0.9.0, se puede instalar encima: conserva los servidores y la
  identidad de la PC. **Las tres PCs tienen que tener esta misma versión** para pasarse el servidor.

## Instalar (en cada PC)

1. Conseguir el instalador `939Host-0.9.0-instalador.exe` del artefacto `939Host-version-de-prueba`
   (la versión interna sigue siendo 0.9.0; la identidad real está en `VERSION-DE-PRUEBA.txt`).
2. Verificar el archivo (opcional, recomendado). En PowerShell, dentro de la carpeta:
   `Get-FileHash .\<nombre-del-instalador>.exe -Algorithm SHA256`
   El resultado tiene que coincidir con la línea correspondiente de `SHA256SUMS.txt`.
3. Abrir el instalador. No pide permisos de administrador.
4. **Aviso de SmartScreen.** Como el archivo no tiene firma digital, Windows muestra «Windows
   protegió tu PC» / «Editor desconocido». Es esperable. Si verificaste el SHA-256, elegí
   «Más información» → «Ejecutar de todos modos». No cambies ninguna configuración de Windows.
5. Al abrir por primera vez, el Firewall de Windows pregunta si 939 Host puede usar la red.
   Permitilo en redes **privadas** (es lo que usa la conexión entre PCs).

### Si aparece «Smart App Control bloqueó…»

Smart App Control (Windows 11, activo sobre todo en instalaciones nuevas) bloquea apps sin firma
que no conoce y **no ofrece «ejecutar de todos modos»**. No hay forma gratuita de evitarlo sin
firma digital, y **no recomendamos apagarlo**: es una protección del sistema (y en versiones
anteriores a Windows 11 25H2, una vez apagado no se puede volver a encender sin reinstalar).

En esa PC, por ahora, 939 Host no se puede usar, y **tampoco puede entrar al servidor del grupo**:
la dirección `127.0.0.1:<puerto>` funciona gracias a 939 Host en cada PC. Es el bloqueo real más
importante de distribuir sin firma. La alternativa sin tocar la seguridad de esa PC es que quien
aloja abra el puerto en su router y le pase su IP pública (fuera de 939 Host). Para saber si está activo: Seguridad de Windows → Control de aplicaciones
y navegador → Smart App Control.

### Portable (sin instalar)

El `.zip` portable: descomprimir en una carpeta propia (por ejemplo `Documentos\939 Host`)
y abrir `939Host.exe`. Usa la misma carpeta de datos que la instalada, así que no conviene tener
las dos a la vez. Los mismos avisos de SmartScreen y Smart App Control aplican.

## Primer uso entre las tres PCs

1. **PC que va a alojar primero:** Crear servidor → elegir versión y tipo (Vanilla, Fabric, Forge
   o NeoForge) → memoria recomendada → mundo nuevo o copia → aceptar el EULA → Crear. Esperar la
   descarga y tocar **Iniciar**.
2. Pestaña **Grupo** → activar **Compartir con el grupo** → **Invitar coanfitrión** (puede alojar) o
   **Invitar jugador** (solo juega). Copiar el código.
3. **En las otras dos PCs:** Servidores de amigos → **Unirme con un código** → pegar el código.
4. **Jugar:** cada uno agrega en Minecraft la dirección que muestra 939 Host
   (`127.0.0.1:<puerto>`). Esa dirección no cambia aunque el servidor se mude de PC.
5. **Pasar el anfitrión:** en la PC que quiere alojar, **Solicitar ser anfitrión**; en la PC
   anfitriona aparece el aviso → **Revisar** → Aceptar. Los jugadores ven «el servidor se está
   mudando» y vuelven a entrar por la misma dirección.

## Si algo falla

- En la configuración del servidor, **Informe de diagnóstico → Generar informe** arma un .zip con
  registros y configuración (sin el mundo ni claves) para pedir ayuda.
- En el servidor con error, **Reparar** revisa puerto, Java, memoria, EULA, mods y mundo, y pide
  autorización antes de cambiar algo importante. Todo cambio se puede deshacer.
- Si la PC anfitriona se apaga de golpe, otra coanfitriona toma el servidor sola en menos de un
  minuto (con al menos 3 PCs en el grupo). Con 2 PCs, usar **Recuperar aquí**.

## Proteger los mundos durante la prueba

- Antes de probar algo arriesgado (traspasos, cortes, reinstalar), en **Copias** tocá **Crear punto**.
- Probá con un servidor nuevo o con una copia importada; **nunca con el mundo 939 Z original**.
- Si 939 Host avisa al abrir que el último cierre «no lo vio nadie» (corte de luz, cierre forzado),
  revisá el mundo antes de seguir; en Copias está el punto anterior.
- No borres la carpeta `%LOCALAPPDATA%\939Host` a mano: ahí están mundos, puntos y la identidad de la PC.

## Desinstalar

- Configuración de Windows → Aplicaciones → 939 Host → Desinstalar (o «Desinstalar 939 Host» en el menú
  Inicio). Cerrá 939 Host antes (desde la bandeja: «Salir»); si está abierto, el desinstalador lo avisa
  y no sigue.
- Por defecto **conserva** mundos, copias, puntos y configuración: reinstalar los recupera.
- Para borrar también los datos, el desinstalador lo pregunta dos veces. Hacelo solo si ya no necesitás
  esos mundos (o tenés copia).
- Minecraft y `.minecraft` no se tocan nunca, ni al instalar ni al desinstalar.
