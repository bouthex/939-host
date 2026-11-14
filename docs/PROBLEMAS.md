# Solución de problemas

## Instalación y arranque

**Windows dice «Windows protegió su PC».**
La versión no está firmada todavía. *Más información → Ejecutar de todas formas*, o esperá la versión
firmada. Ver [Instalación](INSTALACION.md#firma-digital-y-smart-app-control).

**Smart App Control bloqueó 939 Host y no hay opción para continuar.**
Es lo esperado con una versión sin firmar en Windows 11. No hace falta (ni recomendamos) desactivar
Smart App Control: la firma gratuita está solicitada y la release se va a reemplazar por una firmada.

**La app no abre o aparece un error de WebView2.**
Instalá *Microsoft Edge WebView2 Runtime* desde la página oficial de Microsoft. En la versión portable,
`WebView2Loader.dll` tiene que estar en la misma carpeta que `939Host.exe`.

**«939 Host está abierto» al instalar.**
Cerralo con **Salir** (barra lateral o ícono de la bandeja), no con la X: así guarda y pasa los
servidores compartidos.

## Servidores

**«El puerto está ocupado».**
Otro programa (u otro servidor) usa ese puerto. Cambialo en *Ajustes → Red* del servidor.

**El servidor se detuvo con un error.**
Mirá la consola. Si un mod falla, la última línea con `ERROR` suele nombrarlo. Probá quitar el último mod
agregado o restaurar un punto anterior (*Restauración*).

**Falta memoria o el servidor va lento (MSPT alto).**
Subí la memoria asignada en *Ajustes* del servidor, bajá la distancia de visión o de simulación, o pasale
el servidor a una PC con mejor hardware (*Este equipo* muestra la evaluación de cada PC).

**No se puede descargar Java, Minecraft o un loader.**
Revisá la conexión. Algunos antivirus o redes de trabajo bloquean `piston-data.mojang.com`,
`api.adoptium.net` o los repositorios de Fabric/Forge/NeoForge. Fabric tiene un plan B automático.

**Un mod con Zombie Horde 1.3.1 no arranca en un mundo nuevo.**
Es un error de ese mod en servidores dedicados sin `zombie_horde.properties`. Con un mundo donde ya se
jugó con el mod, arranca.

## Jugar con amigos

**Un amigo no puede entrar.**
- Tiene que usar la dirección que le muestra **su** 939 Host (`127.0.0.1:<puerto>`, en *Red*), no tu IP.
- El Firewall de Windows tiene que permitir 939 Host en redes privadas, en ambas PCs.
- En *Grupo y anfitriona* se ve si la conexión es «directa» o «por relay». Si no conecta de ninguna forma
  y desactivaste los relays públicos, activalos en *Configuración*.

**«Lo está hosteando X».**
Solo una PC puede tener el servidor encendido. Pedile a X que te lo pase («Pasar a…») o que lo suelte.
Si X se apagó: con tres o más PCs, otra lo retoma sola en menos de un minuto; con dos, usá
**Recuperar aquí**.

**El servidor «se está mudando».**
Hay un traspaso en curso. En unos segundos se puede volver a entrar con la misma dirección.

## Pedir ayuda

1. En el servidor: *Ajustes → Informe de diagnóstico*. Genera un `.zip` local con registros y
   configuración, **sin el mundo, sin claves y sin invitaciones**.
2. Revisalo si querés y abrí un [issue](https://github.com/bouthex/939-host/issues/new/choose)
   adjuntándolo, con la versión de 939 Host y de Windows y los pasos para reproducir el problema.

Problemas de seguridad: **no** abras un issue público; seguí [SECURITY.md](../SECURITY.md).
