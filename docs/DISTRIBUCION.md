# Distribución

939 Host se distribuye únicamente desde [GitHub Releases](https://github.com/bouthex/939-host/releases):
instalador NSIS por usuario y `.zip` portable, con `SHA256SUMS.txt` y atestación de procedencia. Los
usuarios no necesitan cuenta de GitHub.

## Firma

La única vía gratuita y sin suscripciones que permite que la app corra con Smart App Control activado
es la firma de una CA del Microsoft Trusted Root Program. Para eso se solicitó el programa open source de
**SignPath Foundation** (ver [firma/SIGNPATH.md](firma/SIGNPATH.md)).

Descartado:

| Opción | Motivo |
|---|---|
| Esperar reputación de SmartScreen | No hay forma de pedirla y cada versión sin firmar empieza de cero; con pocos usuarios no alcanza |
| Certificado autofirmado | Windows lo trata como sin firma; habría que instalar el certificado en cada PC |
| Certificados comerciales y Azure Artifact Signing | Pagos o con suscripción (y Artifact Signing no está disponible para personas fuera de EE. UU. y Canadá) |
| Microsoft Store | Descartado por decisión del proyecto |
| MSIX, MSI, ClickOnce | No evitan el requisito: también necesitan firma de confianza |
