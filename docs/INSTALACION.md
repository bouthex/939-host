# Instalación

## Descargar

Entrá a la [última versión](https://github.com/bouthex/939-host/releases/latest) y bajá uno de estos
archivos (no hace falta cuenta de GitHub):

| Archivo | Cuándo |
|---|---|
| `939Host-<versión>-instalador.exe` | Lo recomendado. Instala para tu usuario, con accesos en Inicio y escritorio |
| `939Host-<versión>-portable.zip` | Para usar sin instalar (por ejemplo desde un pendrive). Descomprimí todo y abrí `939Host.exe` |

Requisitos: Windows 10 u 11 de 64 bits, Microsoft Edge WebView2 (incluido en Windows 11 y en
Windows 10 actualizado), unos 4 GB de RAM libres para hostear un servidor con mods. Java y el servidor
de Minecraft se descargan solos.

## Instalar

1. Ejecutá el instalador. No pide permisos de administrador.
2. Se instala en `%LOCALAPPDATA%\Programs\939 Host`.
3. La primera vez que hostees, el Firewall de Windows puede preguntar por 939 Host y por Java:
   permitilos en **redes privadas**.

## Verificar la descarga (opcional)

Cada release trae `SHA256SUMS.txt`. En PowerShell:

```powershell
Get-FileHash .\939Host-0.9.0-instalador.exe -Algorithm SHA256
```

El resultado tiene que coincidir con la línea de ese archivo en `SHA256SUMS.txt`.

Cada archivo tiene además una atestación de procedencia de GitHub que prueba que se compiló con los
workflows públicos de este repositorio, a partir de la etiqueta de la versión:

```bash
gh attestation verify 939Host-0.9.0-instalador.exe -R bouthex/939-host
```

## Firma digital y Smart App Control

| Versión | Estado |
|---|---|
| 0.9.0 | **Sin firmar**. La firma gratuita de SignPath Foundation está solicitada |

Qué pasa con una versión sin firmar:

- **SmartScreen** («Windows protegió su PC»): podés elegir *Más información → Ejecutar de todas formas*.
- **Smart App Control** (Windows 11): bloquea el programa y no ofrece una opción para continuar.
  No te recomendamos desactivarlo: según la documentación de Microsoft, apagarlo puede no tener vuelta
  atrás sin reinstalar Windows. Esperá la versión firmada.

Cuando SignPath apruebe el proyecto, los archivos de la release se van a reemplazar por versiones firmadas
con los mismos nombres. Para comprobar la firma: clic derecho en el `.exe` → *Propiedades* → *Firmas
digitales*. El firmante va a ser **SignPath Foundation**. Ver [CODE_SIGNING_POLICY.md](../CODE_SIGNING_POLICY.md).

## Actualizar

Ejecutá el instalador de la versión nueva encima. Cerrá antes 939 Host con **Salir** (así guarda y pasa
los servidores compartidos). Tus servidores y mundos se conservan.

## Desinstalar

*Configuración de Windows → Aplicaciones → 939 Host → Desinstalar*, o el acceso «Desinstalar 939 Host»
del menú Inicio. Tus servidores, mundos y copias quedan en `%LOCALAPPDATA%\939Host`; borralos a mano si
ya no los querés. Si usaste la reconexión automática, quitala antes desde Configuración para sacar el
mod de tu perfil de Minecraft.
