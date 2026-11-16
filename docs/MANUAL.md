# Manual de 939 Host 0.9

## 1. Instalar

1. Ejecutá `939Host-0.9.0-instalador.exe`. Se instala para tu usuario en `%LOCALAPPDATA%\Programs\939 Host` (no pide permisos de administrador) y crea accesos en el menú Inicio y el escritorio.
2. Si Windows bloquea o advierte sobre el programa, mirá [Instalación → Firma digital](INSTALACION.md#firma-digital-y-smart-app-control).
3. La primera vez que 939 Host o Java usen la red, el Firewall de Windows pregunta: **permitilo en redes privadas**. Sin eso no hay conexión directa con tus amigos.

Requisitos: Windows 10/11 de 64 bits con Microsoft Edge WebView2 (Windows 11 ya lo trae; el instalador avisa si falta). Java no hace falta: 939 Host descarga el que corresponda a cada versión de Minecraft.

**Versión portable:** descomprimí el `.zip` y abrí `939Host.exe`; `WebView2Loader.dll` tiene que quedar en la misma carpeta.

**Desinstalar:** desde Configuración de Windows → Aplicaciones, o el acceso «Desinstalar 939 Host». Tus servidores quedan en `%LOCALAPPDATA%\939Host`.

## 2. Crear un servidor

**Servidores → Nuevo servidor.** El asistente tiene cuatro pasos:

1. **Nombre, versión y loader** (Vanilla, Fabric, NeoForge o Forge). 939 Host comprueba que exista el loader para esa versión.
2. **Mundo**: nuevo (con semilla opcional) o importado desde una carpeta. **Importar siempre copia**: la carpeta original no se toca. Si el mundo está abierto en Minecraft, 939 Host lo detecta y no lo copia.
3. **Recursos y reglas**: memoria (con recomendación según tu PC), dificultad, modo, jugadores, lista blanca, distancias.
4. **Confirmar** y aceptar el EULA de Minecraft.

La instalación (Java, servidor, loader) corre en segundo plano con el progreso a la vista.

## 3. Administrar

Pestañas de cada servidor:

- **Resumen**: estado, jugadores conectados, MSPT (tiempo por tick; menos de 50 ms es fluido), memoria, cómo entrar, grupo y anfitriona.
- **Consola**: registro en vivo y comandos.
- **Jugadores**: lista blanca, operadores, baneos y expulsiones. Con el servidor encendido se aplica al instante; apagado, se guarda y se aplica al encender.
- **Mods** (Fabric/NeoForge/Forge): buscar en Modrinth, instalar con sus dependencias, actualizar, quitar (no deja quitar algo que otro mod necesita) y **Importar de una carpeta** (por ejemplo, la carpeta `mods` de tu perfil: copia los mods de servidor, omite los solo de cliente salvo que otro los necesite, y no toca la carpeta original).
- **Ajustes**: todo `server.properties` en secciones, memoria y perfil de Java, intervalos de guardado. Zona de peligro: **Mundo nuevo**, **Informe de diagnóstico**, **Borrar**.
- **Restauración**: línea de tiempo de puntos de restauración y copias `.zip`.

### Puntos de restauración

- Con el servidor encendido se toma uno automáticamente cada 15 minutos (configurable). El guardado normal de Minecraft sigue aparte.
- Son **incrementales**: solo se guardan los bloques que cambiaron (en un mundo real de 119 MB, el segundo punto ocupó 0,2 MB).
- Incluyen el mundo, los mods y sus configuraciones.
- Restaurar nunca destruye nada: lo actual queda como punto «seguridad» y en la Papelera.
- Retención: todos los de las últimas 6 h, uno por hora 48 h, uno por día 14 días, uno por semana 8 semanas. Los manuales se conservan siempre.

## 4. Jugar con amigos y compartir el servidor

### Compartir

En **Resumen → Grupo y anfitriona**, activá **Compartido**. Aparece la **dirección para jugar**: `127.0.0.1:<puerto>`. Esa dirección:

- es la que usan todos en Minecraft (Multijugador → Agregar servidor),
- **no cambia** aunque cambie la PC que hostea,
- si nadie está hosteando o el servidor se está mudando, en la lista de servidores de Minecraft aparece un aviso claro en vez de «no se pudo conectar».

### Invitar

- **Invitar coanfitriona**: esa PC guarda una copia sincronizada del servidor y puede hostearlo cuando la tuya no esté.
- **Invitar jugador**: solo juega.

Se genera un código `939-…`. Tu amigo lo pega en **Red → Unirme**. Cada código sirve para una sola PC y tu 939 Host tiene que estar abierto cuando lo use. Podés revocar códigos sin usar y quitar integrantes.

### Quién hostea

Siempre hay una sola PC con **permiso de anfitriona**. Nadie más puede encender el servidor mientras tanto: el botón Iniciar lo explica.

- **Pasar a…** (en la anfitriona): guarda y apaga acá, la otra PC copia lo que le falta (solo los cambios, verificados) y enciende. Los jugadores se desconectan unos segundos y vuelven a entrar con la misma dirección. En la prueba con Minecraft real tardó 11 s.
- **Guardar y soltar**: apaga, deja el último punto y espera a que otra PC lo copie. Después cualquiera puede encenderlo **con ese estado**. Si ninguna PC alcanzó a copiarlo, nadie puede encender con una versión vieja (salvo con «Recuperar aquí», de forma explícita).
- **Al salir de 939 Host** con un servidor compartido encendido: pasa solo a otra PC coanfitriona conectada (si está activado en las opciones del grupo). Si no hay ninguna, guarda y suelta.
- **Recuperación automática**: si la anfitriona deja de responder unos 35 s, las PCs conectadas eligen a la que tenga el estado más reciente (y mejor hardware) **por mayoría** y la levantan. Hace falta mayoría del grupo (2 de 3, 3 de 4…).
- **Con solo dos PCs no hay mayoría posible**: si la otra no responde, aparece **Recuperar aquí**. Usalo solo si sabés que la otra se apagó o se quedó sin internet; se enciende con el último estado que copió tu PC.
- **Si la anfitriona vieja vuelve** después de que otra tomó el servidor, se detiene sola y aparta su copia como punto de seguridad. Nunca se mezclan dos mundos.

### Qué se pierde y qué no

- En un **traspaso** o al **soltar**: nada.
- En una **caída** (corte de luz, PC colgada): lo jugado desde el último punto sincronizado (como máximo el intervalo de sincronización, 15 min por defecto; podés bajarlo en Ajustes → Guardado).

### Reconexión automática

**Configuración → Reconexión automática al jugar**: elegí tu perfil de Minecraft (Fabric o NeoForge) y activalo. 939 Host agrega el mod AutoReconnect (Reforged) y sus dependencias a ese perfil: solo agrega archivos, nunca borra ni reemplaza los tuyos, y si alguien lo borra por error lo vuelve a poner al abrir 939 Host. «Desactivar» quita únicamente lo que agregó.

### Conexión

Orden que se intenta: directa (misma red, IPv6, UPnP) → hole punching entre routers → otra PC del grupo como puente → relay público gratuito solo para los datos si nada funciona. Sin relays públicos (se desactivan en **Este equipo**) solo queda directa o por otra PC. En **Grupo y anfitriona** cada PC muestra si está conectada «directa» o «por relay» y la latencia.

## 5. Este equipo y configuración

- **Este equipo**: evaluación del hardware, métricas en vivo (solo con esa pantalla abierta), prueba rápida a pedido, si puede ser anfitriona y si puede hacer de puente.
- **Configuración**: nombre de la PC, iniciar con Windows (minimizado en la bandeja), cerrar a la bandeja, reconexión automática, carpetas de datos y Papelera, versiones de Java instaladas.
- **Salir** (barra lateral o bandeja): pasa o guarda los servidores de forma ordenada y libera toda la RAM y la CPU.

## 6. Migrar un mundo con mods que ya usan

Ensayado con copias (ver `docs/PRUEBAS.md`):

1. Cerrá Minecraft (el mundo no puede estar abierto).
2. **Nuevo servidor** → el nombre que quieras, la **misma versión y loader** que usa el mundo (por ejemplo 26.2 + Fabric).
3. Mundo: **Importar** → `%APPDATA%\.minecraft\saves\<tu mundo>` (se copia; el original queda igual).
4. Al terminar la instalación: **Mods → Importar de una carpeta** → `%APPDATA%\.minecraft\mods`.
5. Encender. En la copia de prueba de un modpack Fabric 26.2 arrancó en 16 s con 35 mods, MSPT 0,7 ms y sin errores.

Detalle importante: el mod Zombie Horde (1.3.1) hace fallar a un servidor dedicado si el mundo **no** tiene su archivo `zombie_horde.properties`. Un mundo donde ya se jugó con ese mod lo tiene y se copia con el mundo; en un mundo nuevo con ese mod, el servidor no arranca (es un error del mod).

## 7. Próxima versión (rama `rediseno-v3`, en pruebas)

Estas funciones todavía no están en una versión publicada.

### Cambiar la versión de Minecraft
**Ajustes → Versión de Minecraft → Cambiar versión.** 939 Host revisa qué implica el cambio para ese
servidor (Java, loader, cada mod y el mundo) y ofrece tres caminos:

- **Actualizar este servidor:** copia completa y verificada, mods actualizados, arranque de prueba y
  opción de volver atrás.
- **Crear una copia con la versión nueva:** el original no se toca.
- **Empezar de cero.**

Volver a una versión anterior con el mismo mundo no se hace sobre el servidor original, porque
Minecraft no convierte mundos hacia atrás.

### Archivos
Pestaña **Archivos** de cada servidor: explorar, crear carpetas, copiar, cortar y pegar, renombrar,
borrar, importar (también arrastrando desde Windows), exportar y buscar. Si el servidor lo aloja otra
PC del grupo, los archivos se administran allá por la conexión cifrada (solo la propietaria y las
coanfitrionas). Con el servidor encendido, el mundo se puede ver pero no modificar ni descargar. Lo
borrado o reemplazado va a la Papelera.

### Papelera
Los servidores y archivos borrados se guardan 30 días desde el borrado. Se restauran completos; si ya
hay algo con el mismo nombre, vuelven como «(restaurado)» sin pisar nada.

### Mods de rendimiento y Chunky
Al crear un servidor con mods, o desde **Mods → Rendimiento**, 939 Host propone mods de optimización
que existan para esa versión exacta (Recomendado, Máximo rendimiento o Personalizado). Con Chunky
instalado, **Mundo → Pregenerar** genera el terreno por adelantado; nunca empieza solo.

### Modpacks
**Servidores → Desde un modpack:** elegí uno de Modrinth, de CurseForge o un archivo (`.mrpack` o el
`.zip` de CurseForge). Se crea el servidor con la versión y el loader del pack y se instala solo lo que
el servidor necesita.

### Perfil para jugar
**Resumen → Preparar perfil de mods** genera un `.mrpack` en Descargas. Se abre con Modrinth App,
Prism Launcher o ATLauncher y crea un perfil aparte, sin tocar tu Minecraft de siempre.

### Direcciones `.939host`
**Resumen → Dirección propia:** una dirección como `supervivencia.939host`, sin puerto. Windows pide
permiso de administrador una vez para agregarla. Es **interna**: funciona en las PCs con 939 Host que la
crean, no es una dirección pública de Internet.

### CurseForge
CurseForge exige una clave de su API oficial para que otras apps busquen y descarguen.

1. Entrá a [console.curseforge.com](https://console.curseforge.com/) (CurseForge for Studios), creá una
   cuenta y completá el formulario para una clave de API, aceptando sus términos para terceros.
2. CurseForge revisa el pedido y avisa por correo cuando la aprueba.
3. Para usarla en tu PC: **Mods → CurseForge → Pegar la clave** (se guarda solo en esa PC).
4. Para que venga en las versiones de 939 Host: en GitHub, **Settings → Secrets and variables →
   Actions → New repository secret**, con el nombre `CURSEFORGE_API_KEY`. Las compilaciones la usan
   sin que aparezca en el código.

La clave es personal: no la compartas ni la subas al repositorio. Si el autor de un mod no permite
descargarlo desde otras apps, 939 Host no lo descarga y te ofrece abrir su página.

## 8. Problemas comunes

Ver [Solución de problemas](PROBLEMAS.md).
