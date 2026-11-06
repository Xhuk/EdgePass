# Seguridad de edgePass

Estado: revisión hecha el 6 de noviembre de 2026, sobre la versión 0.3.0.
Este documento dice qué protege la app, qué se arregló en 0.3.1 y qué queda abierto.

## Modelo de confianza

edgePass no escucha en la red. No hay puerto, ni servidor, ni API remota.

Hay dos superficies:

1. **El pipe local** (`\\.\pipe\edgepass`). Cualquier proceso de tu mismo usuario puede abrirlo y mandar JSON.
2. **El vault**. La app habla solo con OpenBao en loopback (`127.0.0.1`, `::1`, `localhost`).

La frontera de seguridad es entonces **"cualquier proceso del mismo usuario"**, no un atacante remoto.
Un proceso que corre con tu cuenta ya puede leer el archivo de configuración y tus archivos.
El objetivo de la app es más chico y más útil: que el token del vault no salga de la máquina,
y que el pipe no sea un camino más corto que el propio sistema operativo.

## Lo que se arregló en 0.3.1

### Alto: salida del path declarado

`net::join_url` concatenaba el path relativo sin mirar qué contenía. El intérprete de URL
normaliza `..`, así que la ruta se escapaba del path que el cliente declaraba.

Prueba que mostró el fallo, antes del arreglo:

```
{"op":"run","entries":[{"namespace":"vetgroom","path":"surfaces/demo-runtime/../saas-runtime"}],"keys":["ADMIN_KEY"]}
→ {"code":0,"env":["ADMIN_KEY"],"ok":true}
```

El cliente pidió un path y leyó otro. `meter` con `../sys/health` respondía `ok:true`.

Ahora `join_url` valida cada segmento antes de armarlo. Rechaza `.`, `..`, `/`, `\`, `?`,
`#` y caracteres de control. La comprobación corre sobre la forma decodificada, así que
`%2e%2e` y `%2f` caen igual que `..` y `/`. Un nombre de clave normal pasa sin cambios.

El mismo camino se usa para escribir y para borrar, así que el arreglo cubre los tres.

### Medio: el token podía seguir un redirect

El cliente HTTP seguía redirects. `X-Vault-Token` es un header propio, y un header propio
no se borra al cambiar de host. Algo que respondiera en `127.0.0.1:8200` podía redirigir
el token a un sitio externo. Ahora el cliente usa `redirect(Policy::none())`.

## Lo que queda abierto

| Severidad | Hallazgo | Dónde |
|---|---|---|
| Medio | Sin CSP y sin carpeta `capabilities/` de Tauri | `tauri.conf.json` |
| Medio | `{"op":"seal"}` no pide confirmación a ningún humano | `ipc.rs` |
| Bajo | Los errores reenvían JSON del vault con `request_id` y `warnings` | `ipc.rs` |
| Bajo | `localhost` se acepta por nombre: un `hosts` envenenado redirige el token | `net.rs` |
| Info | `cmd /C` sobre `credential_command`; ningún op del pipe escribe configuración | `mcp.rs` |

El hallazgo de `seal` es de disponibilidad, no de confidencialidad. Un proceso del mismo
usuario puede sellar el vault y cortar a todos sus consumidores. No hay cuenta privilegiada
que lo impida.

## Lo que está bien y se midió

- `run` pasa el comando por argv, sin shell. El JSON no puede inyectar comandos.
- El pipe resiste JSON roto, una op desconocida y una query de 20 KB.
- `npm audit` da cero vulnerabilidades.
- El frontend no usa `dangerouslySetInnerHTML`.
- El loopback se aplica antes de construir el cliente y el proxy está desactivado.
- El token vive en el keychain del sistema, no en disco.
- El reply de `run` devuelve nombres de variables, nunca valores.

## Si encontrás un secreto expuesto

Avisá a la persona dueña del repo. El valor no se copia al chat, al issue ni al informe.
Se rota, y la rotación es la contención.
