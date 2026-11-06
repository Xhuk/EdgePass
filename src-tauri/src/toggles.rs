//! Header toggles: the vocabulary of what a LED may read (`probe`) and what a
//! click may do (`action`).
//!
//! A toggle is *data* (`config.toggles[]`), never UI code. The widget renders the
//! specs the backend hands it, and this module is the whole vocabulary: adding a
//! new kind of toggle is one entry in [`PROBES`]/[`ACTIONS`] plus its arm in
//! [`probe_state`]/[`run_action`]. The same tables are published to the outside —
//! `get_status`, `{"op":"toggles"}` — so a script, a skill or an agent discovers
//! the ids instead of guessing them.
//!
//! Two rules keep this honest:
//!
//! - **The caller names a LED, not an action.** `fire` reads the spec from the
//!   config and decides which action a click means, so nothing outside the widget
//!   can fire an action a toggle does not own.
//! - **Probes never leave the process.** They read what `seal_status`/`qring`
//!   already know; the vault itself is only an HTTP round trip because the seal
//!   state is not cached anywhere else.

use crate::config::AppConfig;
use crate::qring::{self, QringStatusDto};
use crate::unseal::SealStatusDto;
use serde::{Deserialize, Serialize};

/// Hard cap. The header is a single row: with six 3-6 letter LEDs `App.css`
/// shrinks them and the widget widens the window to fit the row whole, so the
/// layout and this number move together — never raise it without the layout.
pub const MAX_TOGGLES: usize = 6;

/// Longest label the header pill carries without pushing the row around. With
/// five LEDs of four characters the whole row still fits the 410 px floating bar
/// next to the drag handle and the `edgePass` label — that is the real budget.
pub const MAX_LABEL_CHARS: usize = 4;

/// One header LED, as stored in `config.json` and as the UI sends it back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleSpec {
    /// Stable key: what a click sends back, what the pipe uses to address one
    /// LED. Derived from the label when empty and made unique by [`validate`].
    pub id: String,
    /// What the LED says (`KV`, `GW`, `GOV`…).
    pub label: String,
    /// Probe id — how "is it on?" is answered (see [`PROBES`]).
    pub probe: String,
    /// Action run when the probe says off (see [`ACTIONS`]).
    pub on: String,
    /// Action run when the probe says on.
    pub off: String,
    /// Optional third action for `Ctrl`/right click (`GW` restarts with it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
}

/// A probe: how a LED reads its state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeInfo {
    pub id: &'static str,
    /// Shown in the settings picker and in the wire catalog. Spanish, like the
    /// rest of the widget's own copy.
    pub doc: &'static str,
    /// Resolving this probe shells out (`tasklist`) and `get_status` runs on a
    /// 5 s poll — so it is only paid for while a toggle actually asks for it.
    pub needs_qring: bool,
    /// Same rule for the local GHA runner probe: it shells out to `docker`
    /// (`info` + `inspect`, and `logs` only when something is wrong).
    pub needs_gha: bool,
    /// What a new toggle on this probe should do. The picker fills these in when
    /// the probe changes; either side can then be overridden.
    pub on: &'static str,
    pub off: &'static str,
    pub alt: Option<&'static str>,
}

/// An action: what a click does.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionInfo {
    pub id: &'static str,
    pub doc: &'static str,
    /// Needs the vault open (and, for `vault.seal`, a token): the settings editor
    /// says so before anyone clicks.
    pub needs_vault: bool,
}

/// The published vocabulary. `get_status` and `{"op":"toggles"}` both carry it,
/// so a client never hardcodes an id — and a new probe/action is a new row here
/// plus its arm in [`probe_state`]/[`run_action`] (the tests keep the two in
/// step).
pub const PROBES: &[ProbeInfo] = &[
    ProbeInfo {
        id: "vault.seal",
        doc: "OpenBao: abierto (unsealed) o sellado. Ámbar = no responde o sin inicializar.",
        needs_qring: false,
        needs_gha: false,
        on: "vault.unseal",
        off: "vault.seal",
        alt: None,
    },
    ProbeInfo {
        id: "qring.process",
        doc: "Gateway Q-ring: proceso qringd vivo o parado. Ámbar = falta el pack.",
        needs_qring: true,
        needs_gha: false,
        on: "qring.on",
        off: "qring.off",
        alt: Some("qring.restart"),
    },
    ProbeInfo {
        id: "qring.governance",
        doc: "Política Q-ring: si los rings pueden correr (y gastar). Este LED es el que pulsa cuando el gateway tiene un Allow esperando.",
        needs_qring: true,
        needs_gha: false,
        on: "qring.govern.on",
        off: "qring.govern.off",
        alt: None,
    },
    ProbeInfo {
        id: "docker.desktop",
        doc: "Docker Desktop: el motor responde. Ámbar = instalado pero parado (el click lo arranca).",
        needs_qring: false,
        needs_gha: true,
        on: "docker.up",
        off: "gha.down",
        alt: None,
    },
    ProbeInfo {
        id: "gha.runner",
        doc: "Runner local de GitHub Actions: Docker Desktop arriba Y el contenedor del runner sano (si no, el job se queda en cola para siempre). Ámbar = falta el runner, está parado o quedó en loop de credencial.",
        needs_qring: false,
        needs_gha: true,
        on: "gha.up",
        off: "gha.down",
        alt: Some("daemoncurier.open"),
    },
    ProbeInfo {
        id: "skills.sync",
        doc: "Biblioteca de skills: los enlaces del hub (~/.agents/skills) están al día en los 50 agentes. Ámbar = deriva, o falta el sincronizador.",
        needs_qring: false,
        needs_gha: false,
        on: "skills.sync",
        off: "skills.sync",
        alt: None,
    },
    ProbeInfo {
        id: "skills.watch",
        doc: "Auto-sincronización: vigilante del hub, re-enlaza solo cuando aparece, se renombra o se borra una skill.",
        needs_qring: false,
        needs_gha: false,
        on: "skills.watch.on",
        off: "skills.watch.off",
        alt: None,
    },
    ProbeInfo {
        id: "mcp.servers",
        doc: "Servidores MCP de la lista: encendido cuando están todos vivos Y el túnel declarado de cada uno está conectado. Ámbar = falta el binario de alguno o falta cloudflared. El nombre del servidor, su túnel y su URL salen del panel MCP.",
        needs_qring: false,
        needs_gha: false,
        on: "mcp.on",
        off: "mcp.off",
        alt: Some("mcp.restart"),
    },
];

pub const ACTIONS: &[ActionInfo] = &[
    ActionInfo {
        id: "vault.unseal",
        doc: "Abrir el vault (unseal con las keys del init).",
        needs_vault: false,
    },
    ActionInfo {
        id: "vault.seal",
        doc: "Sellar el vault: corta a TODOS sus clientes hasta que alguien lo abra.",
        needs_vault: true,
    },
    ActionInfo {
        id: "qring.on",
        doc: "Arrancar el gateway Q-ring. No enciende la política: eso es GOV.",
        needs_vault: true,
    },
    ActionInfo {
        id: "qring.off",
        doc: "Detener el gateway Q-ring. Apaga también la política: un gateway parado que dice «los rings pueden correr» sería una promesa que nadie puede cumplir.",
        needs_vault: false,
    },
    ActionInfo {
        id: "qring.restart",
        doc: "Reiniciar el gateway Q-ring.",
        needs_vault: true,
    },
    ActionInfo {
        id: "qring.govern.on",
        doc: "Permitir que los rings corran. Si el gateway está parado, lo arranca primero (con el vault, como qring.on).",
        needs_vault: true,
    },
    ActionInfo {
        id: "qring.govern.off",
        doc: "Bloquear los rings: no corren, no gastan. Vale con el gateway arriba: es el caso que GOV existe para cubrir.",
        needs_vault: false,
    },
    ActionInfo {
        id: "skills.sync",
        doc: "Re-enlazar los agentes desde el hub de skills (no copia archivos). Ambos lados del LED hacen lo mismo: sincronizar.",
        needs_vault: false,
    },
    ActionInfo {
        id: "skills.watch.on",
        doc: "Encender la auto-sincronización: un vigilante del hub que re-enlaza al detectar un cambio.",
        needs_vault: false,
    },
    ActionInfo {
        id: "skills.watch.off",
        doc: "Apagar la auto-sincronización (el botón manual sigue funcionando).",
        needs_vault: false,
    },
    ActionInfo {
        id: "mcp.on",
        doc: "Arrancar todos los servidores MCP de la lista y el túnel nombrado que declare cada fila. Idempotente: lo que ya está vivo no se duplica.",
        needs_vault: true,
    },
    ActionInfo {
        id: "mcp.off",
        doc: "Detener los servidores MCP y sus conectores de túnel. Solo mata por pid los que arrancó edgePass; el túnel, por pid con su nombre en la línea de comandos (nunca por imagen).",
        needs_vault: false,
    },
    ActionInfo {
        id: "mcp.restart",
        doc: "Reiniciar todos los servidores MCP y sus túneles (el gesto Ctrl / click derecho del LED).",
        needs_vault: true,
    },
    ActionInfo {
        id: "docker.up",
        doc: "Arrancar Docker Desktop y esperar al motor (hasta 120 s). Idempotente: si ya responde, no hace nada.",
        needs_vault: false,
    },
    ActionInfo {
        id: "gha.up",
        doc: "Asegurar el runner local: si Docker no responde lo arranca, si falta la imagen la construye, y si el contenedor no existe, está parado o quedó en loop de credencial, lo recrea con un registration token nuevo (gh api).",
        needs_vault: false,
    },
    ActionInfo {
        id: "gha.down",
        doc: "Parar el contenedor del runner local. Docker Desktop queda quieto: apagarlo cortaría todos los demás stacks de la máquina.",
        needs_vault: false,
    },
    ActionInfo {
        id: "gha.repair",
        doc: "Recrear el runner aunque parezca sano (si GitHub lo ve offline): borra el contenedor, acuña un token nuevo y lo vuelve a crear.",
        needs_vault: false,
    },
    ActionInfo {
        id: "daemoncurier.open",
        doc: "Abrir DaemonCurier (el HUD de WSL2/sistema): si ya corre, le pide foco por su pipe; si no, lo lanza.",
        needs_vault: false,
    },
];

/// Everything the outside is allowed to know about the vocabulary in one object.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub max_toggles: usize,
    pub max_label_chars: usize,
    pub probes: &'static [ProbeInfo],
    pub actions: &'static [ActionInfo],
}

pub fn catalog() -> Catalog {
    Catalog {
        max_toggles: MAX_TOGGLES,
        max_label_chars: MAX_LABEL_CHARS,
        probes: PROBES,
        actions: ACTIONS,
    }
}

/// The LED as it is right now: the four bits the header has always shown, plus
/// the tooltip text.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleState {
    pub on: bool,
    /// Reachable but unhealthy (vault unreachable, pack missing): amber.
    pub warn: bool,
    /// The gateway has an Allow waiting: the LED asks.
    pub ask: bool,
    /// Whether a click can do anything at all; `false` disables the button.
    pub live: bool,
    /// What the probe saw, for the tooltip.
    pub detail: String,
}

/// The docs a tooltip needs, resolved from the catalog so the UI never maps ids
/// to text itself.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleDocs {
    pub probe: String,
    pub on: String,
    pub off: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
}

/// One LED: the stored spec, what its probe sees, and the text for its tooltip.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleView {
    #[serde(flatten)]
    pub spec: ToggleSpec,
    pub state: ToggleState,
    pub docs: ToggleDocs,
}

/// What every probe is allowed to look at, resolved once per snapshot.
pub struct ToggleCtx<'a> {
    pub cfg: &'a AppConfig,
    pub seal: &'a SealStatusDto,
    /// `Some` only when some toggle needs it — see [`any_needs_qring`].
    pub qring: Option<QringStatusDto>,
    /// Same rule for the local runners — see [`any_needs_gha`].
    pub gha: Option<crate::gha::GhaStatusDto>,
    /// A pending Allow on the gateway channel.
    pub grant_pending: bool,
}

impl<'a> ToggleCtx<'a> {
    /// Probes the Q-ring status only when a toggle asks for it: the call spawns
    /// `tasklist`, and an install that removed the Q-ring LEDs should not pay for
    /// it every 5 s. The Docker/GHA probe follows the same rule (`docker info` +
    /// `docker inspect` per declared runner).
    pub fn new(cfg: &'a AppConfig, seal: &'a SealStatusDto, grant_pending: bool) -> Self {
        let qring = if any_needs_qring(&cfg.toggles) {
            Some(qring::status(&cfg.qring_pack, &cfg.qring_host))
        } else {
            None
        };
        let gha = if any_needs_gha(&cfg.toggles) {
            Some(crate::gha::status(cfg))
        } else {
            None
        };
        Self {
            cfg,
            seal,
            qring,
            gha,
            grant_pending,
        }
    }
}

pub fn any_needs_qring(specs: &[ToggleSpec]) -> bool {
    specs.iter().any(|s| probe_needs_qring(&s.probe))
}

pub fn any_needs_gha(specs: &[ToggleSpec]) -> bool {
    specs.iter().any(|s| probe_needs_gha(&s.probe))
}

pub fn probe_needs_gha(id: &str) -> bool {
    probe_info(id).map(|p| p.needs_gha).unwrap_or(false)
}

pub fn probe_needs_qring(id: &str) -> bool {
    probe_info(id).map(|p| p.needs_qring).unwrap_or(false)
}

pub fn probe_info(id: &str) -> Option<&'static ProbeInfo> {
    PROBES.iter().find(|p| p.id == id)
}

pub fn action_info(id: &str) -> Option<&'static ActionInfo> {
    ACTIONS.iter().find(|a| a.id == id)
}

pub fn is_known_probe(id: &str) -> bool {
    probe_info(id).is_some()
}

pub fn is_known_action(id: &str) -> bool {
    action_info(id).is_some()
}

fn probe_ids() -> String {
    PROBES
        .iter()
        .map(|p| p.id)
        .collect::<Vec<_>>()
        .join(", ")
}

fn action_ids() -> String {
    ACTIONS
        .iter()
        .map(|a| a.id)
        .collect::<Vec<_>>()
        .join(", ")
}

/// A fresh install: just the vault LED. The Q-ring pair *and* the local GHA runner
/// are in the catalog (Ajustes → Toggles), but an install with no Q-ring pack and no
/// Docker Desktop should not open with a row of amber lights.
pub fn default_toggles() -> Vec<ToggleSpec> {
    vec![kv()]
}

/// `GW` as it shipped before `toggles[]` existed: the Q-ring gateway's process.
///
/// Kept for two reasons: [`legacy_toggles`] has to be faithful to what those headers
/// hardcoded, and the v3→v4 migration matches **exactly** this spec before rewriting it —
/// so a row someone edited by hand (another label, another probe) is left alone. That is
/// a choice, not a default.
pub fn stock_gw() -> ToggleSpec {
    ToggleSpec {
        id: "gw".into(),
        label: "GW".into(),
        probe: "qring.process".into(),
        on: "qring.on".into(),
        off: "qring.off".into(),
        alt: Some("qring.restart".into()),
    }
}

/// What a `config.json` written *before* `toggles[]` existed showed: these three
/// were hardcoded in the header, and an upgrade must not change that.
pub fn legacy_toggles() -> Vec<ToggleSpec> {
    vec![kv(), stock_gw(), gov()]
}

fn kv() -> ToggleSpec {
    ToggleSpec {
        id: "kv".into(),
        label: "KV".into(),
        probe: "vault.seal".into(),
        on: "vault.unseal".into(),
        off: "vault.seal".into(),
        alt: None,
    }
}

pub fn gw() -> ToggleSpec {
    ToggleSpec {
        id: "gw".into(),
        label: "GW".into(),
        probe: "gha.runner".into(),
        on: "gha.up".into(),
        off: "gha.down".into(),
        alt: Some("daemoncurier.open".into()),
    }
}

/// The Q-ring gateway's *process* — what `GW` was before v4, now its own LED so the
/// text "gateway" stops meaning two different things on the same header.
pub fn qr() -> ToggleSpec {
    ToggleSpec {
        id: "qr".into(),
        label: "QR".into(),
        probe: "qring.process".into(),
        on: "qring.on".into(),
        off: "qring.off".into(),
        alt: Some("qring.restart".into()),
    }
}

fn gov() -> ToggleSpec {
    ToggleSpec {
        id: "gov".into(),
        label: "GOV".into(),
        probe: "qring.governance".into(),
        on: "qring.govern.on".into(),
        off: "qring.govern.off".into(),
        alt: None,
    }
}

/// The header row: every stored spec with what its probe sees right now.
pub fn views(cfg: &AppConfig, seal: &SealStatusDto, grant_pending: bool) -> Vec<ToggleView> {
    let ctx = ToggleCtx::new(cfg, seal, grant_pending);
    cfg.toggles.iter().map(|spec| view(spec, &ctx)).collect()
}
fn view(spec: &ToggleSpec, ctx: &ToggleCtx) -> ToggleView {
    // A spec that survived validation always resolves; when one does not (a
    // hand-edited `config.json`) the LED says so instead of lying about the state.
    let state = probe_state(&spec.probe, ctx).unwrap_or_else(|| ToggleState {
        on: false,
        warn: true,
        ask: false,
        live: false,
        detail: format!("probe desconocida: {}", spec.probe),
    });
    let doc = |id: &str| {
        action_info(id)
            .map(|a| a.doc.to_string())
            .unwrap_or_else(|| format!("acción desconocida: {id}"))
    };
    ToggleView {
        docs: ToggleDocs {
            probe: probe_info(&spec.probe)
                .map(|p| p.doc.to_string())
                .unwrap_or_else(|| format!("probe desconocida: {}", spec.probe)),
            on: doc(&spec.on),
            off: doc(&spec.off),
            alt: spec.alt.as_deref().map(doc),
        },
        state,
        spec: spec.clone(),
    }
}

/// Reads one LED. `None` = no such probe (the tests keep every table entry
/// resolving).
pub fn probe_state(id: &str, ctx: &ToggleCtx) -> Option<ToggleState> {
    match id {
        "vault.seal" => {
            let s = ctx.seal;
            Some(ToggleState {
                on: s.reachable && !s.sealed,
                warn: !s.reachable || !s.initialized,
                ask: false,
                // A vault that does not answer cannot be sealed/unsealed either.
                live: s.reachable,
                detail: s.message.clone(),
            })
        }
        "qring.process" => {
            let (on, warn, live, detail) = match ctx.qring.as_ref() {
                Some(q) => (q.running, !q.pack_ok, q.pack_ok, q.message.clone()),
                None => (
                    false,
                    true,
                    false,
                    "Q-ring sin sonda: no pude leer su estado".to_string(),
                ),
            };
            Some(ToggleState {
                on,
                warn,
                // The Allow is a *policy* question ("may this run?"), so the pulse
                // lives on the policy LED. `QR` is process control only.
                ask: false,
                live,
                detail,
            })
        }
        "docker.desktop" => {
            let (on, detail) = match ctx.gha.as_ref() {
                Some(g) if g.docker_up => (true, "Docker Desktop responde".to_string()),
                Some(g) => (false, g.message.clone()),
                None => (
                    false,
                    "Docker sin sonda: no pude leer su estado".to_string(),
                ),
            };
            Some(ToggleState {
                on,
                // Amber, not off: a Docker that is installed and simply stopped is a
                // click away from being fine, and that is what the colour says.
                warn: !on,
                ask: false,
                // Always clickable: the click is what starts it.
                live: true,
                detail,
            })
        }
        "gha.runner" => {
            let (on, detail) = match ctx.gha.as_ref() {
                Some(g) => (crate::gha::all_healthy(g), g.message.clone()),
                None => (
                    false,
                    "runner sin sonda: no pude leer su estado".to_string(),
                ),
            };
            Some(ToggleState {
                on,
                warn: !on,
                ask: false,
                live: true,
                detail,
            })
        }
        "qring.governance" => {
            let on = ctx.cfg.qring_governed;
            Some(ToggleState {
                on,
                warn: false,
                ask: ctx.grant_pending,
                // The policy lives in edgePass, but the LED belongs to the
                // gateway: with no Q-ring there is nothing to govern.
                live: ctx.qring.is_some(),
                detail: if on {
                    "los rings pueden correr".to_string()
                } else {
                    "los rings están bloqueados".to_string()
                },
            })
        }
        "skills.sync" => {
            let status = crate::skills::status();
            let sin_script = status
                .error
                .as_deref()
                .map(|e| e.contains("no encontré"))
                .unwrap_or(false);
            Some(ToggleState {
                on: status.insync,
                warn: status.error.is_some() || status.drift > 0,
                ask: false,
                // Sin script no hay nada que clickear; con él, el botón siempre sirve
                // (aunque el status todavía no se haya leído).
                live: !sin_script,
                detail: match &status.error {
                    Some(msg) => msg.clone(),
                    None if status.drift > 0 => format!(
                        "{} enlace(s) por arreglar: {} faltan, {} apuntan mal, {} muertos",
                        status.drift, status.faltan, status.mal_apuntados, status.muertos
                    ),
                    None if status.skills == 0 => "leyendo el hub…".to_string(),
                    None => format!(
                        "{} skills en {} tipos · {} agentes{}{}",
                        status.skills,
                        status.tipos,
                        status.agentes,
                        if status.anidadas > 0 {
                            format!(" · {} anidadas", status.anidadas)
                        } else {
                            String::new()
                        },
                        if status.omitidos > 0 {
                            format!(" · {} omitidas (carpeta real)", status.omitidos)
                        } else {
                            String::new()
                        }
                    ),
                },
            })
        }
        "skills.watch" => {
            let status = crate::skills::status();
            Some(ToggleState {
                on: status.watching,
                warn: false,
                ask: false,
                live: status.error.is_none(),
                detail: if status.watching {
                    "vigilando el hub: re-enlaza solo cada 5 s".to_string()
                } else {
                    "apagado: una skill nueva del hub no aparece sola en los agentes".to_string()
                },
            })
        }
        "mcp.servers" => {
            let status = crate::mcp::status(&ctx.cfg);
            let sin_config = status.total == 0;
            Some(ToggleState {
                // "on" = every configured server is alive.
                on: status.all_running(),
                // Amber is a broken row, not a stopped one: a missing binary (the
                // server's or `cloudflared`) needs a fix, and a stopped process
                // just needs a click.
                warn: status.broken,
                ask: false,
                live: !sin_config,
                detail: if sin_config {
                    "sin servidores MCP: cargá la lista en el panel MCP".to_string()
                } else {
                    status.detail.clone()
                },
            })
        }
        _ => None,
    }
}

/// Which action one click means. `flip` (the default) picks the side the probe
/// is *not* on; `on`/`off`/`alt` force one.
pub fn pick_action(spec: &ToggleSpec, dir: &str, current_on: bool) -> Result<String, String> {
    match dir {
        "" | "flip" => Ok(if current_on {
            spec.off.clone()
        } else {
            spec.on.clone()
        }),
        "on" => Ok(spec.on.clone()),
        "off" => Ok(spec.off.clone()),
        "alt" => spec
            .alt
            .clone()
            .ok_or_else(|| format!("el toggle '{}' no tiene acción alt", spec.id)),
        other => Err(format!(
            "dirección desconocida: '{other}' (on | off | alt | flip)"
        )),
    }
}

/// What an action did, in the shape both the UI (toast) and the pipe (reply) want.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionOutcome {
    /// The action id that actually ran (`flip` resolves to one of the spec's).
    pub action: String,
    /// What happened, for a tooltip or a log.
    pub detail: String,
    /// One line for the toast.
    pub toast: String,
}

/// What an action may touch. `state` is here for the same reason the dedicated
/// commands have it: unsealing counts as vault activity.
pub struct ActionCtx<'a> {
    pub cfg: &'a AppConfig,
    pub state: &'a crate::AppState,
}

/// Fires one LED by id — the click path, and the pipe's `{"op":"toggle"}`.
///
/// The spec comes from the *config*, never from the caller: the UI and the pipe
/// say which LED and which direction, not which action.
pub async fn fire(
    cfg: &AppConfig,
    id: &str,
    dir: &str,
    state: &crate::AppState,
) -> Result<ActionOutcome, String> {
    let spec = cfg.toggles.iter().find(|t| t.id == id).ok_or_else(|| {
        format!(
            "toggle desconocido: '{id}' · hay: {}",
            cfg.toggles
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    // Only a `flip` needs to know the current side: `on`/`off`/`alt` name it.
    let current_on = if matches!(dir, "" | "flip") {
        let seal = crate::unseal::seal_status(cfg).await;
        let ctx = ToggleCtx::new(cfg, &seal, !state.grants.list_public().is_empty());
        probe_state(&spec.probe, &ctx).map(|s| s.on).unwrap_or(false)
    } else {
        false
    };

    let action = pick_action(spec, dir, current_on)?;
    run_action(&action, &ActionCtx { cfg, state }).await
}

/// Runs one action. Delegates to the same functions the dedicated buttons and the
/// pipe's `unseal`/`seal` ops use, so a header click and a script cannot drift.
pub async fn run_action(id: &str, ctx: &ActionCtx<'_>) -> Result<ActionOutcome, String> {
    match id {
        "vault.unseal" => {
            let seal = crate::unseal_now(ctx.cfg, ctx.state).await?;
            // The old KV button ran a full `bootstrap` after opening the vault,
            // and that follow-up is what made search stop being empty. Keep it:
            // the index is only useful against an open vault.
            let synced = if !seal.sealed && ctx.cfg.auto_sync {
                crate::sync_index_inner(ctx.state).await.ok()
            } else {
                None
            };
            let detail = match synced {
                Some(n) => format!("{} · {n} claves", seal.message),
                None => seal.message.clone(),
            };
            Ok(ActionOutcome {
                action: id.into(),
                detail,
                toast: if seal.sealed {
                    seal.message
                } else {
                    "Vault abierto".into()
                },
            })
        }
        "vault.seal" => {
            let seal = crate::seal_vault_now(ctx.cfg).await?;
            Ok(ActionOutcome {
                action: id.into(),
                detail: seal.message.clone(),
                toast: if seal.sealed {
                    "Vault cerrado".into()
                } else {
                    seal.message
                },
            })
        }
        "qring.on" => qring_action(ctx, "on").await,
        "qring.off" => qring_action(ctx, "off").await,
        "qring.restart" => qring_action(ctx, "restart").await,
        "qring.govern.on" => govern_action(ctx, true).await,
        "qring.govern.off" => govern(ctx.cfg, false, None),
        "docker.up" => gha_action(ctx, GhaAct::Docker).await,
        "gha.up" => gha_action(ctx, GhaAct::Up).await,
        "gha.down" => gha_action(ctx, GhaAct::Down).await,
        "gha.repair" => gha_action(ctx, GhaAct::Repair).await,
        "daemoncurier.open" => {
            // Launching a process and talking to a pipe are blocking calls.
            let detail = tokio::task::spawn_blocking({
                let cfg = ctx.cfg.clone();
                move || crate::apps::open_daemoncurier(&cfg)
            })
            .await
            .map_err(|e| format!("daemoncurier task: {e}"))??;
            Ok(ActionOutcome {
                action: id.into(),
                toast: detail.clone(),
                detail,
            })
        }
        "skills.sync" => {
            // Un sync completo tarda ~1 s (y hasta 5 s si hay que crear 21 mil enlaces):
            // fuera del runtime async, igual que el CLI de Q-ring.
            let (report, _status) = tokio::task::spawn_blocking(crate::skills::sync_now)
                .await
                .map_err(|e| format!("skills-sync task: {e}"))??;
            let resumen = format!(
                "+{} enlaces, ~{} arreglados, -{} retirados{}",
                report.creados,
                report.arreglados,
                report.retirados,
                if report.omitidos > 0 {
                    format!(" · {} omitidos", report.omitidos)
                } else {
                    String::new()
                }
            );
            Ok(ActionOutcome {
                action: id.into(),
                detail: resumen.clone(),
                toast: if report.creados + report.arreglados + report.retirados == 0 {
                    "Skills ya estaban al día".into()
                } else {
                    format!("Skills sincronizadas · {resumen}")
                },
            })
        }
        "skills.watch.on" => {
            let detail = tokio::task::spawn_blocking(|| crate::skills::watch_set(true))
                .await
                .map_err(|e| format!("skills-sync task: {e}"))??;
            Ok(ActionOutcome {
                action: id.into(),
                toast: "Auto-sync encendido".into(),
                detail,
            })
        }
        "skills.watch.off" => {
            let detail = tokio::task::spawn_blocking(|| crate::skills::watch_set(false))
                .await
                .map_err(|e| format!("skills-sync task: {e}"))??;
            Ok(ActionOutcome {
                action: id.into(),
                toast: "Auto-sync apagado".into(),
                detail,
            })
        }
        "mcp.on" => mcp_action(ctx, "on").await,
        "mcp.off" => mcp_action(ctx, "off").await,
        "mcp.restart" => mcp_action(ctx, "restart").await,
        other => Err(format!(
            "acción desconocida: '{other}' · disponibles: {}",
            action_ids()
        )),
    }
}

/// Start/stop/restart the whole MCP list.
///
/// The vault is opened **only** when some row declares `secretEnv`: a list of
/// plain commands must not unseal anything just because a LED was clicked. The
/// resolved values go straight into the child's environment and never leave the
/// process — the same rule the `run` op follows.
async fn mcp_action(ctx: &ActionCtx<'_>, action: &'static str) -> Result<ActionOutcome, String> {
    let mut secrets = crate::mcp::Secrets::new();
    let mut cfg = ctx.cfg.clone();
    if action != "off" && crate::mcp::any_needs_vault(&cfg) {
        let (fresh, token) = crate::ensure_ready(ctx.state).await?;
        cfg = fresh;
        secrets = crate::mcp::resolve_all(&cfg, &token).await;
    }
    let verb = match action {
        "on" => "arrancado",
        "off" => "detenido",
        _ => "reiniciado",
    };
    // Spawning and `tasklist` are blocking; keep them off the async runtime,
    // like the Q-ring CLI and the skills sync.
    let res = tokio::task::spawn_blocking(move || match action {
        "on" => crate::mcp::start_all(&cfg, &secrets),
        "off" => crate::mcp::stop_all(&cfg),
        _ => crate::mcp::restart_all(&cfg, &secrets),
    })
    .await
    .map_err(|e| format!("mcp task: {e}"))??;
    Ok(ActionOutcome {
        action: format!("mcp.{action}"),
        detail: res.clone(),
        toast: format!("MCP {verb} · {res}"),
    })
}

/// `qring.py enable|disable` already dials this widget for vault hold/unseal, so
/// starting or restarting needs the vault open first — same as the old GW button.
async fn qring_action(ctx: &ActionCtx<'_>, action: &'static str) -> Result<ActionOutcome, String> {
    if action != "off" {
        let _ = crate::unseal::ensure_unsealed(ctx.cfg).await;
    }
    let pack = ctx.cfg.qring_pack.clone();
    let host = ctx.cfg.qring_host.clone();
    // Blocking CLI work (up to 90 s) off the async runtime.
    let res = tokio::task::spawn_blocking(move || qring::control(&pack, &host, action))
        .await
        .map_err(|e| format!("qring task: {e}"))??;
    let verb = match action {
        "on" => "on",
        "off" => "off",
        _ => "reiniciado",
    };
    Ok(ActionOutcome {
        action: format!("qring.{action}"),
        detail: res.message.clone(),
        toast: format!("Q-ring {verb} · {}", res.message),
    })
}

/// Turning the policy ON is a claim about the gateway: with `qringd` down the LED
/// would say "los rings pueden correr" while nothing can run, so the click starts it
/// first — the same path `qring.on` takes, vault included — and only then writes the
/// policy. A start that fails fails the whole action: enabling a policy on top of no
/// gateway is exactly the lie this coupling exists to remove.
///
/// `off` is deliberately *not* symmetric here: policy OFF over a running gateway is
/// the case `GOV` exists for (see `docs/TOGGLE-CONTRACT.md`).
async fn govern_action(ctx: &ActionCtx<'_>, enabled: bool) -> Result<ActionOutcome, String> {
    if !enabled {
        return govern(ctx.cfg, false, None);
    }
    let pack = ctx.cfg.qring_pack.clone();
    let host = ctx.cfg.qring_host.clone();
    // `tasklist` is blocking, off the async runtime like every other Q-ring probe.
    let status = tokio::task::spawn_blocking(move || qring::status(&pack, &host))
        .await
        .map_err(|e| format!("qring task: {e}"))?;
    let started = if status.running {
        None
    } else {
        Some(qring_action(ctx, "on").await?.detail)
    };
    govern(ctx.cfg, true, started.as_deref())
}

/// The policy write itself; `started` is what the gateway had to do on the way in.
fn govern(cfg: &AppConfig, enabled: bool, started: Option<&str>) -> Result<ActionOutcome, String> {
    let mut next = cfg.clone();
    next.qring_governed = enabled;
    crate::config::save_config(&next)?;
    let policy = if enabled {
        "los rings pueden correr"
    } else {
        "los rings están bloqueados"
    };
    Ok(ActionOutcome {
        action: if enabled {
            "qring.govern.on".into()
        } else {
            "qring.govern.off".into()
        },
        detail: match started {
            Some(what) => format!("{what} · {policy}"),
            None => policy.to_string(),
        },
        toast: if !enabled {
            "Q-ring gobernado OFF · nada corre, nada se gasta".into()
        } else if started.is_some() {
            format!("Q-ring arrancado y gobernado ON · {policy}")
        } else {
            format!("Q-ring gobernado ON · {policy}")
        },
    })
}

/// Which of the `GW` actions a click resolved to.
#[derive(Debug, Clone, Copy, PartialEq)]
enum GhaAct {
    /// Only wake Docker Desktop up.
    Docker,
    /// Ensure the runner: image, container, credential.
    Up,
    /// Stop the runner container (`docker stop`); Docker Desktop is left alone.
    Down,
    /// Recreate the container even when it looks healthy.
    Repair,
}

/// The `GW` LED's actions.
///
/// All of it is blocking — a cold `docker build` takes minutes and starting Docker
/// Desktop can take a while — so it runs off the async runtime, like the Q-ring CLI, the
/// MCP sweep and the skills sync. The vault is opened **only** when some runner declares
/// credentials from it, the same rule the MCP list follows.
async fn gha_action(ctx: &ActionCtx<'_>, act: GhaAct) -> Result<ActionOutcome, String> {
    let id = match act {
        GhaAct::Docker => "docker.up",
        GhaAct::Up => "gha.up",
        GhaAct::Down => "gha.down",
        GhaAct::Repair => "gha.repair",
    };
    let mut cfg = ctx.cfg.clone();
    let wants_secrets =
        act != GhaAct::Down && cfg.gha_runners.iter().any(|r| !r.secret_env.is_empty());
    let secrets = if wants_secrets {
        let (fresh, token) = crate::ensure_ready(ctx.state).await?;
        cfg = fresh;
        crate::gha::resolve_all(&cfg, &token).await
    } else {
        crate::gha::SecretMap::new()
    };

    let detail = tokio::task::spawn_blocking(move || match act {
        GhaAct::Docker => crate::gha::ensure_docker(&cfg)
            .map(|said| said.unwrap_or_else(|| "Docker Desktop ya estaba arriba".into())),
        GhaAct::Up => crate::gha::ensure(&cfg, None, &secrets),
        GhaAct::Down => crate::gha::stop(&cfg, None),
        GhaAct::Repair => crate::gha::repair(&cfg, &secrets),
    })
    .await
    .map_err(|e| format!("gha task: {e}"))??;

    Ok(ActionOutcome {
        action: id.into(),
        toast: format!("Local GHA · {detail}"),
        detail,
    })
}

/// Normalizes a list before it is stored: trims, uppercases the label, derives a
/// missing id from the label and keeps ids unique. Errors only when an entry
/// cannot work at all — and always names what *is* available, because the caller
/// is usually an agent that guessed (`{"op":"toggles","set":[…]}`).
pub fn validate(specs: Vec<ToggleSpec>) -> Result<Vec<ToggleSpec>, String> {
    if specs.len() > MAX_TOGGLES {
        return Err(format!(
            "máximo {MAX_TOGGLES} toggles (llegaron {})",
            specs.len()
        ));
    }
    let mut used: Vec<String> = Vec::with_capacity(specs.len());
    let mut out: Vec<ToggleSpec> = Vec::with_capacity(specs.len());
    for mut spec in specs {
        spec.label = spec.label.trim().to_uppercase();
        spec.probe = spec.probe.trim().to_string();
        spec.on = spec.on.trim().to_string();
        spec.off = spec.off.trim().to_string();
        spec.alt = spec
            .alt
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty());

        if spec.label.is_empty() {
            return Err(format!("un toggle necesita label (1-{MAX_LABEL_CHARS} caracteres)"));
        }
        if spec.label.chars().count() > MAX_LABEL_CHARS {
            return Err(format!(
                "label '{}' tiene más de {MAX_LABEL_CHARS} caracteres",
                spec.label
            ));
        }
        if !is_known_probe(&spec.probe) {
            return Err(format!(
                "probe desconocida: '{}' · disponibles: {}",
                spec.probe,
                probe_ids()
            ));
        }
        for (side, id) in [("on", &spec.on), ("off", &spec.off)] {
            if !is_known_action(id) {
                return Err(format!(
                    "acción {side} desconocida: '{id}' · disponibles: {}",
                    action_ids()
                ));
            }
        }
        if let Some(alt) = &spec.alt {
            if !is_known_action(alt) {
                return Err(format!(
                    "acción alt desconocida: '{alt}' · disponibles: {}",
                    action_ids()
                ));
            }
        }

        spec.id = unique_id(&spec.id, &spec.label, &used);
        used.push(spec.id.clone());
        out.push(spec);
    }
    Ok(out)
}

fn unique_id(raw: &str, label: &str, used: &[String]) -> String {
    let raw = slug(raw);
    let base = if raw.is_empty() { slug(label) } else { raw };
    let base = if base.is_empty() {
        "toggle".to_string()
    } else {
        base
    };
    if !used.contains(&base) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !used.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn slug(raw: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(specs: Vec<ToggleSpec>) -> AppConfig {
        AppConfig {
            toggles: specs,
            ..Default::default()
        }
    }

    fn seal(reachable: bool, sealed: bool, initialized: bool) -> SealStatusDto {
        SealStatusDto {
            reachable,
            health_code: if sealed { 503 } else { 200 },
            initialized,
            sealed,
            progress: 0,
            threshold: 3,
            version: None,
            message: "sonda".into(),
        }
    }

    fn qring(running: bool, pack_ok: bool) -> QringStatusDto {
        QringStatusDto {
            running,
            pack: "C:\\pack".into(),
            host: "C:\\pack\\..".into(),
            pack_ok,
            pid: None,
            message: "sonda".into(),
            governed: true,
        }
    }

    /// A context with no Q-ring result, for the probes that do not need one.
    fn ctx<'a>(cfg: &'a AppConfig, seal: &'a SealStatusDto) -> ToggleCtx<'a> {
        ToggleCtx {
            cfg,
            seal,
            qring: None,
            gha: None,
            grant_pending: false,
        }
    }

    #[test]
    fn shipped_defaults_are_valid() {
        assert!(validate(default_toggles()).is_ok());
        assert!(validate(legacy_toggles()).is_ok());
        assert!(default_toggles().len() <= MAX_TOGGLES);
        assert!(legacy_toggles().len() <= MAX_TOGGLES);
    }

    #[test]
    fn legacy_default_is_the_three_hardcoded_leds() {
        let ids: Vec<String> = legacy_toggles().into_iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["kv", "gw", "gov"]);
        // `GW` kept its Ctrl-click restart; a fresh install ships only `KV`.
        assert_eq!(legacy_toggles()[1].alt.as_deref(), Some("qring.restart"));
        assert_eq!(default_toggles().len(), 1);
    }

    #[test]
    fn catalog_ids_are_unique_and_documented() {
        for probe in PROBES {
            assert!(is_known_probe(probe.id));
            assert!(!probe.doc.is_empty(), "{} sin doc", probe.id);
            assert!(is_known_action(probe.on), "{} on", probe.id);
            assert!(is_known_action(probe.off), "{} off", probe.id);
            if let Some(alt) = probe.alt {
                assert!(is_known_action(alt), "{} alt", probe.id);
            }
        }
        for action in ACTIONS {
            assert!(!action.doc.is_empty(), "{} sin doc", action.id);
        }
        let mut ids: Vec<&str> = PROBES.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "probe repetida");

        let mut acts: Vec<&str> = ACTIONS.iter().map(|a| a.id).collect();
        acts.sort_unstable();
        let before = acts.len();
        acts.dedup();
        assert_eq!(before, acts.len(), "acción repetida");
    }

    #[test]
    fn every_table_entry_resolves() {
        let cfg = cfg_with(legacy_toggles());
        let seal = seal(true, false, true);
        for probe in PROBES {
            let ctx = ToggleCtx {
                cfg: &cfg,
                seal: &seal,
                qring: Some(qring(true, true)),
                gha: None,
                grant_pending: false,
            };
            assert!(
                probe_state(probe.id, &ctx).is_some(),
                "{} no resuelve",
                probe.id
            );
        }
        for action in ACTIONS {
            assert!(is_known_action(action.id), "{} no es conocida", action.id);
        }
    }

    #[test]
    fn unknown_ids_are_rejected_and_the_error_names_the_known_ones() {
        let mut spec = kv();
        spec.probe = "vault.sealed".into();
        let err = validate(vec![spec]).unwrap_err();
        assert!(err.contains("vault.seal"), "{err}");

        let mut spec = kv();
        spec.on = "vault.open".into();
        let err = validate(vec![spec]).unwrap_err();
        assert!(err.contains("vault.unseal"), "{err}");

        let mut spec = kv();
        spec.alt = Some("vault.restart".into());
        let err = validate(vec![spec]).unwrap_err();
        assert!(err.contains("alt"), "{err}");

        // …and there is no way to name an action that is not in the table: the
        // only ids the config can carry are the catalog's.
        assert!(!is_known_action("run_anything"));
    }

    #[test]
    fn the_list_is_capped() {
        let six = vec![kv(); MAX_TOGGLES + 1];
        let err = validate(six).unwrap_err();
        assert!(err.contains(&MAX_TOGGLES.to_string()), "{err}");
    }

    #[test]
    fn labels_are_normalized_and_ids_made_unique() {
        let out = validate(vec![
            ToggleSpec {
                id: String::new(),
                label: "  kv  ".into(),
                probe: "vault.seal".into(),
                on: "vault.unseal".into(),
                off: "vault.seal".into(),
                alt: Some("  ".into()),
            },
            ToggleSpec {
                id: String::new(),
                label: "KV".into(),
                probe: "vault.seal".into(),
                on: "vault.unseal".into(),
                off: "vault.seal".into(),
                alt: None,
            },
        ])
        .unwrap();
        assert_eq!(out[0].label, "KV");
        assert_eq!(out[0].id, "kv");
        assert_eq!(out[1].id, "kv-2");
        assert_eq!(out[0].alt, None, "un alt vacío no es una acción");
    }

    #[test]
    fn labels_must_fit_the_pill() {
        let four = ToggleSpec {
            id: "x".into(),
            label: "SEAL".into(),
            probe: "vault.seal".into(),
            on: "vault.unseal".into(),
            off: "vault.seal".into(),
            alt: None,
        };
        assert_eq!("SEAL".chars().count(), MAX_LABEL_CHARS);
        assert!(validate(vec![four.clone()]).is_ok());
        let mut longer = four;
        longer.label = "SEALA".into();
        assert!(validate(vec![longer]).is_err());
    }

    #[test]
    fn flip_picks_the_other_side() {
        let spec = qr();
        assert_eq!(pick_action(&spec, "flip", true).unwrap(), "qring.off");
        assert_eq!(pick_action(&spec, "", false).unwrap(), "qring.on");
        assert_eq!(pick_action(&spec, "on", true).unwrap(), "qring.on");
        assert_eq!(pick_action(&spec, "alt", false).unwrap(), "qring.restart");
        assert!(pick_action(&kv(), "alt", false).is_err());
        assert!(pick_action(&kv(), "lado", false).is_err());
    }

    #[test]
    fn gw_is_the_local_runner_and_qr_is_the_gateway() {
        let gw = gw();
        assert_eq!(gw.probe, "gha.runner");
        assert_eq!(pick_action(&gw, "flip", false).unwrap(), "gha.up");
        assert_eq!(pick_action(&gw, "flip", true).unwrap(), "gha.down");
        assert_eq!(pick_action(&gw, "alt", false).unwrap(), "daemoncurier.open");

        let qr = qr();
        assert_eq!(qr.id, "qr");
        assert_eq!(qr.probe, "qring.process");
        // El proceso del Q-ring ya no comparte LED con Docker: se fue con su gesto.
        assert_eq!(pick_action(&qr, "alt", false).unwrap(), "qring.restart");
    }

    #[test]
    fn vault_probe_reports_reachability_and_initialization() {
        let cfg = cfg_with(vec![kv()]);

        let open = probe_state("vault.seal", &ctx(&cfg, &seal(true, false, true))).unwrap();
        assert!(open.on && open.live && !open.warn);

        let closed = probe_state("vault.seal", &ctx(&cfg, &seal(true, true, true))).unwrap();
        assert!(!closed.on && closed.live && !closed.warn);

        let uninitialized = probe_state("vault.seal", &ctx(&cfg, &seal(true, true, false))).unwrap();
        assert!(uninitialized.warn);

        let offline = probe_state("vault.seal", &ctx(&cfg, &seal(false, true, true))).unwrap();
        assert!(offline.warn && !offline.live, "sin respuesta no hay click");
    }

    #[test]
    fn qring_probes_are_idle_not_broken_without_a_probe_result() {
        let cfg = cfg_with(vec![gw(), gov()]);
        let seal = seal(true, false, true);
        let ctx = ToggleCtx {
            cfg: &cfg,
            seal: &seal,
            qring: None,
            gha: None,
            grant_pending: false,
        };
        let process = probe_state("qring.process", &ctx).unwrap();
        assert!(!process.on && process.warn && !process.live);

        let govern = probe_state("qring.governance", &ctx).unwrap();
        assert!(govern.on, "la política vive en la config");
        assert!(!govern.live, "sin gateway no hay nada que gobernar");
    }

    #[test]
    fn views_carry_state_and_docs_for_every_led() {
        let cfg = cfg_with(legacy_toggles());
        let seal = seal(true, false, true);
        let views = views(&cfg, &seal, true);
        assert_eq!(views.len(), 3);
        assert_eq!(views[0].spec.id, "kv");
        assert!(views[0].state.on);
        assert!(!views[0].docs.probe.is_empty());
        assert!(!views[0].docs.off.is_empty());
        assert!(views[0].docs.alt.is_none());
        // The Allow pulses on the *policy* LED: `GW` is process control only, so a
        // machine that dropped `GOV` keeps the Allow chip as its only prompt.
        assert!(!views[1].state.ask, "GW ya no lleva el pulso de Allow");
        assert!(views[1].docs.alt.as_deref().unwrap().contains("Reiniciar"));
        assert!(views[2].state.ask, "GOV lleva el Allow pendiente");
    }

    #[test]
    fn only_the_policy_led_asks_for_the_pending_allow() {
        let cfg = cfg_with(legacy_toggles());
        let seal = seal(true, false, true);

        let asking = views(&cfg, &seal, true);
        let pulse: Vec<&str> = asking
            .iter()
            .filter(|v| v.state.ask)
            .map(|v| v.spec.id.as_str())
            .collect();
        assert_eq!(pulse, vec!["gov"], "un solo LED pide, y es el de política");

        let quiet = views(&cfg, &seal, false);
        assert!(
            quiet.iter().all(|v| !v.state.ask),
            "sin Allow, ningún LED pulsa"
        );
    }

    #[test]
    fn an_empty_list_is_a_valid_header() {
        assert!(validate(Vec::new()).unwrap().is_empty());
        let cfg = cfg_with(Vec::new());
        assert!(views(&cfg, &seal(true, false, true), false).is_empty());
        assert!(!any_needs_qring(&[]));
    }
}
