# Prueba de la 1.3.0 entre dos PCs (vos y Archimak)

Estas pruebas necesitan **dos PCs reales en casas distintas**. Lo que se probó antes de entregar
la 1.3.0 corrió en una sola máquina (varios procesos de 939 Host que se hablan por la red P2P) y
con servidores de Minecraft reales sin jugadores: eso **no** reemplaza esta prueba.

Usen un servidor de prueba (no el mundo de siempre). En cada paso anoten **Pasó / Falló** y, si
falló, una captura y el archivo de diagnóstico (Ajustes → Diagnóstico).

**PC 1** = la propietaria del servidor de prueba. **PC 2** = coanfitriona.

## Antes de empezar: actualizar desde la 1.2.0

1. Cierren 939 Host en las dos PCs (bandeja → *Salir*). Si había un servidor encendido, se guarda.
2. Comprueben el SHA-256 del instalador (PowerShell: `Get-FileHash .\939Host-1.3.0-instalador.exe`)
   contra el que viene en `SHA256SUMS.txt`.
3. Ejecuten `939Host-1.3.0-instalador.exe` e instalen encima de la 1.2.0: se conservan servidores,
   mundos, amigos, grupos y ajustes (los datos están en `%LOCALAPPDATA%\939Host`, el instalador no
   los toca). Windows SmartScreen avisa porque no está firmado con certificado: *Más información →
   Ejecutar de todas formas*.
   - Portable: descomprimir el `.zip` en una carpeta y abrir `939Host.exe` (usa los mismos datos).
4. Abran 939 Host y confirmen en *Ajustes → Acerca de* que dice 1.3.0.
   (La actualización automática desde la 1.2.0 funcionará con la primera versión oficial firmada;
   esta es una versión de prueba y se instala a mano.)
2. PC 1 crea un servidor de prueba (Vanilla o Fabric reciente), lo comparte y lo invita a PC 2
   como **Coanfitrión**. PC 2 se une y espera a que diga que tiene copia.

## Encender, apagar y estado en vivo

1. Con el servidor apagado y en la PC 1, **PC 2** toca *Iniciar*.
   → Se enciende **en la PC 1** (no cambia de anfitriona). PC 2 ve «Encendiéndose en la PC de …»,
   y después «Listo» en la lista de servidores y en la pantalla del servidor, sin tocar nada.
   Si tiene el sonido de inicio activado, suena una sola vez.
2. **PC 2** toca *Detener*. → La PC 1 guarda y apaga. Las dos ven «Apagado».
3. Las dos tocan *Iniciar* casi al mismo tiempo. → Se enciende una sola vez; la que perdió ve
   quién lo está encendiendo.
4. Con el servidor encendido, PC 2 mira el *Resumen*: jugadores, tiempo por tick/TPS, memoria,
   CPU, tiempo encendido y latencia de la PC 1 (cambian solos cada pocos segundos).

## Pasar el servidor

5. PC 1: *Grupo* → *Pasar a…* PC 2. → PC 2 lo enciende con el mundo de PC 1 (pongan un bloque
   antes para comprobarlo). No tiene que aparecer «época o PC no autorizadas».
6. Con el servidor en PC 2, **PC 2 cierra 939 Host** (bandeja → *Salir*). → Lo pasa a PC 1 y
   recién después se cierra. Si PC 1 no puede recibirlo, PC 2 tiene que explicar por qué.
7. Con el servidor en PC 1, **PC 1 cierra 939 Host**. → Lo pasa a PC 2 (la propietaria no
   necesita estar conectada para que el grupo siga jugando).
8. PC 2 toca *Solicitar ser anfitrión* mientras lo aloja PC 1 y está encendido. → PC 1 recibe la
   solicitud; al aceptar, pasa a PC 2.
9. *Grupo* → *Mejor PC para alojar*: después de unos minutos conectadas, cada PC tiene puntaje con
   explicación (red, equipo, subida).

## Dirección y voz

10. PC 1: *Resumen* → *Dirección propia* → por ejemplo `prueba` → *Activar (pide permiso)*.
11. PC 2 (sin hacer nada más que *Activar* cuando aparece): entra desde Minecraft a
    `prueba.939host`. Tiene que entrar **aunque el servidor esté en PC 1 o en PC 2**.
12. Con Simple Voice Chat instalado en el servidor y en los dos Minecraft: entrando por
    `prueba.939host` (o por la dirección `127.0.0.1:…` que muestra 939 Host), se escuchan entre los
    dos. Repetir después de pasar el servidor. Si tienen un tercero sin 939 Host que entre por la
    entrada pública, también tiene que tener voz (el panel de la entrada dice si el puerto UDP de
    voz quedó abierto en el router).

## Jugadores

13. PC 2 entra al juego. PC 1 abre *Jugadores* → PC 2 → *Inventario*: *Dar un objeto*
    (`minecraft:diamond`, 5), cambiar la cantidad (− y +), *Mover…* a un casillero vacío y
    *Quitar*. Cada cambio se ve en el juego de PC 2.
14. Con el servidor alojado en PC 2, PC 1 (propietaria) repite el paso anterior desde su PC.
15. PC 2 (coanfitriona) intenta cambiar la vida de alguien. → Lo rechaza: solo la propietaria.

## Versiones viejas y reparación

16. Crear un servidor **Fabric 1.14** (o 1.16.5) e *Iniciar*. → Arranca («Listo»).
17. En ese servidor, *Mods de rendimiento*: lista lo que Modrinth tiene para esa versión exacta o
    explica que no hay y por qué.

## Amigos

18. PC 1 crea un grupo de chat con PC 2 (y un tercer amigo si hay). Mensajes en las dos
    direcciones; PC 2 le cambia el nombre y PC 1 lo ve.
19. Mientras PC 2 escribe (sin mandar), PC 1 ve «… está escribiendo»; desaparece al mandar, al
    borrar el texto y, si se corta, solo a los pocos segundos.

## Corte inesperado

20. Con el servidor en PC 2, **cierren 939 Host de golpe** en PC 2 (Administrador de tareas →
    Finalizar tarea). → A los ~70 s PC 1 lo levanta sola con el último estado (tiene internet).
    Al abrir otra vez 939 Host en PC 2, esa PC ve que ya lo aloja PC 1 y no lo enciende.
21. Con el servidor en PC 2, desconecten el cable/Wi-Fi **de PC 1** dos minutos. → PC 1, sin
    internet, NO lo levanta por su cuenta (PC 2 sigue siendo la anfitriona).

## Lo corregido después de la primera prueba (24/11)

Repitan solo esto si lo anterior ya pasó.

22. **Jugadores en un servidor 26.3** (Fabric o Vanilla): PC 2 entra al juego. → En PC 1,
    *Jugadores* muestra «En línea · N min», el UUID y la vida, hambre, saturación e inventario al
    día sin tocar nada (cada ~2,5 s). El contador del *Resumen* deja de estar en 0.
23. **Arrastrar en el inventario**: PC 1 arrastra un objeto de PC 2 a un casillero vacío. → En el
    juego se mueve; si el casillero no lo acepta (por ejemplo, una espada al casco), la app lo
    dice y no se pierde nada.
24. **Pedido para alojar**: PC 2 toca *Solicitar ser anfitrión*. → PC 1 recibe el pedido y queda
    esperando su respuesta (no se acepta solo). PC 2 ve «esperando»; al aceptar, «transfiriendo» y
    «completado». El aviso «Sincronizando» se cierra solo al terminar o al fallar.
25. **Pasar la propiedad**: PC 1 → *Grupo* → *Pasar la propiedad* a PC 2. → PC 2 ve el pedido
    con *Aceptar* / *Rechazar*. Si rechaza, PC 1 sigue siendo la propietaria. Si acepta, PC 2 pasa
    a propietaria y PC 1 a coanfitriona, en las dos pantallas. Nunca quedan las dos ni ninguna.
26. **Nombres del grupo**: el nombre de PC 2 se ve en *Grupo* en las dos PCs (no «Amigo»). Si PC 2
    cambia su nombre en *Ajustes*, a los pocos segundos se ve el nuevo en PC 1.
27. **Grupo de chat**: en un grupo de chat ya creado, *Sumar amigos* agrega a otro amigo; esa
    persona ve el grupo. Seleccionar texto de un mensaje, el botón de copiar al pasar el mouse, y
    tocar una dirección (`prueba.939host`, `1.2.3.4:25565`) o un comando la copia sin ejecutarla.
28. **Direcciones**: *Resumen* → *Desde tu red* muestra la IP real de la PC con el puerto y la
    copia. *Copiar todas las direcciones* trae solo las que funcionan.
29. **`.939host` libre al borrar**: creen un servidor, pónganle `prueba2.939host`, bórrenlo y
    usen `prueba2` en otro servidor. → Se acepta.
30. **Teclas en el perfil**: con un servidor nuevo, *Preparar para jugar*. → El perfil nuevo
    tiene las mismas teclas e idioma que su Minecraft de siempre.
31. **Cambiar versión o loader**: en un servidor de prueba, *Ajustes → Cambiar versión* → elegir
    *Fabric* con la misma versión de Minecraft. → Muestra el análisis, hace la copia, instala y
    arranca. *Volver a la versión anterior* lo deja como estaba.
32. **Protección del spawn**: un servidor nuevo tiene la protección en 0 (todos construyen).
