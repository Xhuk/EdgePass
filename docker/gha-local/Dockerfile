# gha-local — un runner self-hosted **genérico**, en contenedor
#
# Por qué existe: los minutos hospedados de GitHub Actions se acaban. Un runner
# self-hosted no consume minutos — corre en esta máquina —, así que el pipeline se da
# vuelta: el build pesado corre en Docker Desktop, la imagen va a GHCR, y el job de
# deploy (fino) lo ejecuta este contenedor. GitHub orquesta; el trabajo y los secretos
# se quedan acá.
#
# Quién lo maneja: el widget edgePass (LED `GW`, módulo `src-tauri/src/gha.rs`). Le acuña
# el registration token con `gh api`, lo arranca, y lo **recrea cuando la credencial
# vence** (el 404 de `runner-registration` que deja los runs en `queued` para siempre).
#
# La receta de arranque (env, volúmenes, socket) vive en dos lugares y solo dos:
#   - `gha::run_args` en el widget (el camino normal), y
#   - `scripts/gha-local.ps1` (el camino manual, para cuando no querés el widget).
# Este Dockerfile no sabe nada de eso a propósito: solo construye la caja.
#
# Build:  docker build -t gha-local:latest docker/gha-local
#
# Base 22.04 (jammy), no 24.04: este runner es *framework-dependent* de .NET 6 — su
# propio `config.sh` avisa "Dotnet Core 6.0 dependencies" y aborta si faltan — y Ubuntu
# 24.04 ya no trae .NET 6 en el feed de Microsoft. Jammy es además lo que usa la imagen
# oficial de runners de GitHub. Node 22, Go y el CLI de Docker funcionan igual acá.
FROM ubuntu:22.04

# El runner de GitHub y las versiones del toolchain. Se pueden pisar con --build-arg.
# 2.338.0 (oct-2026): versiones viejas ya NO pueden registrarse — GitHub responde 404
# "Your runner version is out of date". Si el registro empieza a fallar así, esto es lo
# primero que hay que subir (y el widget lo ve en el log, ver `credential_error_from_logs`).
ARG RUNNER_VERSION=2.338.0
ARG GO_VERSION=1.25.7
ARG NODE_MAJOR=22
ARG DOCKER_CLI_VERSION=27.5.1
ARG BUILDX_VERSION=v0.17.1

ENV DEBIAN_FRONTEND=noninteractive

# Base de los repos del founder: Go, Node, Python, Docker CLI, gh, make, gcc.
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git jq unzip zip xz-utils tar \
      build-essential pkg-config libssl-dev \
      python3 python3-pip python3-venv python3-dev \
      openssh-client rsync sudo procps iproute2 && \
    rm -rf /var/lib/apt/lists/*

# Node (los frontends del repo) — vía el repo oficial de NodeSource.
RUN curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash - && \
    apt-get install -y nodejs && rm -rf /var/lib/apt/lists/*

# Go
RUN curl -fsSL "https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz" -o /tmp/go.tgz && \
    tar -C /usr/local -xzf /tmp/go.tgz && rm /tmp/go.tgz
ENV PATH="/usr/local/go/bin:/usr/local/bin:${PATH}"

# Docker CLI + buildx: el *socket* lo monta el widget desde el host, así que el runner
# puede construir y empujar imágenes sin un dockerd propio.
RUN curl -fsSL "https://download.docker.com/linux/static/stable/x86_64/docker-${DOCKER_CLI_VERSION}.tgz" \
      -o /tmp/docker.tgz && \
    tar -C /tmp -xzf /tmp/docker.tgz && \
    cp /tmp/docker/docker /usr/local/bin/ && \
    rm -rf /tmp/docker.tgz /tmp/docker && \
    mkdir -p /usr/local/lib/docker/cli-plugins && \
    curl -fsSL "https://github.com/docker/buildx/releases/download/${BUILDX_VERSION}/buildx-${BUILDX_VERSION}.linux-amd64" \
      -o /usr/local/lib/docker/cli-plugins/docker-buildx && \
    chmod +x /usr/local/lib/docker/cli-plugins/docker-buildx

# GitHub CLI (para `gh api`, `gh workflow run`, `gh run watch` dentro del job)
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
      -o /usr/share/keyrings/githubcli-archive-keyring.gpg && \
    chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg && \
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
      > /etc/apt/sources.list.d/github-cli.list && \
    apt-get update && apt-get install -y gh && rm -rf /var/lib/apt/lists/*

# El runner en sí. Vive en /home/runner/actions-runner.
ARG RUNNER_SCOPE=repo
ENV RUNNER_HOME=/home/runner/actions-runner
RUN mkdir -p "${RUNNER_HOME}" && \
    curl -fsSL "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz" \
      -o /tmp/runner.tar.gz && \
    tar -C "${RUNNER_HOME}" -xzf /tmp/runner.tar.gz && \
    rm /tmp/runner.tar.gz && \
    "${RUNNER_HOME}/bin/installdependencies.sh" && \
    # Verificación, no confianza. El runner trae su .NET embebido (no hay comando
    # `dotnet`), así que lo que puede fallar son las dependencias NATIVAS — y `ldd` no
    # las ve todas: en Ubuntu 24.04 `libicu74` está presente pero .NET 6 no la acepta y
    # el fallo aparece en runtime ("Couldn't find a valid ICU package"). `--version`
    # ejercita el runtime de verdad; si esto pasa, la imagen no se construye.
    "${RUNNER_HOME}/bin/Runner.Listener" --version

# El contrato de env → configuración del runner, y `config.sh` + `run.sh` en marcha.
# Corre como root a propósito: el socket de Docker que se monta es del root del host, y
# un `docker` sin permisos convierte cada job en un "permission denied" confuso. El
# entrypoint pone RUNNER_ALLOW_RUNASROOT=1 por lo mismo (si no, `config.sh` aborta con
# "Must not run with sudo").
COPY entrypoint.sh /usr/local/bin/gha-entrypoint
RUN chmod +x /usr/local/bin/gha-entrypoint

WORKDIR /home/runner

# RUNNER_SCOPE  = repo (por defecto) | org
# REPO_URL      = https://github.com/<owner>/<repo>   (o la URL de la org)
# RUNNER_TOKEN  = registration token, caduca en ~1 h (lo acuña el widget con `gh`)
# RUNNER_NAME   = nombre visible en GitHub (default: el hostname)
# RUNNER_LABELS = self-hosted,Linux,X64,local
# RUNNER_WORKDIR= /tmp/runner-work   (montá un volumen acá)
ENTRYPOINT ["/usr/local/bin/gha-entrypoint"]
