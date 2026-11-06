#!/usr/bin/env bash
#
# Configura (si hace falta) y corre el runner. Es lo único que el contenedor hace:
# nada de timers, nada de reintentos infinitos — cuando la credencial vence, esto
# falla, el contenedor sale, y quien lo vigila (el LED `GW` de edgePass) lo recrea con
# un token nuevo. Fingir que sigue vivo es lo que deja los runs en `queued` para
# siempre.
set -euo pipefail

# Un comando explícito gana: `docker run gha-local bash` tiene que darte un shell, no
# arrancar el runner (útil para inspeccionar la caja sin una fila en la config).
if [ "$#" -gt 0 ]; then
  exec "$@"
fi

RUNNER_HOME="${RUNNER_HOME:-/home/runner/actions-runner}"
SCOPE="${RUNNER_SCOPE:-repo}"
WORKDIR_RUNNER="${RUNNER_WORKDIR:-/tmp/runner-work}"
NAME="${RUNNER_NAME:-$(hostname)}"
LABELS="${RUNNER_LABELS:-self-hosted,Linux,X64,local}"

say() { printf '\n\033[1m%s\033[0m\n' "$1"; }
fail() { printf '\033[31m%s\033[0m\n' "$1" >&2; exit 1; }

[ -n "${REPO_URL:-}" ] || fail "Falta REPO_URL (https://github.com/<owner>/<repo>)."
cd "$RUNNER_HOME"
mkdir -p "$WORKDIR_RUNNER"

if [ -f .runner ] && [ -f .credentials ]; then
  say "Ya estaba configurado como «$(sed -n 's/.*"agentName": *"\([^"]*\)".*/\1/p' .runner 2>/dev/null || echo "$NAME")»"
else
  [ -n "${RUNNER_TOKEN:-}" ] || fail "Falta RUNNER_TOKEN (registration token, caduca en ~1 h)."
  # Estado a medias: `.runner` sin `.credentials`, que es lo que deja un cierre brusco
  # (o el borrado del credential). Llamar a `config.sh` así responde "Cannot configure
  # the runner because it is already configured" y el contenedor entra en loop: se
  # limpia y se registra de nuevo, que es la única salida.
  if [ -f .runner ] || [ -f .credentials ]; then
    say "Configuración a medias: la borro y registro de nuevo"
    rm -f .runner .credentials .runner_migrated .service 2>/dev/null || true
  fi
  say "Configurando «$NAME» para $REPO_URL (scope $SCOPE, labels $LABELS)"
  # `config.sh` aborta con "Must not run with sudo" cuando ve uid 0 — y acá corremos
  # como root a propósito: el socket de Docker que llega por volumen es del root del
  # host, y un `docker` sin permisos convierte cada job en un "permission denied"
  # confuso. La variable es la salida documentada por el propio runner en contenedor.
  export RUNNER_ALLOW_RUNASROOT=1
  ./config.sh \
    --url "$REPO_URL" \
    --token "$RUNNER_TOKEN" \
    --name "$NAME" \
    --labels "$LABELS" \
    --work "$WORKDIR_RUNNER" \
    --unattended \
    --replace
fi

# Si el token quedó muerto a mitad de camino, GitHub lo dice acá y con código de salida
# distinto de cero: el vigía tiene que verlo para saber que hay que recrear.
say "Escuchando trabajos…"
exec ./run.sh
