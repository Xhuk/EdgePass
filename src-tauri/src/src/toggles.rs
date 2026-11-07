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

/// Hard cap. The header is a single row: with eight 3-6 letter LEDs `App.css`
/// shrinks them and the widget widens the window to fit the row whole, so the
/// layout and this number move together — never raise it without the layout.
///
/// Plan: eight slots is what the floating bar and the top dock can hold after
/// the compact rule (see `.led-toggles-compact`). A *fresh* install ships one
/// (`KV`); the rest are the owner's to add.
pub const MAX_TOGGLES: usize = 8;

/// Longest label the header pill carries without pushing the row around. With
/// eight LEDs of four characters the whole row still fits the floating bar next
/// to the drag handle and the `edgePass` label — that is the real budget.
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
///
/// Owned strings, not `&'static str`: besides the compiled-in probes the
/// catalog carries the ones derived from config — `svc.<id>` (owner services)
/// and `mcp.<id>` (each MCP row) — and those ids exist only at runtime.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeInfo {
    pub id: String,
    /// Shown in the settings picker and in the wire catalog. English, like the
    /// rest of the widget's own copy.
    pub doc: String,
    /// Resolving this probe shells out (`tasklist`) and `get_status` runs on a
    /// 5 s poll — so it is only paid for while a toggle actually asks for it.
    pub needs_qring: bool,
    /// Same rule for the local GHA runner probe: it shells out to `docker`
    /// (`info` + `inspect`, and `logs` only when something is wrong).
    pub needs_gha: bool,
    /// What a new toggle on this probe should do. The picker fills these in when
    /// the probe changes; either side can then be overridden. Empty = the probe
    /// is read-only (no click does anything).
    pub on: String,
    pub off: String,
    pub alt: Option<String>,
}

/// An action: what a click does.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionInfo {
    pub id: String,
    pub doc: String,
    /// Needs the vault open (and, for `vault.seal`, a token): the settings editor
    /// says so before anyone clicks.
    pub needs_vault: bool,
}

impl ActionInfo {
    fn new(id: &str, doc: &str, needs_vault: bool) -> Self {
        Self {
            id: id.to_string(),
            doc: doc.to_string(),
            needs_vault,
        }
    }
}

/// The compiled-in half of the catalog. `&'static str` so the table stays a
/// `const`; [`builtin_probes`] widens it to the owned shape the wire uses.
struct RawProbe {
    id: &'static str,
    doc: &'static str,
    needs_qring: bool,
    needs_gha: bool,
    on: &'static str,
    off: &'static str,
    alt: Option<&'static str>,
}

struct RawAction {
    id: &'static str,
    doc: &'static str,
    needs_vault: bool,
}

/// The published vocabulary. `get_status` and `{"op":"toggles"}` both carry it,
/// so a client never hardcodes an id — and a new probe/action is a new row here
/// plus its arm in [`probe_state`]/[`run_action`] (the tests keep the two in
/// step).
///
/// Owner-defined probes (`svc.<id>`, `mcp.<id>`) are *derived* from config at
/// catalog time; this table is only what the binary ships knowing.
const RAW_PROBES: &[RawProbe] = &[
    RawProbe {
        id: "vault.seal",
        doc: "OpenBao: open (unsealed) or sealed. Amber = not answering or not initialized.",
        needs_qring: false,
        needs_gha: false,
        on: "vault.unseal",
        off: "vault.seal",
        alt: None,
    },
    RawProbe {
        id: "qring.process",
        doc: "Q-ring gateway: qringd process alive or stopped. Amber = pack missing.",
        needs_qring: true,
        needs_gha: false,
        on: "qring.on",
        off: "qring.off",
        alt: Some("qring.restart"),
    },
    RawProbe {
        id: "qring.governance",
        doc: "Q-ring policy: whether rings may run (and spend). This is the LED that pulses when the gateway has an Allow waiting.",
        needs_qring: true,
        needs_gha: false,
        on: "qring.govern.on",
        off: "qring.govern.off",
        alt: None,
    },
    RawProbe {
        id: "docker.desktop",
        doc: "Docker Desktop: the engine answers. Amber = installed but stopped (a click starts it).",
        needs_qring: false,
        needs_gha: true,
        on: "docker.up",
        off: "gha.down",
        alt: None,
    },
    RawProbe {
        id: "gha.runner",
        doc: "Local GitHub Actions runner: Docker Desktop up AND the runner container healthy (otherwise the job queues forever). Amber = runner missing, stopped, or stuck in a credential loop.",
        needs_qring: false,
        needs_gha: true,
        on: "gha.up",
        off: "gha.down",
        alt: Some("daemoncurier.open"),
    },
    RawProbe {
        id: "skills.sync",
        doc: "Skill library: the hub links (~/.agents/skills) are current across all 50 agents. Amber = drift, or the synchronizer is missing.",
        needs_qring: false,
        needs_gha: false,
        on: "skills.sync",
        off: "skills.sync",
        alt: None,
    },
    RawProbe {
        id: "skills.watch",
        doc: "Auto-sync: a hub watcher that re-links on its own when a skill appears, is renamed or is deleted.",
        needs_qring: false,
        needs_gha: false,
        on: "skills.watch.on",
        off: "skills.watch.off",
        alt: None,
    },
    RawProbe {
        id: "mcp.servers",
        doc: "MCP servers in the list: on when they are all alive AND each declared tunnel is connected. Amber = one is missing its binary, or cloudflared is missing. The server name, its tunnel and its URL come from the MCP panel.",
        needs_qring: false,
        needs_gha: false,
        on: "mcp.on",
        off: "mcp.off",
        alt: Some("mcp.restart"),
    },
];

const RAW_ACTIONS: &[RawAction] = &[
    RawAction {
        id: "vault.unseal",
        doc: "Open the vault (unseal with the init keys).",
        needs_vault: false,
    },
    RawAction {
        id: "vault.seal",
        doc: "Seal the vault: it cuts off ALL its clients until someone opens it.",
        needs_vault: true,
    },
    RawAction {
        id: "qring.on",
        doc: "Start the Q-ring gateway. It does not turn the policy on: that is GOV.",
        needs_vault: true,
    },
    RawAction {
        id: "qring.off",
        doc: "Stop the Q-ring gateway. It also turns the policy off: a stopped gateway claiming «rings may run» would be a promise nobody can keep.",
        needs_vault: false,
    },
    RawAction {
        id: "qring.restart",
        doc: "Restart the Q-ring gateway.",
        needs_vault: true,
    },
    RawAction {
        id: "qring.govern.on",
        doc: "Let the rings run. If the gateway is stopped it starts it first (vault included, like qring.on).",
        needs_vault: true,
    },
    RawAction {
        id: "qring.govern.off",
        doc: "Block the rings: they do not run, they do not spend. It holds with the gateway up: that is the case GOV exists to cover.",
        needs_vault: false,
    },
    RawAction {
        id: "skills.sync",
        doc: "Re-link the agents from the skill hub (it copies no files). Both sides of the LED do the same: sync.",
        needs_vault: false,
    },
    RawAction {
        id: "skills.watch.on",
        doc: "Turn auto-sync on: a hub watcher that re-links when it detects a change.",
        needs_vault: false,
    },
    RawAction {
        id: "skills.watch.off",
        doc: "Turn auto-sync off (the manual button keeps working).",
        needs_vault: false,
    },
    RawAction {
        id: "mcp.on",
        doc: "Start every MCP server in the list and the named tunnel each row declares. Idempotent: what is already alive is not duplicated.",
        needs_vault: true,
    },
    RawAction {
        id: "mcp.off",
        doc: "Stop the MCP servers and their tunnel connectors. It only kills by pid the ones edgePass started; the tunnel, by pid with its name on the command line (never by image).",
        needs_vault: false,
    },
    RawAction {
        id: "mcp.restart",
        doc: "Restart every MCP server and its tunnels (the LED's Ctrl / right-click gesture).",
        needs_vault: true,
    },
    RawAction {
        id: "docker.up",
        doc: "Start Docker Desktop and wait for the engine (up to 120 s). Idempotent: if it already answers, it does nothing.",
        needs_vault: false,
    },
    RawAction {
        id: "gha.up",
        doc: "Ensure the local runner: if Docker does not answer it starts it, if the image is missing it builds it, and if the container does not exist, is stopped or is stuck in a credential loop, it recreates it with a fresh registration token (gh api).",
        needs_vault: false,
    },
    RawAction {
        id: "gha.down",
        doc: "Stop the local runner container. Docker Desktop is left alone: shutting it down would cut every other stack on the machine.",
        needs_vault: false,
    },
    RawAction {
        id: "gha.repair",
        doc: "Recreate the runner even when it looks healthy (if GitHub sees it offline): it deletes the container, mints a fresh token and creates it again.",
        needs_vault: false,
    },
    RawAction {
        id: "daemoncurier.open",
        doc: "Open DaemonCurier (the WSL2/system HUD): if it is already running it asks it for focus through its pipe; if not, it launches it.",
        needs_vault: false,
    },
];

/// Everything the outside is allowed to know about the vocabulary in one object.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub max_toggles: usize,
    pub max_label_chars: usize,
    pub probes: Vec<ProbeInfo>,
    pub actions: Vec<ActionInfo>,
}

/// The catalog for *this* config: the compiled-in vocabulary plus one probe and
/// its actions per `services[]` row (`svc.<id>`) and per `mcpServers[]` row
/// (`mcp.<id>`).
///
/// The binary stays immutable: an install that wants a new light writes a
/// service into `config.json`, and the catalog (which the UI, the pipe and the
/// skill all read) grows on its own.
pub fn catalog(cfg: &AppConfig) -> Catalog {
    let mut probes = builtin_probes();
    let mut actions = builtin_actions();
    for s in &cfg.services {
        probes.push(service_probe(s));
        if let Some(a) = service_action(s, "on") {
            actions.push(a);
        }
        if let Some(a) = service_action(s, "off") {
            actions.push(a);
        }
    }
    for s in &cfg.mcp_servers {
        probes.push(mcp_probe(s));
        for side in ["on", "off", "restart"] {
            actions.push(mcp_action_info(s, side));
        }
    }
    Catalog {
        max_toggles: MAX_TOGGLES,
        max_label_chars: MAX_LABEL_CHARS,
        probes,
        actions,
    }
}

fn builtin_probes() -> Vec<ProbeInfo> {
    RAW_PROBES
        .iter()
        .map(|p| ProbeInfo {
            id: p.id.to_string(),
            doc: p.doc.to_string(),
            needs_qring: p.needs_qring,
            needs_gha: p.needs_gha,
            on: p.on.to_string(),
            off: p.off.to_string(),
            alt: p.alt.map(str::to_string),
        })
        .collect()
}

fn builtin_actions() -> Vec<ActionInfo> {
    RAW_ACTIONS
        .iter()
        .map(|a| ActionInfo::new(a.id, a.doc, a.needs_vault))
        .collect()
}

/// The probe one `services[]` row publishes.
fn service_probe(s: &crate::services::ServiceSpec) -> ProbeInfo {
    use crate::services::ServiceProbe;
    let what = match &s.probe {
        ServiceProbe::Process {
            image,
            matcher: Some(m),
        } => format!("process {image} with «{m}»"),
        ServiceProbe::Process {
            image,
            matcher: None,
        } => format!("process {image}"),
        ServiceProbe::Cmd { cmd, .. } => format!(
            "probe `{}` (0 = on, 2 = off, 3 = amber)",
            cmd.first().map(String::as_str).unwrap_or("cmd")
        ),
    };
    ProbeInfo {
        id: s.probe_id(),
        doc: format!(
            "Own service «{}»: {what}. You defined it in services[]; the binary does not know it.",
            s.title()
        ),
        needs_qring: false,
        needs_gha: false,
        on: if s.actions.on.is_some() {
            s.on_action()
        } else {
            String::new()
        },
        off: if s.actions.off.is_some() {
            s.off_action()
        } else {
            String::new()
        },
        alt: None,
    }
}

fn service_action(s: &crate::services::ServiceSpec, side: &str) -> Option<ActionInfo> {
    let declared = match side {
        "on" => s.actions.on.as_ref(),
        _ => s.actions.off.as_ref(),
    }?;
    let verb = if side == "on" { "Start" } else { "Stop" };
    let what = declared
        .cmd
        .first()
        .map(String::as_str)
        .unwrap_or("cmd");
    Some(ActionInfo {
        id: if side == "on" {
            s.on_action()
        } else {
            s.off_action()
        },
        doc: format!(
            "{verb} «{}» by running `{what}` (the {side} action its services[] declared).",
            s.title()
        ),
        needs_vault: false,
    })
}

fn mcp_probe(s: &crate::mcp::McpServer) -> ProbeInfo {
    let label = if s.label.trim().is_empty() {
        s.id.as_str()
    } else {
        s.label.trim()
    };
    ProbeInfo {
        id: format!("mcp.{}", s.id),
        doc: format!(
            "MCP server «{label}»: alive or stopped (and its tunnel, if it declares one). One server per LED; the aggregate is mcp.servers."
        ),
        needs_qring: false,
        needs_gha: false,
        on: format!("mcp.{}.on", s.id),
        off: format!("mcp.{}.off", s.id),
        alt: Some(format!("mcp.{}.restart", s.id)),
    }
}

fn mcp_action_info(s: &crate::mcp::McpServer, side: &str) -> ActionInfo {
    let label = if s.label.trim().is_empty() {
        s.id.as_str()
    } else {
        s.label.trim()
    };
    let (doc, needs_vault) = match side {
        "on" => (format!("Start the MCP server «{label}» and its declared tunnel."), true),
        "off" => (format!("Stop the MCP server «{label}» and its tunnel."), false),
        _ => (format!("Restart the MCP server «{label}» and its tunnel."), true),
    };
    ActionInfo {
        id: format!("mcp.{}.{side}", s.id),
        doc,
        needs_vault,
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
    /// Who owns the process behind the light: `managed` (edgePass spawned it),
    /// `external` (alive, started elsewhere) or `down`. Absent for probes that
    /// are not about a process (vault, policy, skills…). Reported, never
    /// configured — the UI/agent uses it to decide whether stopping is safe.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<crate::services::Origin>,
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
    /// One entry per `svc.<id>` a LED watches, probed **in parallel** by
    /// [`crate::services::probe_many`] so three cold `cmd` probes do not add up
    /// their timeouts on the 5 s poll.
    pub services: std::collections::HashMap<String, crate::services::ServiceState>,
    /// The MCP sweep, read once when any `mcp.` LED exists (it is cached inside
    /// `mcp`, but this also keeps one snapshot consistent with itself).
    pub mcp: Option<crate::mcp::McpStatusDto>,
}

impl<'a> ToggleCtx<'a> {
    /// Probes the Q-ring status only when a toggle asks for it: the call spawns
    /// `tasklist`, and an install that removed the Q-ring LEDs should not pay for
    /// it every 5 s. The Docker/GHA probe follows the same rule (`docker info` +
    /// `docker inspect` per declared runner). Services and MCP follow it too.
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
        let service_ids: Vec<String> = cfg
            .toggles
            .iter()
            .filter_map(|t| t.probe.strip_prefix("svc.").map(str::to_string))
            .collect();
        let services = crate::services::probe_many(cfg, &service_ids);
        let mcp = if cfg.toggles.iter().any(|t| t.probe.starts_with("mcp.")) {
            Some(crate::mcp::status(cfg))
        } else {
            None
        };
        Self {
            cfg,
            seal,
            qring,
            gha,
            grant_pending,
            services,
            mcp,
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
    RAW_PROBES
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.needs_gha)
        .unwrap_or(false)
}

pub fn probe_needs_qring(id: &str) -> bool {
    RAW_PROBES
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.needs_qring)
        .unwrap_or(false)
}

/// Resolves one probe id against the config's catalog. `None` = no such probe
/// (a hand-edited `config.json`, or a service that was removed).
pub fn probe_info(cfg: &AppConfig, id: &str) -> Option<ProbeInfo> {
    catalog(cfg).probes.into_iter().find(|p| p.id == id)
}

pub fn action_info(cfg: &AppConfig, id: &str) -> Option<ActionInfo> {
    catalog(cfg).actions.into_iter().find(|a| a.id == id)
}

pub fn is_known_probe(cfg: &AppConfig, id: &str) -> bool {
    probe_info(cfg, id).is_some()
}

pub fn is_known_action(cfg: &AppConfig, id: &str) -> bool {
    action_info(cfg, id).is_some()
}

fn probe_ids(cfg: &AppConfig) -> String {
    catalog(cfg)
        .probes
        .iter()
        .map(|p| p.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn action_ids(cfg: &AppConfig) -> String {
    catalog(cfg)
        .actions
        .iter()
        .map(|a| a.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A fresh install: just the vault LED. The Q-ring pair *and* the local GHA runner
/// are in the catalog (Settings → Toggles), but an install with no Q-ring pack and no
/// Docker Desktop should not open with a row of amber lights.
/// What a *new* install shows: `KV`.
///
/// OpenBao is a prerequisite of edgePass, so the vault is the one light every
/// install has something behind. The rest (Q-ring, runner, MCP, the owner's own
/// services) stay opt-in: a light with nothing behind it is a broken light.
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
    // One catalog for the whole row: `catalog()` allocates every service and
    // MCP probe, and the old per-LED `probe_info`/`action_info` rebuilt it twice
    // per LED on every 5 s poll.
    let cat = catalog(cfg);
    cfg.toggles
        .iter()
        .map(|spec| view(spec, &ctx, &cat))
        .collect()
}
fn view(spec: &ToggleSpec, ctx: &ToggleCtx, cat: &Catalog) -> ToggleView {
    // A spec that survived validation always resolves; when one does not (a
    // hand-edited `config.json`) the LED says so instead of lying about the state.
    let mut state = probe_state(&spec.probe, ctx).unwrap_or_else(|| ToggleState {
        on: false,
        warn: true,
        ask: false,
        live: false,
        detail: format!("unknown probe: {}", spec.probe),
    });
    // A derived probe (`svc.<id>`) may be read-only: no `on`/`off` declared.
    // Then the light has nothing to click and says so by disabling the button.
    if spec.on.trim().is_empty() && spec.off.trim().is_empty() {
        state.live = false;
    }
    let action = |id: &str| {
        if id.trim().is_empty() {
            return "watch only: this LED declares no action".to_string();
        }
        cat.actions
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.doc.clone())
            .unwrap_or_else(|| format!("unknown action: {id}"))
    };
    ToggleView {
        docs: ToggleDocs {
            probe: cat
                .probes
                .iter()
                .find(|p| p.id == spec.probe)
                .map(|p| p.doc.clone())
                .unwrap_or_else(|| format!("unknown probe: {}", spec.probe)),
            on: action(&spec.on),
            off: action(&spec.off),
            alt: spec.alt.as_deref().map(action),
        },
        origin: origin_of(&spec.probe, ctx),
        state,
        spec: spec.clone(),
    }
}

/// Who owns the process behind a probe, when the probe is about one.
fn origin_of(probe: &str, ctx: &ToggleCtx) -> Option<crate::services::Origin> {
    if let Some(id) = probe.strip_prefix("svc.") {
        return ctx
            .services
            .get(id)
            .map(|s| s.origin)
            .or_else(|| crate::services::probe(ctx.cfg, id).map(|s| s.origin));
    }
    if let Some(id) = probe.strip_prefix("mcp.") {
        // `mcp.servers` is the aggregate LED, not a row.
        if id != "servers" {
            return crate::mcp::origin_of(ctx.cfg, id);
        }
    }
    None
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
                    "Q-ring with no probe: could not read its state".to_string(),
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
                Some(g) if g.docker_up => (true, "Docker Desktop answers".to_string()),
                Some(g) => (false, g.message.clone()),
                None => (
                    false,
                    "Docker with no probe: could not read its state".to_string(),
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
                    "runner with no probe: could not read its state".to_string(),
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
                    "rings may run".to_string()
                } else {
                    "rings are blocked".to_string()
                },
            })
        }
        "skills.sync" => {
            let status = crate::skills::status();
            let no_script = status
                .error
                .as_deref()
                .map(|e| e.contains("not configured") || e.contains("not found at"))
                .unwrap_or(false);
            Some(ToggleState {
                on: status.insync,
                warn: status.error.is_some() || status.drift > 0,
                ask: false,
                // Without the script there is nothing to click; with it, the button always
                // works (even when the status has not been read yet).
                live: !no_script,
                detail: match &status.error {
                    Some(msg) => msg.clone(),
                    None if status.drift > 0 => format!(
                        "{} link(s) to fix: {} missing, {} misdirected, {} dead",
                        status.drift, status.faltan, status.mal_apuntados, status.muertos
                    ),
                    None if status.skills == 0 => "reading the hub…".to_string(),
                    None => format!(
                        "{} skills in {} kinds · {} agents{}{}",
                        status.skills,
                        status.tipos,
                        status.agentes,
                        if status.anidadas > 0 {
                            format!(" · {} nested", status.anidadas)
                        } else {
                            String::new()
                        },
                        if status.omitidos > 0 {
                            format!(" · {} skipped (real directory)", status.omitidos)
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
                    "watching the hub: re-links on its own every 5 s".to_string()
                } else {
                    "off: a new skill in the hub does not reach the agents on its own".to_string()
                },
            })
        }
        "mcp.servers" => {
            let Some(status) = ctx.mcp.as_ref() else {
                return Some(ToggleState {
                    on: false,
                    warn: true,
                    ask: false,
                    live: false,
                    detail: "MCP with no probe: could not read its state".to_string(),
                });
            };
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
                    "no MCP servers: load the list in the MCP panel".to_string()
                } else {
                    status.detail.clone()
                },
            })
        }
        other => {
            // `svc.<id>` — an owner-defined service, already probed for this
            // snapshot by `ToggleCtx::new` (in parallel). The fallback re-probes
            // through the 10 s cache for hand-built contexts (tests, one-off).
            if let Some(id) = other.strip_prefix("svc.") {
                let state = ctx
                    .services
                    .get(id)
                    .cloned()
                    .or_else(|| crate::services::probe(ctx.cfg, id));
                return state.map(|s| ToggleState {
                    on: s.on,
                    warn: s.warn,
                    ask: false,
                    live: true,
                    detail: s.detail,
                });
            }
            // `mcp.<id>` — one server out of `mcpServers[]`, not the aggregate
            // `mcp.servers`. Uses the same sweep the aggregate got this snapshot.
            if let Some(id) = other.strip_prefix("mcp.") {
                if let Some(status) = ctx.mcp.as_ref() {
                    if let Some(row) = status.servers.iter().find(|v| v.spec.id == id) {
                        return Some(ToggleState {
                            on: row.running,
                            warn: row.broken,
                            ask: false,
                            live: true,
                            detail: row.detail.clone(),
                        });
                    }
                }
                return crate::mcp::server_state(ctx.cfg, id).map(|(on, broken, detail)| {
                    ToggleState {
                        on,
                        warn: broken,
                        ask: false,
                        live: true,
                        detail,
                    }
                });
            }
            None
        }
    }
}

/// Which action one click means. `flip` (the default) picks the side the probe
/// is *not* on; `on`/`off`/`alt` force one.
pub fn pick_action(spec: &ToggleSpec, dir: &str, current_on: bool) -> Result<String, String> {
    let picked = match dir {
        "" | "flip" => {
            if current_on {
                spec.off.clone()
            } else {
                spec.on.clone()
            }
        }
        "on" => spec.on.clone(),
        "off" => spec.off.clone(),
        "alt" => spec
            .alt
            .clone()
            .ok_or_else(|| format!("toggle '{}' has no alt action", spec.id))?,
        other => {
            return Err(format!(
                "unknown direction: '{other}' (on | off | alt | flip)"
            ))
        }
    };
    if picked.trim().is_empty() {
        return Err(format!(
            "toggle '{}' is read-only: its probe declares no action for '{dir}'",
            spec.id
        ));
    }
    Ok(picked)
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
            "unknown toggle: '{id}' · have: {}",
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
    // `svc.<id>.on|off` — the owner's own service, run exactly as declared.
    // The command is never synthesised here: no killing by image, no guessing.
    if let Some(rest) = id.strip_prefix("svc.") {
        let (sid, side) = rest.rsplit_once('.').ok_or_else(|| {
            format!("malformed service action: '{id}' (expected svc.<id>.on|off)")
        })?;
        let side = match side {
            "on" => crate::services::Side::On,
            "off" => crate::services::Side::Off,
            other => return Err(format!("unknown side: '{other}' (on | off)")),
        };
        let cfg = ctx.cfg.clone();
        let sid = sid.to_string();
        let detail = tokio::task::spawn_blocking(move || {
            crate::services::run_action(&cfg, &sid, side)
        })
        .await
        .map_err(|e| format!("service task: {e}"))??;
        return Ok(ActionOutcome {
            action: id.into(),
            toast: detail.clone(),
            detail,
        });
    }
    // `mcp.<id>.on|off|restart` — one row, not the whole list.
    if let Some(rest) = id.strip_prefix("mcp.") {
        if let Some((mid, side)) = rest.rsplit_once('.') {
            return mcp_one_action(ctx, mid, side).await;
        }
    }
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
                Some(n) => format!("{} · {n} keys", seal.message),
                None => seal.message.clone(),
            };
            Ok(ActionOutcome {
                action: id.into(),
                detail,
                toast: if seal.sealed {
                    seal.message
                } else {
                    "Vault open".into()
                },
            })
        }
        "vault.seal" => {
            let seal = crate::seal_vault_now(ctx.cfg).await?;
            Ok(ActionOutcome {
                action: id.into(),
                detail: seal.message.clone(),
                toast: if seal.sealed {
                    "Vault closed".into()
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
            // A full sync takes ~1 s (and up to 5 s if 21 thousand links have to be created):
            // off the async runtime, like the Q-ring CLI.
            let (report, _status) = tokio::task::spawn_blocking(crate::skills::sync_now)
                .await
                .map_err(|e| format!("skills-sync task: {e}"))??;
            let resumen = format!(
                "+{} links, ~{} fixed, -{} retired{}",
                report.creados,
                report.arreglados,
                report.retirados,
                if report.omitidos > 0 {
                    format!(" · {} skipped", report.omitidos)
                } else {
                    String::new()
                }
            );
            Ok(ActionOutcome {
                action: id.into(),
                detail: resumen.clone(),
                toast: if report.creados + report.arreglados + report.retirados == 0 {
                    "Skills were already current".into()
                } else {
                    format!("Skills synced · {resumen}")
                },
            })
        }
        "skills.watch.on" => {
            let detail = tokio::task::spawn_blocking(|| crate::skills::watch_set(true))
                .await
                .map_err(|e| format!("skills-sync task: {e}"))??;
            Ok(ActionOutcome {
                action: id.into(),
                toast: "Auto-sync on".into(),
                detail,
            })
        }
        "skills.watch.off" => {
            let detail = tokio::task::spawn_blocking(|| crate::skills::watch_set(false))
                .await
                .map_err(|e| format!("skills-sync task: {e}"))??;
            Ok(ActionOutcome {
                action: id.into(),
                toast: "Auto-sync off".into(),
                detail,
            })
        }
        "mcp.on" => mcp_action(ctx, "on").await,
        "mcp.off" => mcp_action(ctx, "off").await,
        "mcp.restart" => mcp_action(ctx, "restart").await,
        other => Err(format!(
            "unknown action: '{other}' · available: {}",
            action_ids(ctx.cfg)
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
        "on" => "started",
        "off" => "stopped",
        _ => "restarted",
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

/// One MCP row's actions. Same vault rule as the aggregate: open it **only**
/// when this row declares `secretEnv`, and never for `off`.
async fn mcp_one_action(ctx: &ActionCtx<'_>, id: &str, side: &str) -> Result<ActionOutcome, String> {
    if !matches!(side, "on" | "off" | "restart") {
        return Err(format!("unknown MCP side: '{side}' (on | off | restart)"));
    }
    let mut cfg = ctx.cfg.clone();
    let mut secrets = crate::mcp::Secrets::new();
    let wants_secrets = side != "off"
        && cfg
            .mcp_servers
            .iter()
            .find(|s| s.id == id)
            .map(|s| !s.secret_env.is_empty())
            .unwrap_or(false);
    if wants_secrets {
        let (fresh, token) = crate::ensure_ready(ctx.state).await?;
        cfg = fresh;
        secrets = crate::mcp::resolve_all(&cfg, &token).await;
    }
    let sid = id.to_string();
    let action = side.to_string();
    let res = tokio::task::spawn_blocking(move || match action.as_str() {
        "on" => crate::mcp::start(&cfg, &sid, &secrets),
        "off" => crate::mcp::stop(&cfg, &sid),
        _ => crate::mcp::restart(&cfg, &sid, &secrets),
    })
    .await
    .map_err(|e| format!("mcp task: {e}"))??;
    let verb = match side {
        "on" => "started",
        "off" => "stopped",
        _ => "restarted",
    };
    Ok(ActionOutcome {
        action: format!("mcp.{id}.{side}"),
        toast: format!("MCP {verb} · {res}"),
        detail: res,
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
        _ => "restarted",
    };
    Ok(ActionOutcome {
        action: format!("qring.{action}"),
        detail: res.message.clone(),
        toast: format!("Q-ring {verb} · {}", res.message),
    })
}

/// Turning the policy ON is a claim about the gateway: with `qringd` down the LED
/// would say "rings may run" while nothing can run, so the click starts it
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
        "rings may run"
    } else {
        "rings are blocked"
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
            "Q-ring governed OFF · nothing runs, nothing is spent".into()
        } else if started.is_some() {
            format!("Q-ring started and governed ON · {policy}")
        } else {
            format!("Q-ring governed ON · {policy}")
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
            .map(|said| said.unwrap_or_else(|| "Docker Desktop was already up".into())),
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
pub fn validate(cfg: &AppConfig, specs: Vec<ToggleSpec>) -> Result<Vec<ToggleSpec>, String> {
    if specs.len() > MAX_TOGGLES {
        return Err(format!(
            "at most {MAX_TOGGLES} toggles (got {})",
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
            return Err(format!("a toggle needs a label (1-{MAX_LABEL_CHARS} characters)"));
        }
        if spec.label.chars().count() > MAX_LABEL_CHARS {
            return Err(format!(
                "label '{}' is longer than {MAX_LABEL_CHARS} characters",
                spec.label
            ));
        }
        if !is_known_probe(cfg, &spec.probe) {
            return Err(format!(
                "unknown probe: '{}' · available: {}",
                spec.probe,
                probe_ids(cfg)
            ));
        }
        // Empty = read-only: a derived probe (`svc.<id>`) whose service declares
        // no action is a light, not a button. One side may be empty too.
        for (side, id) in [("on", &spec.on), ("off", &spec.off)] {
            if id.trim().is_empty() {
                continue;
            }
            if !is_known_action(cfg, id) {
                return Err(format!(
                    "unknown {side} action: '{id}' · available: {}",
                    action_ids(cfg)
                ));
            }
        }
        if let Some(alt) = &spec.alt {
            if !is_known_action(cfg, alt) {
                return Err(format!(
                    "unknown alt action: '{alt}' · available: {}",
                    action_ids(cfg)
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
            message: "probe".into(),
        }
    }

    fn qring(running: bool, pack_ok: bool) -> QringStatusDto {
        QringStatusDto {
            running,
            pack: "C:\\pack".into(),
            host: "C:\\pack\\..".into(),
            pack_ok,
            pid: None,
            message: "probe".into(),
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
            services: Default::default(),
            mcp: None,
        }
    }

    #[test]
    fn shipped_defaults_are_valid() {
        let cfg = AppConfig::default();
        assert!(validate(&cfg, default_toggles()).is_ok());
        assert!(validate(&cfg, legacy_toggles()).is_ok());
        assert!(default_toggles().len() <= MAX_TOGGLES);
        assert!(legacy_toggles().len() <= MAX_TOGGLES);
    }

    #[test]
    fn legacy_default_is_the_three_hardcoded_leds() {
        let ids: Vec<String> = legacy_toggles().into_iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["kv", "gw", "gov"]);
        // `GW` kept its Ctrl-click restart; a fresh install ships the vault light,
        // because OpenBao is the prerequisite every install has something behind.
        assert_eq!(legacy_toggles()[1].alt.as_deref(), Some("qring.restart"));
        assert_eq!(default_toggles().len(), 1);
        assert_eq!(default_toggles()[0].id, "kv");
    }

    #[test]
    fn catalog_ids_are_unique_and_documented() {
        let cfg = AppConfig::default();
        for probe in builtin_probes() {
            assert!(is_known_probe(&cfg, &probe.id), "{} missing", probe.id);
            assert!(!probe.doc.is_empty(), "{} has no doc", probe.id);
            assert!(is_known_action(&cfg, &probe.on), "{} on", probe.id);
            assert!(is_known_action(&cfg, &probe.off), "{} off", probe.id);
            if let Some(alt) = probe.alt {
                assert!(is_known_action(&cfg, &alt), "{} alt", probe.id);
            }
        }
        for action in builtin_actions() {
            assert!(!action.doc.is_empty(), "{} has no doc", action.id);
        }
        let mut ids: Vec<String> = builtin_probes().into_iter().map(|p| p.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate probe");

        let mut acts: Vec<String> = builtin_actions().into_iter().map(|a| a.id).collect();
        acts.sort_unstable();
        let before = acts.len();
        acts.dedup();
        assert_eq!(before, acts.len(), "duplicate action");
    }

    #[test]
    fn every_table_entry_resolves() {
        let cfg = cfg_with(legacy_toggles());
        let seal = seal(true, false, true);
        for probe in builtin_probes() {
            let ctx = ToggleCtx {
                cfg: &cfg,
                seal: &seal,
                qring: Some(qring(true, true)),
                gha: None,
                grant_pending: false,
                services: Default::default(),
                mcp: None,
            };
            assert!(
                probe_state(&probe.id, &ctx).is_some(),
                "{} does not resolve",
                probe.id
            );
        }
        for action in builtin_actions() {
            assert!(
                is_known_action(&cfg, &action.id),
                "{} is not known",
                action.id
            );
        }
    }

    /// A LED for one of the owner's services and one per MCP row: both probes
    /// and both action sets exist only because the config says so.
    #[test]
    fn derived_probes_and_actions_follow_the_config() {
        use crate::services::{ServiceAction, ServiceActions, ServiceProbe, ServiceSpec};
        let mut cfg = AppConfig::default();
        cfg.services.push(ServiceSpec {
            id: "mi-api".into(),
            label: "API".into(),
            name: "mi api".into(),
            probe: ServiceProbe::Process {
                image: "mi-api.exe".into(),
                matcher: None,
            },
            actions: ServiceActions {
                on: Some(ServiceAction {
                    cmd: vec!["start".into()],
                    cwd: String::new(),
                }),
                off: None,
            },
        });
        let cat = catalog(&cfg);
        assert!(cat.probes.iter().any(|p| p.id == "svc.mi-api"));
        assert!(cat.actions.iter().any(|a| a.id == "svc.mi-api.on"));
        assert!(
            !cat.actions.iter().any(|a| a.id == "svc.mi-api.off"),
            "no declared off action means none is published"
        );
        let probe = cat.probes.iter().find(|p| p.id == "svc.mi-api").unwrap();
        assert_eq!(probe.on, "svc.mi-api.on");
        assert!(probe.off.is_empty(), "a read-only LED does not invent an off");

        // …and `validate` accepts the derived ids without the table changing.
        let spec = ToggleSpec {
            id: "api".into(),
            label: "API".into(),
            probe: "svc.mi-api".into(),
            on: "svc.mi-api.on".into(),
            off: String::new(),
            alt: None,
        };
        assert!(validate(&cfg, vec![spec]).is_ok());

        // An MCP row publishes both sides plus the restart gesture.
        cfg.mcp_servers.push(crate::mcp::McpServer {
            id: "github".into(),
            label: "GH".into(),
            command: "node".into(),
            args: Vec::new(),
            cwd: String::new(),
            image: String::new(),
            env: Vec::new(),
            secret_env: Vec::new(),
            public_url: String::new(),
            tunnel: String::new(),
            path: "/mcp".into(),
            clients: Vec::new(),
            credential_command: String::new(),
            autostart: false,
        });
        let cat = catalog(&cfg);
        let probe = cat.probes.iter().find(|p| p.id == "mcp.github").unwrap();
        assert_eq!(probe.on, "mcp.github.on");
        assert_eq!(probe.alt.as_deref(), Some("mcp.github.restart"));
        assert!(cat.actions.iter().any(|a| a.id == "mcp.github.restart"));
    }

    #[test]
    fn unknown_ids_are_rejected_and_the_error_names_the_known_ones() {
        let cfg = AppConfig::default();
        let mut spec = kv();
        spec.probe = "vault.sealed".into();
        let err = validate(&cfg, vec![spec]).unwrap_err();
        assert!(err.contains("vault.seal"), "{err}");

        let mut spec = kv();
        spec.on = "vault.open".into();
        let err = validate(&cfg, vec![spec]).unwrap_err();
        assert!(err.contains("vault.unseal"), "{err}");

        let mut spec = kv();
        spec.alt = Some("vault.restart".into());
        let err = validate(&cfg, vec![spec]).unwrap_err();
        assert!(err.contains("alt"), "{err}");

        // …and there is no way to name an action that is not in the table: the
        // only ids the config can carry are the catalog's.
        assert!(!is_known_action(&cfg, "run_anything"));
    }

    #[test]
    fn the_list_is_capped() {
        let nine = vec![kv(); MAX_TOGGLES + 1];
        let err = validate(&AppConfig::default(), nine).unwrap_err();
        assert!(err.contains(&MAX_TOGGLES.to_string()), "{err}");
    }

    #[test]
    fn labels_are_normalized_and_ids_made_unique() {
        let out = validate(
            &AppConfig::default(),
            vec![
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
            ],
        )
        .unwrap();
        assert_eq!(out[0].label, "KV");
        assert_eq!(out[0].id, "kv");
        assert_eq!(out[1].id, "kv-2");
        assert_eq!(out[0].alt, None, "an empty alt is not an action");
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
        assert!(validate(&AppConfig::default(), vec![four.clone()]).is_ok());
        let mut longer = four;
        longer.label = "SEALA".into();
        assert!(validate(&AppConfig::default(), vec![longer]).is_err());
    }

    #[test]
    fn flip_picks_the_other_side() {
        let spec = qr();
        assert_eq!(pick_action(&spec, "flip", true).unwrap(), "qring.off");
        assert_eq!(pick_action(&spec, "", false).unwrap(), "qring.on");
        assert_eq!(pick_action(&spec, "on", true).unwrap(), "qring.on");
        assert_eq!(pick_action(&spec, "alt", false).unwrap(), "qring.restart");
        assert!(pick_action(&kv(), "alt", false).is_err());
        assert!(pick_action(&kv(), "side", false).is_err());
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
        // The Q-ring process no longer shares a LED with Docker: it left with its gesture.
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
        assert!(offline.warn && !offline.live, "no answer, no click");
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
            services: Default::default(),
            mcp: None,
        };
        let process = probe_state("qring.process", &ctx).unwrap();
        assert!(!process.on && process.warn && !process.live);

        let govern = probe_state("qring.governance", &ctx).unwrap();
        assert!(govern.on, "the policy lives in the config");
        assert!(!govern.live, "with no gateway there is nothing to govern");
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
        assert!(!views[1].state.ask, "GW no longer carries the Allow pulse");
        assert!(views[1].docs.alt.as_deref().unwrap().contains("Restart"));
        assert!(views[2].state.ask, "GOV carries the pending Allow");
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
        assert_eq!(pulse, vec!["gov"], "only one LED asks, and it is the policy one");

        let quiet = views(&cfg, &seal, false);
        assert!(
            quiet.iter().all(|v| !v.state.ask),
            "with no Allow, no LED pulses"
        );
    }

    #[test]
    fn an_empty_list_is_a_valid_header() {
        assert!(validate(&AppConfig::default(), Vec::new())
            .unwrap()
            .is_empty());
        let cfg = cfg_with(Vec::new());
        assert!(views(&cfg, &seal(true, false, true), false).is_empty());
        assert!(!any_needs_qring(&[]));
    }
}
