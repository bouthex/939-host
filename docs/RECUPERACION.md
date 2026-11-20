# Recuperación: qué hacer si algo sale mal

Guía para cualquier persona que use 939 Host. Ninguno de estos pasos necesita conocimientos técnicos ni
borrar nada a mano.

## Un mundo quedó mal o falta lo último que se jugó

1. Abrí el servidor → pestaña **Copias**.
2. Elegí un punto de restauración anterior al problema y tocá **Restaurar**.
3. Lo que había antes de restaurar no se pierde: queda como punto «Antes de restaurar» y en la Papelera.

## Al abrir, 939 Host avisa que el último cierre «no lo vio nadie»

Pasa después de un corte de luz, de cerrar 939 Host desde el Administrador de tareas con «Finalizar árbol
de procesos» o de una falla de Windows. El mundo quedó como estaba en el último guardado automático de
Minecraft.

1. Encendé el servidor y revisá el mundo.
2. Si falta algo o no abre: **Copias** → restaurar el punto anterior.
3. Si le pasás el servidor a otra PC, ese punto aparece como «tras un cierre sin confirmar»: el anterior
   sigue ahí por si hiciera falta.

## 939 Host avisa que no pudo actualizar sus datos

Se restauró solo el respaldo y todo quedó como estaba. Seguí usando la app y avisá al proyecto para
revisarlo. El respaldo queda en la carpeta de datos, en `respaldos-datos`.

## 939 Host no abre porque los datos «son de una versión más nueva»

Pasa si se instaló una versión anterior encima de una más nueva. La app no toca nada para no dañar los
datos.

- Lo más simple: volver a instalar la versión más nueva.
- Si volviste atrás a propósito: con 939 Host cerrado, en `%LOCALAPPDATA%\939Host\respaldos-datos`
  está la copia de los datos de antes de la última migración (carpeta `v<N>-<fecha>`). Copiá su contenido
  sobre `%LOCALAPPDATA%\939Host` y abrí la versión anterior. Los mundos no forman parte de esa copia
  porque nunca se migran.

## Cambié de PC o reinstalé Windows

- Instalá 939 Host. Si tenías un grupo, la nueva PC se une con un código nuevo o con el código de
  recuperación del servidor (ver [MANUAL.md](MANUAL.md)).
- Los chats de amigos estaban cifrados con la cuenta de Windows anterior: no se pueden recuperar y hay que
  volver a invitar a los amigos.

## Para el propietario del proyecto: perdí la PC desde la que publico

1. En una PC nueva, con la carpeta del propietario (sin la clave vieja, que quedó en la otra PC):
   `939publicar rotar --carpeta <carpeta> --con-recuperacion <archivo de recuperación>`.
2. Subí el `raiz-<N>.json` nuevo al lugar de distribución y volvé a autorizar la última publicación de
   cada canal. La clave de la PC perdida deja de servir para publicar en todas las apps que reciban la
   raíz nueva.
3. Si también se perdió el archivo de recuperación, no hay forma segura de recuperar la cadena: hay que
   publicar a mano una versión nueva con una identidad nueva.
