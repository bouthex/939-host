# Prueba real con un amigo (dos PCs, dos conexiones distintas)

Guía corta para probar la versión 1.1.0 de prueba. Vos sos **PC 1** (propietaria); tu amigo es
**PC 2**, en su casa, con otra conexión de Internet.

## Antes de empezar (las dos PCs)

1. Bajá el instalador `939Host-1.1.0-instalador.exe` (y `SHA256SUMS.txt`) que te pasaron.
   Verificá el hash: en PowerShell, `Get-FileHash .\939Host-1.1.0-instalador.exe` tiene que dar el
   mismo valor que figura en `SHA256SUMS.txt`.
2. Instalalo. No está firmado: Windows puede mostrar «Windows protegió su PC» → *Más información* →
   *Ejecutar de todas formas*. Con Smart App Control activado no se puede abrir (no lo desactives).
3. Tener el **Launcher oficial de Minecraft** instalado y abierto al menos una vez, con una cuenta
   de Minecraft: Java Edition.

## A. Jugar con 939 Host (lo principal)

**PC 1**
1. *Crear servidor* (por ejemplo, Fabric con un par de mods de Modrinth, o Vanilla). *Iniciar* y
   esperar «Listo para jugar».
2. En el resumen, *Preparar para jugar*. Abrir el Launcher, elegir «939 Host · <servidor>» y entrar
   (el servidor ya está en Multijugador).
3. Pestaña *Grupo* → *Invitar* → *Jugador* (o *Coanfitrión* si querés probar el traspaso). Mandarle
   el código por mensaje privado. 939 Host tiene que quedar abierto mientras tu amigo lo usa.

**PC 2**
1. *Servidores de amigos* → pegar el código → *Unirme*.
2. En la tarjeta del servidor, *Preparar para jugar* (le pide la lista de mods a tu PC y deja todo
   instalado). Abrir el Launcher, elegir «939 Host · <servidor>» y entrar por la dirección
   `127.0.0.1:…` que muestra la tarjeta (ya está en Multijugador).

Para probar también: ver al otro en *Jugadores*, su inventario y sus sesiones; usar − y + en la
vida (PC 1); desconectarse y volver; reiniciar el servidor; si lo invitaste como coanfitrión,
pasarle el servidor desde *Grupo* y seguir entrando por la misma dirección.

## B. Jugar sin 939 Host, por IP (opcional)

1. **PC 1**: resumen → *Sin 939 Host* → *Configurar* → activar *Permitir conexiones sin 939 Host* y
   *Recibir jugadores de Internet en esta PC* → *Guardar*. Windows puede preguntar si 939 Host puede
   recibir conexiones: aceptá para *redes privadas*.
2. Esperar que la entrada diga **Funciona** (la prueba otra PC del grupo: tu amigo tiene que tener
   939 Host abierto) o, si no, leer el motivo (CGNAT, router sin UPnP, firewall).
3. Pasarle la dirección `IP:puerto` a alguien que **no** tenga 939 Host: Multijugador → *Agregar
   servidor*. Probar también apagando el permiso: tiene que dejar de entrar.

Si dice CGNAT o «No se llega desde Internet», probá activando la entrada en la PC de tu amigo, o
abrí a mano el puerto que indica la app.

## Si algo falla

- En la PC donde pasó: servidor → pestaña *Ajustes* → *Informe de diagnóstico* (un .zip con
  registros, sin el mundo ni claves). Anotá la hora y qué estabas haciendo.
- Los registros están en `%LOCALAPPDATA%\939Host\logs`.
