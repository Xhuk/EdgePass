# Manifiesto de winget: qué pidió Microsoft

Registro de la primera sumisión, PR [microsoft/winget-pkgs#472420](https://github.com/microsoft/winget-pkgs/pull/472420).
Fecha: 6 de noviembre de 2026.

## Lo que respondió Microsoft

### 1. Falta el acuerdo de licencia (CLA)

Estado del chequeo: `license/cla` en cola. El bot dice:

> This PR cannot be merged until you sign the Contributor License Agreement (CLA).

El arreglo escribe un comentario desde la cuenta dueña del PR:

```
@microsoft-github-policy-service agree
```

Solo lo puede hacer la persona dueña de la cuenta. Es un acuerdo legal, no un trámite técnico.
Si la contribución se hace por una empresa, la forma es `agree company="Nombre"`.

### 2. El manifiesto singleton no sirve

Chequeo `02. Manifest Validation`: falló en 16 segundos.

> `Manifest type not supported. ManifestType: singleton`.

El catálogo comunitario exige un juego de tres archivos, cada uno con su `ManifestType`:

| Archivo | `ManifestType` |
|---|---|
| `Xhuk.EdgePass.yaml` | `version` |
| `Xhuk.EdgePass.locale.en-US.yaml` | `defaultLocale` |
| `Xhuk.EdgePass.installer.yaml` | `installer` |

Ese diagnóstico vino de un asistente marcado como experimental. Lo verifiqué contra
`docs/Authoring.md` del catálogo: el juego de tres archivos es el formato esperado.

## Una trampa que vale registrar

`winget validate` **acepta** un singleton y responde "Manifest validation succeeded".
El catálogo no lo acepta. Validar en local no prueba que el PR vaya a pasar.
La única prueba real es el chequeo del PR.

## Estado

| Paso | Estado |
|---|---|
| Tres manifiestos escritos | hecho, en `packaging/winget/` |
| `winget validate` sobre el juego completo | hecho: "Manifest validation succeeded" |
| Acuerdo de licencia firmado | **pendiente, lo hace la persona dueña de la cuenta** |
| PR con la versión 0.3.1 | pendiente |

El PR 472420 apunta a la versión 0.3.0 y queda obsoleto. Cuando el CLA esté firmado,
se envía la 0.3.1 con el juego de tres archivos.
