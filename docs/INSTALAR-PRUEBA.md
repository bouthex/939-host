# Versión de prueba de 939 Host: instalación y guías

Versión de desarrollo de la rama `rediseno-v3`. **No es una release publicada.** Sale de la ejecución
«Pruebas en Windows» de GitHub Actions, trabajo «Versión de prueba (compilación de entrega)», artefacto
**`939Host-version-de-prueba`**: se compila igual que una release (sin las herramientas de prueba de la
CI), se revisa que no tenga secretos, datos personales ni el puerto de depuración, y en Windows se
instala, se abre y se desinstala automáticamente antes de dártela.

El artefacto trae `VERSION-DE-PRUEBA.txt` con el identificador (`0.9.0+prueba.<commit>`), la rama, el
commit exacto, el enlace a la ejecución y los SHA-256. **Si el commit no coincide con el del informe, no
lo instales.** (El artefacto `939Host-windows-prueba` es otro: la compilación con herramientas de prueba
que usa la CI; no es para sus PCs.)

## Requisitos

- Windows 10 u 11 de 64 bits, con Microsoft Edge WebView2 (viene en Windows 11 y en Windows 10
  actualizado; si falta, el instalador avisa).
- **Java: no hace falta instalar nada.** 939 Host descarga solo el Java que pide cada versión de Minecraft
  (Eclipse Temurin, verificado) en su propia carpeta. No usa ni cambia otro Java de la PC.
- Unos 4 GB de RAM libres para alojar un servidor con mods (menos para Vanilla).
- Conexión a internet para descargar Minecraft, Java y mods la primera vez.

## Sin firma digital: qué significa

**El instalador y el programa no están firmados.** Conseguir una firma confiable sin pagar ni poner el
nombre real del propietario dentro del programa depende de que SignPath acepte el proyecto (pendiente).
Consecuencias, sin tocar ninguna protección de Windows:

- **SmartScreen** muestra «Windows protegió tu PC» / «Editor desconocido». Si verificaste el SHA-256,
  «Más información» → «Ejecutar de todos modos» es seguro para este archivo.
- **Smart App Control** (Windows 11, activo sobre todo en instalaciones nuevas) bloquea apps sin firma que
  no conoce, **sin opción de continuar**. En esa PC 939 Host no se puede usar por ahora. **No lo apaguen.**
  Para saber si está activo: Seguridad de Windows → Control de aplicaciones y navegador → Smart App Control.
- **Defender** puede analizar el archivo al descargarlo: es normal, no hay que agregar exclusiones.
- **Firewall**: al abrir por primera vez puede preguntar si 939 Host (y después Java) pueden usar la red.
  Permitilo en redes **privadas**.

## 1. Instalar (en cada PC)

1. Descargá el artefacto `939Host-version-de-prueba` (lo pasa el propietario) y descomprimilo.
2. Recomendado: verificá el archivo. En PowerShell, en esa carpeta:
   `Get-FileHash .\939Host-0.9.0-instalador.exe -Algorithm SHA256`
   Tiene que dar lo mismo que la línea del instalador en `SHA256SUMS.txt` y en el informe.
3. Abrí el instalador. No pide permisos de administrador; se instala solo para tu usuario.
4. Si ya tenías 939 Host, se instala encima y conserva servidores, mundos, amigos e identidad de la PC.
   **Todas las PCs del grupo tienen que tener esta misma versión.**

**Portable** (sin instalar): el `.zip` se descomprime en una carpeta propia y se abre `939Host.exe`. Usa la
misma carpeta de datos que la instalada: no conviene tener las dos a la vez.

## 2. Crear un servidor de prueba

1. **Crear servidor** → elegí la versión de Minecraft y el tipo (Vanilla, Fabric, Forge o NeoForge). La
   app avisa si ese tipo no existe para esa versión.
2. Memoria: tocá la recomendada.
3. Mundo: **Mundo nuevo**, o **Usar una copia de un mundo** (elegís la carpeta que tiene `level.dat`; 939
   Host la **copia** y nunca toca el original). Para la primera prueba, mejor un mundo nuevo.
4. Aceptá el EULA de Minecraft → **Crear**. La primera vez descarga Java y el servidor (unos minutos).
5. **Iniciar**. Cuando diga «Listo para jugar», en Minecraft agregá la dirección que muestra la app.

## 3. Invitar a otra PC

1. En la PC que aloja: servidor → pestaña **Grupo** → activar **Compartir con el grupo**.
2. **Invitar coanfitrión** (puede alojar y administrar) o **Invitar jugador** (solo juega). Copiá el
   código y pasáselo por chat. Sirve una sola vez y vence en 24 horas.
3. En la otra PC: **Servidores de amigos** → **Unirme con un código** → pegarlo.
4. Los dos agregan en Minecraft la dirección que muestra 939 Host (`127.0.0.1:<puerto>`). Esa dirección
   no cambia aunque el servidor se mude de PC.

En **Grupo** se ve si la conexión es «directa» o «por relay» (el relay solo reenvía tráfico cifrado).

## 4. Pasar el anfitrión a otra PC

- Desde la anfitriona: **Pasar a otra PC…** y elegir la PC.
- O desde la otra PC (coanfitriona): **Solicitar ser anfitrión**; en la anfitriona aparece el aviso →
  **Revisar** → aceptar.
- La app guarda, copia lo que cambió, apaga en una PC y enciende en la otra. Quien juega ve «el servidor
  se está mudando» y vuelve a entrar con la misma dirección. Nunca queda encendido en dos PCs.
- Si la anfitriona se apaga de golpe: con 3 o más PCs, otra toma el servidor sola en menos de un minuto;
  con 2 PCs, en la otra usar **Recuperar aquí**.

## 5. Proteger y recuperar los mundos

- **Nunca prueben con el mundo 939 Z original.** Usen un servidor nuevo o una copia.
- Antes de algo arriesgado (traspasos, cortar la red, reinstalar): **Copias** → **Crear punto**.
- Para volver atrás: **Copias** → elegir un punto → **Restaurar**. Lo que había queda como punto «Antes de
  restaurar» y en la Papelera (30 días).
- Si al abrir aparece que el último cierre «no lo vio nadie» (corte de luz, cierre forzado), revisá el
  mundo; en Copias está el punto anterior.
- Para cerrar 939 Host del todo: ícono de la bandeja → **Salir de 939 Host…** (guarda y pasa los
  servidores compartidos). Cerrar la ventana con la X **no** detiene los servidores: siguen en la bandeja.
- No borren a mano `%LOCALAPPDATA%\939Host`: ahí están mundos, puntos e identidad de la PC.
- Más casos en [RECUPERACION.md](RECUPERACION.md).

## 6. Desinstalar

1. Cerrá 939 Host desde la bandeja (**Salir**). Si sigue abierto, el desinstalador lo avisa y no sigue.
2. Configuración de Windows → Aplicaciones → 939 Host → Desinstalar (o «Desinstalar 939 Host» en el
   menú Inicio).
3. Por defecto **conserva** mundos, copias, puntos y configuración (reinstalar los recupera). Para borrar
   también los datos, el desinstalador lo pregunta dos veces.
4. Minecraft y `.minecraft` no se tocan nunca.
5. Si al principio Windows te preguntó por el acceso de red de 939 Host, puede quedar **su** regla de
   Firewall para `939Host.exe` (la crea Windows, no 939 Host, y solo se borra con permisos de
   administrador). No abre nada: apunta a un programa que ya no existe. Si querés quitarla: Seguridad de
   Windows → Firewall y protección de red → Configuración avanzada → Reglas de entrada.

## Si algo falla

- Servidor con error → **Reparar**: revisa puerto, Java, memoria, EULA, mods y mundo, y pide permiso antes
  de cambiar algo importante; todo se puede deshacer.
- Ajustes del servidor → **Informe de diagnóstico** → **Generar informe**: un .zip sin el mundo, sin claves,
  sin IPs ni rutas personales, para mandar al propietario.
- Anoten qué hicieron, en qué PC y a qué hora: ayuda a encontrar el problema en los registros.

## Actualizaciones

Desde la 1.1.0, 939 Host se actualiza solo con cada versión **oficial** que publique el proyecto
(Configuración → Actualizaciones muestra el estado). Las versiones de prueba como esta no se reciben
por actualización: se instalan encima, a mano, igual que esta, y se conservan todos los datos.
