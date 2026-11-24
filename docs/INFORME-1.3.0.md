# 939 Host 1.3.0 — informe de entrega

Versión de prueba compilada por la CI desde `rediseno-v3`, commit `6c4b562` (ejecución 45327794606).
No es una release: no hay etiqueta ni publicación, y `main` está sin tocar.

| Archivo | SHA-256 |
|---|---|
| `939Host-1.3.0-instalador.exe` | `f8db328c5b96dedf91d267be9984807efe7cbdd14db6ba4945e7e48eec1c74cb` |
| `939Host-1.3.0-portable.zip` | `ae99c7929e9c98892e6b2f8cd839ab06fc681ce55f6fbe2be3e58da6317e0b56` |

Para verificarlo en Windows: `Get-FileHash .\939Host-1.3.0-instalador.exe` tiene que dar el mismo valor.
La CI también instaló este mismo instalador en Windows, lo abrió y lo desinstaló (job «Versión de prueba
instalada en Windows»).

## Qué se resolvió y con qué se comprobó

Cómo leer la columna de verificación:
- **Real**: Minecraft real (servidor y/o cliente) en Linux o Windows.
- **Dos máquinas**: dos máquinas Windows de la CI.
- **P2P local**: varios procesos de 939 Host en una misma máquina.
- **Pendiente con Archimak**: falta la prueba entre las dos PCs reales.

| Problema | Causa real y solución | Verificación |
|---|---|---|
| Fabric 1.14 no arrancaba | Java 8 no entiende «@archivo»: los argumentos se pasan sueltos | Real (Java 8 + Fabric 1.14 → «Done») |
| La reparación decía «sin problemas» | Reconoce ese error; si no lo reconoce, muestra las líneas reales del registro | Test unitario |
| Mods de rendimiento «ninguno» en 1.14 | Modrinth en toda la serie (1.14.x). Del archivo de cada mod se comprueba: que acepte la versión exacta, que corra en el servidor, sus dependencias, que esté compilado para Java 8 y que no use MixinExtras | **Real en la CI**: Fabric 1.14 con Modrinth real arrancó con 5 mods de rendimiento |
| Hambre y saturación solo se podían llenar | Efectos de Minecraft medidos en bucle hasta el valor exacto | Real: cliente de Minecraft 26.2 y 1.20.4 (CI y local) |
| Vida en vivo solo desde 1.19.4 | En versiones viejas, veneno medido en bucle (nunca mata) | Real (servidor 1.16.5); cliente 26.2 y 1.20.4 |
| Inventario en vivo | Dar, cantidad, mover (conserva encantamientos) y quitar; versiones 1.13 a 26.x | Real: cliente 26.2 y 1.20.4 |
| Ticks por tick solo en 1.20.3+ | `debug start/stop` en versiones viejas, sin dejar archivos | Real: 1.16.5, 1.18.2, 1.20.4, 26.2 |
| Traspaso: «época o PC no autorizadas» | La PC que recibe actualiza el permiso desde la de origen | P2P local |
| Iniciar exigía pedir ser anfitrión | Cualquier integrante enciende o detiene en la PC que lo aloja, también desde la lista | P2P local |
| El estado no se actualizaba en las demás PCs / «Preparando» eterno | La anfitriona avisa cada cambio; la comprobación de conexión fallida ya no traba el estado | P2P local |
| Dos PCs: sin recuperación si la anfitriona se cae | Lo levanta la otra si tiene internet; la que vuelve aparta su copia (no se pierde) | P2P local (con y sin internet) |
| `939z.939host` no funcionaba en la otra PC | La dirección viaja al grupo y la app ofrece activarla | Pendiente con Archimak |
| Voz | Por `.939host`, por `127.0.0.1` y por la entrada pública (UDP abierto en el router por UPnP) | Túnel de voz: P2P local. Entre dos PCs: pendiente con Archimak |
| Estadísticas remotas | Jugadores, ms por tick, TPS, memoria, CPU, tiempo encendido y latencia, medidos por la anfitriona | P2P local + interfaz |
| Administrar jugadores con el servidor en otra PC | La ficha y la lista se piden a la anfitriona; los cambios, solo la propietaria (la anfitriona lo vuelve a comprobar) | P2P local |
| Recomendación de anfitriona | Red medida entre PCs, equipo y subida | Test unitario |
| Grupos de chat y «escribiendo» | Grupos, nombre, salida; «escribiendo» que vence solo | 3 procesos P2P |

Todo lo anterior a esta etapa sigue pasando: unas 140 pruebas unitarias, las pruebas de grupo y de
seguridad, la interfaz real en Windows, las dos PCs Windows de la CI, las actualizaciones firmadas y el
cliente real.

## Lo que falta

- **La prueba entre las dos PCs con Archimak**, siguiendo `docs/PRUEBA-1.3.0-DOS-PCS.md` (incluye la
  actualización desde la 1.2.0).
- **Edición del inventario con el jugador desconectado**: solo se puede ver; se edita con el jugador
  conectado.
- **Permiso de Windows para `.939host`**: cada PC lo acepta una vez. Sin permiso de administrador no hay
  otra forma de que Windows resuelva un nombre propio.
- **Publicación oficial firmada**: requiere tu autorización. Desde la 1.2.0, esta versión de prueba se
  instala a mano, encima, conservando los datos.
