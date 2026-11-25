# Lenguaje visual de 939 Host

Criterios para todo el trabajo de interfaz. Las referencias externas (microinteracciones e
iconografía monocromática) se usan como inspiración, nunca como copia: 939 Host tiene su propio
lenguaje, monocromático y profesional, con guiños sutiles a la geometría de bloques.

## Principios

- **Monocromático.** Blanco, negro y grises; los estados se distinguen por forma y texto, nunca solo por color.
- **Bloques, no juguetes.** Ángulos rectos, escalones y proporciones de vóxel como detalle, igual que el logo
  Cinta. Nada de texturas de Minecraft, colores saturados ni tipografías pixeladas en la interfaz.
- **Complejo por dentro, simple por fuera.** La interfaz dice qué pasa en palabras simples; los detalles técnicos
  quedan a un clic.

## Íconos («Bloque»)

- Grilla de 20 × 20, área útil de 3 a 17, trazo de 1,6 con extremos y uniones redondeadas.
- Geometría ortogonal; diagonales a 45° solo cuando la forma lo pide; curvas solo donde mejoran el
  reconocimiento en 16 px (búsqueda, reloj, reiniciar).
- Detalles en «píxel» (cuadrados de 2–3 unidades) para estados: luces del rack, centro del engranaje.
- Los símbolos de PC (bloque, pico, antorcha, hoja, cofre, gema, faro, montaña, luna, rayo, ola,
  estrella) siguen las mismas reglas y se muestran en una baldosa cuadrada.

## Movimiento

- **Solo `transform` y `opacity`** (y ancho en indicadores): nunca se anima algo que obligue a recalcular el diseño.
- Duraciones: 120 ms (respuesta al tocar), 200 ms (cambios de estado), 320 ms (desplazamientos).
- Curvas: salida suave para entrar; un leve «encastre» (sobrepaso mínimo) cuando algo llega a su lugar.
- **Indicadores que se deslizan** al elemento activo: barra lateral, pestañas y segmentados.
- **Carga «escalón»**: tres bloques que se encienden en escalera para estados en curso (iniciando,
  guardando, reiniciando, sincronizando, esperando una respuesta).
- **Indicador circular** para progreso con porcentaje (instalación, mods, descargas).
- **Encastre** al cambiar de estado o terminar bien: el ícono aparece con un pequeño salto y un halo.
- Pantallas y pestañas entran con un fundido corto de 6 px; diálogos y avisos, igual.
- **Reducir movimiento**: si Windows tiene desactivadas las animaciones, o el usuario lo pide en
  Configuración → Apariencia, no se anima nada; la información se mantiene igual.
