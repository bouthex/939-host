# Firma de código (Authenticode) — investigación, noviembre de 2026

Estado: **939 Host no está firmado.** Nada se compró ni se contrató. Esta página resume las opciones para
que el propietario decida; ninguna garantiza que Smart App Control (SAC) o SmartScreen dejen de avisar
desde el primer día.

## Cómo deciden Windows

- **Smart App Control**: primero consulta la reputación en la nube de Microsoft. Si la nube no sabe nada
  del archivo, mira la firma: con una firma válida de una CA del Microsoft Trusted Root Program lo deja
  correr; sin firma (o con firma inválida) lo bloquea, y no hay forma de hacer una excepción para una app
  concreta: solo apagar SAC entero. Hay reportes de 2026 de apps firmadas bloqueadas igual porque la
  reputación era mala o desconocida, así que la firma ayuda mucho pero no es una garantía.
- **SmartScreen**: la reputación se acumula por archivo y por identidad del firmante. Desde 2024 los
  certificados EV ya no saltean SmartScreen: un EV avisa igual que un OV en una versión nueva. Pagar EV solo
  por SmartScreen ya no tiene sentido.
- Desde junio de 2023 la clave privada de cualquier certificado de firma de código tiene que estar en
  hardware (token USB o HSM en la nube): ya no se puede tener un archivo `.pfx` en la PC.
- Desde 2026 la validez máxima de los certificados de firma de código se acorta (Certum indica 459 días
  desde el 27/2/2026): hay que renovar más seguido.

## Opciones

| Opción | Costo aproximado | Quién puede | Clave | Editor que ve la persona | Observaciones |
|---|---|---|---|---|---|
| **SignPath Foundation** | Gratis | Proyectos open source aceptados | En el HSM de SignPath | "SignPath Foundation" | Ya solicitado ([SIGNPATH.md](SIGNPATH.md)). Aceptación discrecional, aprobación manual de cada versión |
| **Certum Open Source Code Signing** | Desde ~69 € (con tarjeta) o más en la nube | Personas que publican software libre | Tarjeta criptográfica o nube de Certum (SimplySign) | "Open Source Developer, <nombre real>" | El certificado lleva el **nombre real** del titular: expone la identidad del propietario |
| **Azure Artifact Signing** (antes Trusted Signing) | 9,99 USD/mes | Personas: solo EE. UU. y Canadá. Organizaciones: EE. UU., Canadá, UE, Reino Unido | En Azure (sin token) | Nombre validado | Hoy no disponible para el propietario como persona fuera de esas regiones; suscripción mensual |
| **Certificado OV comercial** | 150–300 USD/año | Personas u organizaciones con identidad validada | Token o HSM en la nube | Nombre validado | Renovación más frecuente desde 2026 |
| **Certificado EV** | 400 USD/año o más | Organizaciones | Token o HSM | Nombre de la organización | Ya no mejora SmartScreen respecto de OV |
| **Microsoft Store** | Gratis para personas (desde 2025) | Personas, con verificación de identidad (documento y selfie) | Microsoft firma los paquetes MSIX | El nombre de editor elegido | Un .exe/.msi enviado a la Store **no** se vuelve a firmar: hay que firmarlo antes. Con MSIX la firma es de Microsoft, pero exige empaquetar como MSIX (ver abajo) |
| Autofirmado | Gratis | — | Propia | — | Windows lo trata como sin firma salvo que se instale el certificado en cada PC: no sirve para amigos |

## Microsoft Store con MSIX: lo que habría que revisar

Es la única vía gratuita con firma de Microsoft disponible en casi todo el mundo, pero 939 Host hace cosas
que en MSIX se comportan distinto y habría que probar una por una antes de decidir:

- Escribe datos en `%APPDATA%` y en carpetas elegidas por la persona (MSIX virtualiza parte de AppData).
- Lanza Java y un proceso guardián, y usa Job Objects (permitido en apps de escritorio empaquetadas con
  `runFullTrust`, pero hay que probarlo).
- Desinstalar una app MSIX borra sus datos virtualizados: los mundos tienen que quedar fuera de esa zona.
- La decisión anterior del proyecto fue no usar la Store; queda como opción para reconsiderar, no como
  recomendación.

## Proteger la clave (si se elige un certificado propio)

- Nunca en GitHub ni en secretos de Actions; la firma se hace en la PC del propietario o en un HSM del
  proveedor con autenticación de dos factores.
- Firmar solo artefactos cuyo SHA-256 coincida con el que produjo la CI para un commit revisado.
- Sello de tiempo (RFC 3161) en cada firma, para que siga siendo válida después de que venza el
  certificado.

## Lo que no se va a afirmar

- Que firmar elimina los avisos: la reputación se construye con el tiempo y por cada versión.
- Que SAC aceptará la app: depende de la reputación en la nube además de la firma.
- Que una opción es gratis si tiene costos de renovación o requisitos que el propietario no cumple.
