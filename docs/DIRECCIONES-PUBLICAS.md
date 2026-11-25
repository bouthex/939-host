# Conexión por IP para jugadores sin 939 Host

Estado (20/11/2026): **implementado** con entradas públicas en PCs del grupo. Lo que sigue en
este documento después de «Historial» es la investigación anterior, que se conserva.

## Cómo funciona

1. **Interruptor por servidor** «Permitir conexiones sin 939 Host» (resumen del servidor →
   Sin 939 Host → Configurar). Lo decide solo la propietaria; en servidores compartidos va en la
   lista firmada (`externalAccess`), así que ninguna otra PC puede encenderlo. No se puede
   encender con el modo sin conexión de Minecraft (cualquiera entraría con el nombre de un OP).
2. **Entradas**: cada PC del grupo decide si acepta ser entrada («Recibir jugadores de Internet
   en esta PC», apagado por defecto porque muestra su IP pública). Una entrada escucha en su IP
   local, en un puerto propio por servidor (25565 por defecto), y pide al router que lo abra:
   UPnP (con el nombre «939 Host» y renovación cada 20 min) o, si no hay, NAT-PMP/PCP. Si nada
   funciona, muestra cómo abrir el puerto a mano.
3. **Reenvío a la anfitriona actual**: cada jugador que entra se lleva por el mismo camino que
   usan los jugadores con 939 Host (`route_one`: directo o por el túnel cifrado del grupo, con
   relays si hace falta). La dirección no cambia cuando cambia la PC que aloja, y una anfitriona
   detrás de CGNAT igual recibe jugadores siempre que alguna entrada sea alcanzable.
4. **Comprobación real**: otra PC del grupo en línea hace un ping de estado de Minecraft a la
   entrada (como la lista de servidores del juego) al abrirla y cada 10 minutos. La interfaz
   muestra «Funciona», «Abierta, sin comprobar», «No se llega desde Internet» o «CGNAT».
5. **Dirección para compartir**: la de una entrada comprobada y en línea (IPv4 pública:puerto).
   Si la propietaria tiene un dominio o DNS dinámico propio, puede guardarlo como «nombre propio»
   y se muestra en su lugar (939 Host no lo crea ni lo administra).

## Separación de permisos (dónde se aplica cada uno)

| Acceso | Quién | Dónde se hace cumplir |
|---|---|---|
| Jugar (Minecraft) sin 939 Host | Cualquiera con la dirección, si el interruptor está encendido | La entrada solo existe con el permiso firmado y la PC que consiente; se vuelve a mirar en cada conexión. Minecraft escucha solo en `127.0.0.1` cuando el interruptor está apagado (`server-ip`). Lista blanca y modo online los aplica Minecraft. |
| Administración desde 939 Host | Integrantes según su rol | Protocolo P2P con identidad ed25519 y lista firmada. La entrada pública no tiene ningún camino hacia él: solo reenvía bytes al puerto de Minecraft de ese servidor. |
| Alojar el servidor | Propietaria y coanfitrionas | Permiso de anfitrión firmado (época, votos); un jugador externo no es integrante. |
| Identidad de las instalaciones | Cada PC | Clave del nodo P2P; la propietaria firma la lista de integrantes. |

Saltarse el interruptor: con el interruptor apagado no hay escucha pública en ninguna PC y
Minecraft no acepta conexiones de la red. Modificar el cliente de Minecraft no da nada: del
otro lado solo está el servidor de juego. Límite inherente: un integrante del grupo, que ya
tiene acceso por el túnel, siempre podría reenviar su propio acceso a otra persona (como prestar
una cuenta); ningún sistema lo puede impedir del todo.

Límites contra abuso en cada entrada: 8 conexiones a la vez y 40 por minuto por IP, 256 en
total. Las comprobaciones que pide otra PC solo van a IPs públicas y puertos ≥ 1024, con límite
por PC (no sirven para escanear redes).

## Redes domésticas: qué funciona y qué no

- **Router con UPnP/NAT-PMP/PCP**: se abre solo. Es lo más común en routers hogareños.
- **Router sin apertura automática**: funciona si se abre el puerto a mano (la app indica cuál
  y hacia qué IP local).
- **CGNAT** (IP compartida por el proveedor, frecuente en redes móviles y algunos proveedores
  de fibra o cable): desde Internet no se llega a esa PC. Se detecta (rango 100.64/10 o IP
  privada en el lado WAN del router) y se avisa. Otra PC del grupo con conexión común puede ser
  la entrada.
- **IPv6**: si la PC tiene una IPv6 global se escucha también ahí y se muestra; depende del
  firewall del router, que muchas veces bloquea conexiones entrantes.
- **Firewall de Windows**: la primera vez Windows pregunta si 939 Host puede recibir conexiones.
  939 Host no crea reglas ni desactiva protecciones: decide la persona.
- **Cambio de IP pública**: la dirección mostrada se actualiza sola (la entrada vuelve a
  consultar al router y se comprueba cada 10 minutos), pero quien tenía la IP vieja tiene que usar
  la nueva. Para una dirección que no cambie hace falta un nombre propio (DNS dinámico o dominio).
- **Ninguna entrada alcanzable**: no hay conexión por IP. No se promete lo contrario.

## Costos y servicios externos

Ninguno. No hay servidores del proyecto, cuentas, tarjetas ni claves. Se usa el router de cada
casa (UPnP/NAT-PMP/PCP, con bibliotecas que ya estaban en el árbol de dependencias por iroh) y la
red P2P que el grupo ya usaba (relays públicos gratuitos de n0 solo cuando no hay camino directo,
como antes). Alternativas evaluadas y no incorporadas: servicios de relay de terceros (ver la
auditoría abajo), DNS dinámico obligatorio (exige cuenta: queda como «nombre propio» opcional),
y un servidor con IP fija (desde unos 4–6 EUR por mes): la misma arquitectura lo admitiría como
una PC más del grupo que solo hace de entrada, pero implica un gasto que decide el propietario.

## Pruebas

- Automatizadas con varias PCs simuladas (`multi_node::conexion_por_ip_sin_939_host`, también en
  la CI de Windows): interruptor apagado, solo la propietaria lo enciende, nunca con modo sin
  conexión, entrada en una PC que solo juega, comprobación desde otra PC, basura y pedidos de
  administración por la entrada, cambio de anfitriona con la misma dirección, reinicio de la
  anfitriona, apagado efectivo y jugadores con 939 Host sin cambios.
- **No probado**: routers reales (UPnP/NAT-PMP/PCP), conexiones desde otra red de Internet,
  CGNAT real y IPv6 real. Hay que verificarlo en la prueba real antes de publicar.

## Historial

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

## Investigación de noviembre de 2026: relays comunitarios para Minecraft

Existen servicios públicos y gratuitos, hechos justamente para esto, en los que **la PC que aloja se
conecta hacia afuera a un relay** y recibe una dirección que cualquier Minecraft Java normal puede usar,
sin cuenta y sin abrir puertos:

| Proyecto | Qué ofrece | Lo que habría que resolver |
|---|---|---|
| e4mc (mod, código abierto) | Dirección pública tipo `xxxx.e4mc.link` para un mundo abierto en LAN; los clientes no instalan nada | Es un mod del lado de quien aloja; la dirección cambia en cada sesión; servicio de terceros sin garantía; revisar sus términos antes de integrarlo desde otro programa |
| nekorelay (mod/plugin) | Relay público, gratuito y, según su página, sin cuenta ni token por ahora | Mismo tipo de dependencia; «por ahora» puede cambiar; límites de jugadores |
| StealthPipe (mod) | Relay solo para Minecraft; **el servidor del relay es código abierto (MIT)** y se puede elegir otro relay | Su relay público por omisión es de terceros; un relay propio vuelve a necesitar una máquina con IP pública |

Qué cumpliría de las condiciones: ninguna cuenta, tarjeta ni clave del propietario; la IP de la casa no
se expone (los jugadores ven la del relay); los jugadores no instalan 939 Host ni abren puertos; sin
costo.

Qué no cumple todavía o falta verificar:

- **Dependencia de un tercero** que puede cambiar sus reglas, limitarlo o cerrar. No sería
  «infraestructura central del propietario», pero sí un punto único de falla ajeno al proyecto.
- **Dirección fija**: los relays públicos dan nombres aleatorios por sesión. Con el traspaso de
  anfitriona de 939 Host la dirección cambiaría en cada traspaso, salvo que el relay permita reservar un
  nombre (no lo ofrecen hoy).
- **Qué ve el operador del relay**: la IP de cada jugador, el nombre, horarios y volumen, y el contenido
  si el servidor está en modo sin conexión.
- **Términos de uso** de cada servicio para uso desde otro programa: sin revisar todavía. No se integra
  nada sin leerlos y sin la aprobación del propietario.
- **Protocolo**: habría que implementar el cliente del relay dentro de 939 Host (o lanzar el mod en el
  servidor), probarlo con clientes reales desde otra red y comprobar que la IP de la casa no aparece.

Conclusión: es el camino más prometedor encontrado hasta ahora porque no exige infraestructura propia,
pero **no está implementado, no está probado y no se promete**. El siguiente paso, si el propietario lo
aprueba, sería una prueba aislada con un relay público y un servidor de prueba (nunca un mundo real),
revisando antes los términos del servicio.

## Auditoría de relays comunitarios (19/11/2026)

Ninguno se incorpora. Resultado de revisar código, condiciones y diseño:

| Criterio | e4mc | nekorelay | StealthPipe |
|---|---|---|---|
| Jugadores con Minecraft normal | Sí (solo quien aloja instala el mod) | Sí | **No**: exige el mod en los dos lados → descartado para este objetivo |
| Código | Cliente y servidor publicados (MIT) | Plugin/mod MIT; servidor del servicio no revisado | Relay publicado (MIT) |
| Quién opera el servicio | Una desarrolladora independiente | KuronekoServer (Japón); condiciones en japonés, con traducción automática | El autor del mod |
| Estado / disponibilidad | Sin acuerdo de servicio | Publicado como beta; avisos de caídas; plan gratuito con tope de 20 jugadores (implica planes pagos) | Relay público con límites de tamaño de paquete |
| Dirección | Aleatoria por sesión | Asignada por el servicio | — |
| Dirección persistente al cambiar de anfitriona | No | No documentado | — |
| Qué ve el operador | IP de cada jugador, nombre, horarios, volumen; contenido si el servidor está en modo sin conexión | Lo mismo | Lo mismo |
| Aislamiento y proxy abierto | Por diseño solo reenvía hacia el servidor registrado; habría que auditar el código del servidor | Afirma reenviar solo Minecraft; no auditado | Afirma que no es un proxy general |
| Si desaparece | La función deja de andar | Igual | Igual |

Riesgos comunes: el relay recibe conexiones de cualquiera y las entrega al puerto de Minecraft de la
anfitriona, así que escaneo y DoS llegan hasta la PC por ese único puerto; un tercero no puede entrar a
otras partes de la PC (solo se reenvía al servidor de Minecraft), pero sí intentar explotar fallas de
Minecraft o de mods como cualquier jugador. Con «Permitir jugadores sin 939 Host» desactivado nada de esto
existe.

**Conclusión**: ninguno cumple dirección persistente, disponibilidad razonable y condiciones revisadas.
La función **sigue pendiente** y no se ofrece. La alternativa técnicamente válida es la descripta arriba
(subdominio de un dominio del proyecto, por ejemplo `nombre.<dominio-del-proyecto>`, con un relay TCP que
lea el nombre del Handshake): requiere un servidor con IP pública (desde unos 4–6 EUR por mes, precios
de 2026) más un dominio, pagados y administrados por alguien. `.939` no puede ser un dominio público:
no existe ese dominio de primer nivel.

## Pruebas obligatorias antes de declararlo listo

Las 10 del pedido original (§16 F): conexión desde Minecraft Java normal sin 939 Host, con la
opción activada y rechazo con la opción apagada, resolución DNS real, conexión escribiendo la
dirección, cambio de anfitriona conservando la dirección, servidor apagado y reactivado, caída
y reconexión del relay, comprobación de que la IP de la casa no aparece, y rechazo de conexiones
no autorizadas y de abuso del relay. Todas con clientes reales y desde otra red.
