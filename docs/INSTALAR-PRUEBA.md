# Instalar la versión de prueba en las tres PCs

Versión de desarrollo de la rama `rediseno-v3`. No es una release publicada: sale de la ejecución
«Pruebas en Windows» de GitHub Actions (artefacto `939Host-windows-prueba`).

## Antes de empezar

- **No hace falta cerrar Minecraft ni tocar `.minecraft`.** 939 Host usa su propia carpeta
  (`%LOCALAPPDATA%\939Host`) y su propio Java. No lee ni modifica la instalación del juego.
- **No uses el mundo 939 Z para probar.** Creá un servidor nuevo, o importá una *copia* del mundo
  (939 Host siempre copia; el original no se modifica).
- Si alguien tiene instalada la 0.9.0, se puede instalar encima: conserva los servidores y la
  identidad de la PC. **Las tres PCs tienen que tener esta misma versión** para pasarse el servidor.

## Instalar (en cada PC)

1. Conseguir el instalador: `939Host-prueba-<commit>-instalador.exe` (el que se pasa por el chat)
   o, desde GitHub Actions → «Pruebas en Windows» → artefacto `939Host-windows-prueba`,
   `939Host-0.9.0-instalador.exe` (la versión interna sigue siendo 0.9.0).
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
