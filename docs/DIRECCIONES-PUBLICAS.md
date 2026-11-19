# Direcciones personalizadas para jugadores sin 939 Host (§16) — PENDIENTE DE INFRAESTRUCTURA

Estado: **no implementado y no prometido.** Es un requisito fundamental del producto y sigue
abierto. Este documento separa lo que existe hoy de lo que falta, y fija las condiciones que debe
cumplir una solución antes de implementarla.

## Qué existe hoy (y qué no hace)

| Pieza | Qué es | ¿Sirve sin 939 Host? | ¿Oculta la IP de la casa? |
|---|---|---|---|
| Dirección `.939host` (ej. `survival.939host`) | Nombre local en el archivo `hosts` que apunta a `127.0.0.x`; el tráfico viaja por el túnel P2P cifrado del grupo | **No**: solo resuelve en PCs con 939 Host | Sí entre integrantes (y con el modo privado, ni entre ellos) |
| Túnel P2P (iroh, QUIC/TLS 1.3) | Conexión cifrada entre PCs del grupo, directa o por relay | No | Con modo privado, sí |
| Relays de iroh | Retransmiten bytes cifrados entre dos nodos de iroh | No: un Minecraft normal no habla QUIC ni iroh | Sí, pero solo entre nodos de iroh |
| «Dirección pública» del servidor | Un texto que la persona guarda (el de su playit.gg, su dominio, etc.) y 939 Host muestra | Depende enteramente del servicio externo que la persona configuró | **939 Host no la oculta ni comprueba nada**; un dominio que apunta a la casa la revela |
| IP directa / reenvío de puertos | El jugador entra a la IP pública de la casa | Sí | **No** |
| Registro SRV | Hace que `mc.dominio.com` apunte a otro host y puerto | Sí, si el destino es alcanzable | Solo si apunta a un relay, no a la casa |

## Por qué `.939` no puede ser un dominio real

Un sufijo de primer nivel como `.939` solo funciona en cualquier PC si ICANN lo delega. La
ronda 2026 de nuevos dominios cerró el 12 de agosto de 2026, la tasa de evaluación es de
USD 227.000 por solicitud y la siguiente ronda no tiene fecha. Un `.939` hecho con el archivo
`hosts` o con un DNS propio solo funciona en PCs configuradas, que es justo lo que el requisito
descarta. **La alternativa más cercana** es un subdominio bajo un dominio público del proyecto
(por ejemplo `survival.939host.net`), que exige registrar y mantener ese dominio.

## Lo que hace falta técnicamente

Un Minecraft Java normal solo sabe abrir una conexión TCP a un nombre DNS (o seguir un registro
SRV). Para que entre sin revelar la IP de la casa hace falta **un punto de entrada en Internet con
IP pública** que no sea la casa:

1. **Dominio del proyecto** con un comodín `*.dominio` apuntando al punto de entrada.
2. **Relay TCP compatible con Minecraft** en ese punto de entrada. Lee solo el primer paquete del
   protocolo (Handshake, sin cifrar), donde el cliente escribe el nombre al que se conectó, y elige
   el servidor por ese nombre. Así funcionan proyectos conocidos como mc-router, Gate Lite o
   Infrared.
3. **Conexión del relay a la anfitriona actual** por la red cifrada que 939 Host ya tiene: la
   anfitriona (la que tiene el permiso firmado) mantiene abierta una conexión saliente al relay
   (no abre puertos del router) y el relay le reenvía cada jugador por QUIC. Cuando cambia la
   anfitriona, la nueva se registra con su permiso firmado y la dirección no cambia.
4. **Registro firmado:** solo la propietaria (lista firmada) puede reservar un nombre para su
   servidor, y solo la PC con el permiso de anfitrión vigente puede recibir sus jugadores. Así nadie
   puede robar un nombre ni hacerse pasar por anfitriona.
5. **Interruptor «Permitir jugadores sin 939 Host»** por servidor: apagado, el relay rechaza el
   nombre; encendido, acepta solo hacia ese servidor y ese puerto (no es un proxy abierto ni llega
   a otros puertos de la PC).
6. **Límites contra abuso** en el relay: conexiones por IP, tamaño del Handshake, tiempo de espera,
   tope de ancho de banda por servidor.

Casi todo lo del lado de la app se puede reutilizar (identidad, permisos firmados, túnel QUIC,
validación de nombres). **Lo que no existe es el punto de entrada.**

## Qué puede ver quien opere el relay

La IP pública de cada jugador externo, el nombre al que se conecta, cuándo y cuánto tráfico pasa,
y el contenido del juego **si el servidor de Minecraft está en modo sin conexión** (en modo online
el tráfico de Minecraft va cifrado entre el cliente y el servidor después del inicio de sesión).
No ve la IP de la casa de la anfitriona si la conexión relay→anfitriona va por un relay de iroh.

## Condiciones que fijó el propietario del proyecto

- No depender de una cuenta, una tarjeta, una clave de API ni una computadora personal suya como
  infraestructura central.
- No exigir cuentas ni suscripciones a cada usuario.
- No prometer que funciona hasta que exista una solución real, desplegada, probada con un
  Minecraft Java real desde otra red, y segura.

## Opciones investigadas (ninguna cumple todo hoy)

| Opción | Problema |
|---|---|
| Servicio de túnel de cada persona (playit.gg u otro) | Exige una cuenta por persona y su programa abierto |
| VM gratuita en la nube (por ejemplo Oracle Always Free) | Exige una cuenta con tarjeta a nombre de alguien; en 2026 Oracle redujo los límites gratuitos y puede reclamar instancias ociosas |
| Una PC del grupo como relay | Expone la IP de esa casa |
| Cloudflare Tunnel u otros proxys HTTP | El TCP genérico para clientes sin programa propio es pago o exige instalar algo en el cliente |
| Dominio `.939` propio | Imposible sin una delegación de ICANN (ver arriba) |

Caminos a evaluar a futuro: una organización sin fines de lucro o un patrocinio que opere el
relay; relays comunitarios operados por varias personas con el registro firmado ya descripto
(ninguna persona es infraestructura central); o un programa de hosting gratuito para proyectos de
código abierto.

## Pruebas obligatorias antes de declararlo listo

Las 10 del pedido original (§16 F): conexión desde Minecraft Java normal sin 939 Host, con la
opción activada y rechazo con la opción apagada, resolución DNS real, conexión escribiendo la
dirección, cambio de anfitriona conservando la dirección, servidor apagado y reactivado, caída
y reconexión del relay, comprobación de que la IP de la casa no aparece, y rechazo de conexiones
no autorizadas y de abuso del relay. Todas con clientes reales y desde otra red.
