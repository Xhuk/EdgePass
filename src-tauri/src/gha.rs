//! Local GitHub-Actions runner: what the `GW` LED probes and what its two actions do.
//!
//! The hosted minutes are gone, so the pipeline flipped: the heavy build runs *here*
//! (Docker Desktop), the image goes to GHCR, and the deploy job is thin — but it still
//! has to run somewhere. It runs on a **self-hosted runner in a container**: GitHub
//! orchestrates, this box executes, and the credentials come from OpenBao into the
//! container's environment ([`GhaRunnerSpec::secret_env`]), never into GitHub Secrets.
//!
//! The widget owns that container's lifecycle: is Docker Desktop answering? is the
//! runner there, running, and *not* stuck in the credential loop (`404 NotFound from
//! actions/runner-registration`, the failure that leaves runs `queued` forever)?
//! [`ensure`] fixes it, minting a fresh registration token with `gh` when that is what
//! it takes; [`stop`] only stops the runner and leaves Docker Desktop alone.
//!
//! Nothing here holds a secret value: values are read from the vault at spawn time and
//! handed to the `docker` child's *environment* (`-e KEY`, never `-e KEY=value`), so they
//! stay out of the process list too. See [`run_args`].

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const DEFAULT_IMAGE: &str = "gha-local:latest";
const DEFAULT_LABELS: &str = "self-hosted,Linux,X64,local";
/// Docker Desktop's own binary: what starts the engine when it is not answering.
const DEFAULT_DESKTOP: &str = r"C:\Program Files\Docker\Docker\Docker Desktop.exe";
/// How long we wait for Docker Desktop's engine after asking it to start.
const DOCKER_BOOT_TIMEOUT: Duration = Duration::from_secs(120);
/// The engine socket a container needs to build/push images of its own.
const DEFAULT_SOCKET: &str = "/var/run/docker.sock:/var/run/docker.sock";

fn default_image() -> String {
    DEFAULT_IMAGE.into()
}

fn default_labels() -> String {
    DEFAULT_LABELS.into()
}

fn default_socket() -> String {
    DEFAULT_SOCKET.into()
}

/// One runner: **data**, exactly like the header LEDs and the MCP list. The image is
/// generic; the repo, the labels and the credentials are what each project fills in, so
/// any project can be pointed at this widget without a line of Rust.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GhaRunnerSpec {
    /// Stable key: what the pipe addresses.
    pub id: String,
    /// Container name (`gha-local-<id>` when empty).
    #[serde(default)]
    pub name: String,
    /// `owner/repo` on GitHub — where this runner registers.
    pub repo: String,
    /// Runner labels; `runs-on: [self-hosted, linux, x64, local]` on the workflow side.
    #[serde(default = "default_labels")]
    pub labels: String,
    /// Image the container runs.
    #[serde(default = "default_image")]
    pub image: String,
    /// Named volume for the runner's work directory (per runner when empty).
    #[serde(default)]
    pub work_volume: String,
    /// Engine socket mount, `host:container`.
    #[serde(default = "default_socket")]
    pub socket_mount: String,
    /// Extra `host:container[:ro]` mounts.
    ///
    /// The VPS SSH key is the case this exists for: the vault holds its *path*
    /// (`VPS_SSH_KEY_PATH` in another app's namespace), and a path is machine config rather than
    /// a secret value - the container just needs the file to `ssh`.
    #[serde(default)]
    pub mounts: Vec<String>,
    /// Credentials read from OpenBao at spawn time (same shape the MCP list uses).
    #[serde(default)]
    pub secret_env: Vec<crate::mcp::McpSecretEnv>,
}

impl GhaRunnerSpec {
    /// The container name, derived when the spec did not pick one.
    pub fn container(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            format!("gha-local-{}", self.id.trim())
        } else {
            name.to_string()
        }
    }

    /// The image, with the shipped default when the spec left it empty (a hand-written
    /// `config.json` should not have to spell it out).
    pub fn image_or_default(&self) -> String {
        let image = self.image.trim();
        if image.is_empty() {
            DEFAULT_IMAGE.into()
        } else {
            image.to_string()
        }
    }

    /// Same for the labels.
    pub fn labels_or_default(&self) -> String {
        let labels = self.labels.trim();
        if labels.is_empty() {
            DEFAULT_LABELS.into()
        } else {
            labels.to_string()
        }
    }

    /// The socket mount, or `None` when the spec asked for no engine access at all.
    pub fn socket_mount(&self) -> Option<String> {
        let mount = self.socket_mount.trim();
        if mount.is_empty() {
            None
        } else {
            Some(mount.to_string())
        }
    }

    fn volume(&self) -> String {
        let vol = self.work_volume.trim();
        if vol.is_empty() {
            format!("gha-local-work-{}", self.id.trim())
        } else {
            vol.to_string()
        }
    }

    /// Every environment variable the container needs, minus the secrets: those are
    /// resolved per spawn and passed as bare `-e KEY`.
    fn env_keys(&self) -> Vec<(String, String)> {
        vec![
            ("RUNNER_SCOPE".into(), "repo".into()),
            (
                "REPO_URL".into(),
                format!("https://github.com/{}", self.repo.trim()),
            ),
            ("RUNNER_NAME".into(), self.container()),
            ("RUNNER_LABELS".into(), self.labels_or_default()),
            ("RUNNER_WORKDIR".into(), "/tmp/runner-work".into()),
        ]
    }
}

/// What Docker says about the container right now.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ContainerFacts {
    /// Docker's own state: `running`, `exited`, `restarting`, `created`, `dead`…
    pub state: String,
    pub restart_count: u32,
}

impl ContainerFacts {
    /// The container is up and staying up.
    pub fn running(&self) -> bool {
        self.state == "running"
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhaRunnerStatusDto {
    pub id: String,
    pub name: String,
    pub repo: String,
    pub image: String,
    pub image_present: bool,
    /// `absent` when the container does not exist at all.
    pub state: String,
    pub restart_count: u32,
    /// The log shows the registration 404: the token died and the container is looping.
    pub credential_error: bool,
    pub healthy: bool,
    /// The job in flight, if any — "just to know it is running".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<GhaJobDto>,
    pub message: String,
}

/// One job, as the runner's own stdout tells it.
///
/// The runner prints the *lifecycle* only (`Running job: X` … `Job X completed with
/// result: Succeeded`); the step output goes to `_diag/Worker_*.log` inside the container.
/// For "is it still going?" that is exactly enough: a `Running job:` with no matching
/// completion.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhaJobDto {
    pub name: String,
    /// `None` means **still running** (the completion line has not arrived).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    pub started: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished: Option<String>,
    /// Seconds since it started, when the timestamp could be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running_for_secs: Option<i64>,
}

impl GhaJobDto {
    pub fn running(&self) -> bool {
        self.result.is_none()
    }
}

/// `2026-10-14 22:02:56Z` — what the runner prints in front of every line.
fn is_runner_stamp(text: &str) -> bool {
    let t = text.trim();
    t.len() == 20
        && t.ends_with('Z')
        && t.as_bytes()[0].is_ascii_digit()
        && t.contains('-')
        && t.contains(':')
}

/// The elapsed seconds between a runner stamp and `now`, when both are readable.
pub fn elapsed_secs(stamp: &str, now: chrono::NaiveDateTime) -> Option<i64> {
    let at = chrono::NaiveDateTime::parse_from_str(stamp.trim(), "%Y-%m-%d %H:%M:%SZ").ok()?;
    Some((now - at).num_seconds().max(0))
}

/// Parses the lifecycle log into jobs. Pure, so the parsing is pinned by tests — the
/// runner's wording is the only thing tying us to it.
pub fn jobs_from_log(text: &str) -> Vec<GhaJobDto> {
    let mut jobs: Vec<GhaJobDto> = Vec::new();
    for line in text.lines() {
        let Some((stamp, rest)) = line.trim().split_once(": ") else {
            continue;
        };
        if !is_runner_stamp(stamp) {
            continue;
        }
        if let Some(name) = rest.strip_prefix("Running job: ") {
            jobs.push(GhaJobDto {
                name: name.trim().to_string(),
                result: None,
                started: stamp.trim().to_string(),
                finished: None,
                running_for_secs: None,
            });
        } else if let Some(after) = rest.strip_prefix("Job ") {
            if let Some((name, result)) = after.split_once(" completed with result: ") {
                let name = name.trim();
                if let Some(job) = jobs
                    .iter_mut()
                    .rev()
                    .find(|j| j.name == name && j.running())
                {
                    job.result = Some(result.trim().to_string());
                    job.finished = Some(stamp.trim().to_string());
                }
            }
        }
    }
    jobs
}

/// The job in flight: the last `Running job:` with no completion after it.
pub fn current_job(text: &str) -> Option<GhaJobDto> {
    jobs_from_log(text)
        .into_iter()
        .rev()
        .find(GhaJobDto::running)
}

/// `"for 2m 30s"` — for the tooltip, which has no room for a date.
pub fn humanize_secs(secs: i64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {}s", s / 60, s % 60),
        s => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhaStatusDto {
    /// Docker answers (`docker info`).
    pub docker_up: bool,
    /// Docker Desktop itself is on screen (it can be up while the engine still boots).
    pub desktop_running: bool,
    pub desktop_exe: String,
    pub dir: String,
    pub runners: Vec<GhaRunnerStatusDto>,
    pub message: String,
}

/// The one-line state the LED shows, plus what to do about it.
fn runner_message(
    spec: &GhaRunnerSpec,
    docker_up: bool,
    image_present: bool,
    facts: Option<&ContainerFacts>,
    credential_error: bool,
    job: Option<&GhaJobDto>,
) -> String {
    if !docker_up {
        return "Docker Desktop is not responding".into();
    }
    if !image_present {
        return format!(
            "missing image {} (built on start)",
            spec.image_or_default()
        );
    }
    match facts {
        None => format!("{} does not exist: created on start", spec.container()),
        Some(_) if credential_error => format!(
            "{} looping: the runner credential expired (click to recreate it)",
            spec.container()
        ),
        Some(f) if !f.running() => format!(
            "{} {} ({} restarts): click to bring it up",
            spec.container(),
            f.state,
            f.restart_count
        ),
        // Up — and if there is a job, it is said ("just to know it is running"). A
        // finished job leaves its result, which is the other half of the answer.
        Some(_) => match job {
            Some(j) if j.running() => format!(
                "{} up · job «{}»{} · repo {}",
                spec.container(),
                j.name,
                j.running_for_secs
                    .map(|s| format!(" for {}", humanize_secs(s)))
                    .unwrap_or_default(),
                spec.repo
            ),
            Some(j) => format!(
                "{} up · last job «{}» {} · repo {}",
                spec.container(),
                j.name,
                j.result.clone().unwrap_or_else(|| "no result".into()),
                spec.repo
            ),
            None => format!(
                "{} up · repo {} · labels {}",
                spec.container(),
                spec.repo,
                spec.labels_or_default()
            ),
        },
    }
}

/// Reads the whole picture in one pass. Meant for the 5 s status poll, so it shells out
/// (like the Q-ring probe) and only pays for what the declared runners need.
pub fn status(cfg: &crate::config::AppConfig) -> GhaStatusDto {
    let dir = dir_path(cfg);
    let docker_up = docker_ok(&["info", "--format", "{{.ServerVersion}}"]);
    let desktop_running = desktop_process_running();
    let mut runners = Vec::with_capacity(cfg.gha_runners.len());

    for spec in &cfg.gha_runners {
        let image = spec.image_or_default();
        let image_present = docker_up && docker_ok(&["image", "inspect", &image]);
        let facts = if docker_up {
            container_facts(&spec.container())
        } else {
            None
        };
        // Only worth reading the log when something is wrong: a healthy container's
        // log is noise, and this runs every 5 s.
        let credential_error = docker_up
            && match facts.as_ref() {
                Some(f) if !f.running() => container_log_shows_credential_error(&spec.container()),
                None => container_log_shows_credential_error(&spec.container()),
                _ => false,
            };
        let healthy = docker_up
            && image_present
            && facts.as_ref().map(ContainerFacts::running).unwrap_or(false)
            && !credential_error;
        // A job can only be running if the container is up, and reading its log costs a
        // `docker logs --tail 60` (a healthy runner's whole log is ~36 lines): that is
        // what answers "is anything running?" without opening anything.
        let job = if facts.as_ref().map(ContainerFacts::running).unwrap_or(false) {
            container_job(&spec.container())
        } else {
            None
        };
        let message = runner_message(
            spec,
            docker_up,
            image_present,
            facts.as_ref(),
            credential_error,
            job.as_ref(),
        );
        runners.push(GhaRunnerStatusDto {
            id: spec.id.clone(),
            name: spec.container(),
            repo: spec.repo.clone(),
            image,
            image_present,
            state: facts
                .as_ref()
                .map(|f| f.state.clone())
                .unwrap_or_else(|| "absent".into()),
            restart_count: facts.as_ref().map(|f| f.restart_count).unwrap_or(0),
            credential_error,
            healthy,
            job,
            message,
        });
    }

    let message = if !docker_up {
        if desktop_exe(cfg).is_file() {
            "Docker Desktop stopped: click to start it".into()
        } else {
            "Docker Desktop is not responding and I cannot find its .exe".into()
        }
    } else if cfg.gha_runners.is_empty() {
        "Docker up · no declared runners (scripts\\gha-local.ps1-Add)".into()
    } else if runners.iter().all(|r| r.healthy) {
        // With several healthy runners the aggregate summary hid the only thing that
        // matters when something runs: the job is named, and on which runner.
        let running: Vec<String> = runners
            .iter()
            .filter_map(|r| {
                r.job.as_ref().filter(|j| j.running()).map(|j| {
                    format!(
                        "job «{}» on {}{}",
                        j.name,
                        r.id,
                        j.running_for_secs
                            .map(|s| format!(" for {}", humanize_secs(s)))
                            .unwrap_or_default()
                    )
                })
            })
            .collect();
        let up = match runners.len() {
            1 => "1 runner up".to_string(),
            n => format!("{n} runners up"),
        };
        match (running.is_empty(), runners.len()) {
            (false, _) => format!("{up} · {}", running.join(" · ")),
            (true, 1) => runners[0].message.clone(),
            (true, _) => up,
        }
    } else {
        runners
            .iter()
            .find(|r| !r.healthy)
            .map(|r| r.message.clone())
            .unwrap_or_else(|| "runner half-up".into())
    };

    GhaStatusDto {
        docker_up,
        desktop_running,
        desktop_exe: desktop_exe(cfg).display().to_string(),
        dir: dir.display().to_string(),
        runners,
        message,
    }
}

/// `true` when every declared runner is healthy — what the LED calls "on".
///
/// No runners is *not* healthy: a machine that declared none has a GW LED that says so
/// instead of a green light over nothing.
pub fn all_healthy(st: &GhaStatusDto) -> bool {
    !st.runners.is_empty() && st.runners.iter().all(|r| r.healthy)
}

/// Where the viewer's text comes from: the runner's own stdout.
pub const LOG_LIFECYCLE: &str = "lifecycle";
/// …or the step output of the newest job, which lives inside the container.
pub const LOG_WORKER: &str = "worker";
/// The worker log inside the container, as the entrypoint's image lays it out.
const WORKER_LOG_DIR: &str = "/home/runner/actions-runner/_diag";

/// The runner's lifecycle lines: what is running now, and how the last ones ended.
pub fn lifecycle_log(name: &str, tail: usize) -> Result<String, String> {
    docker(&["logs", "--tail", &tail.to_string(), name])
}

/// The `sh -c` script that reads the newest worker log inside the container.
///
/// Split out because the quoting is the part that breaks silently: the whole thing
/// travels as **one** argv element to `sh -c`, so a stray quote would make the viewer
/// show nothing and nobody would notice.
pub fn worker_script(tail: usize) -> String {
    format!(
        "f=$(ls -t {WORKER_LOG_DIR}/Worker_*.log 2>/dev/null | head -1); \
         if [ -n \"$f\" ]; then printf '== %s (last %s lines) ==\\n' \"$f\" {tail}; tail -n {tail} \"$f\"; \
         else echo 'no Worker log: no job has run in this container'; fi"
    )
}

/// The step output of the newest job.
///
/// The runner keeps it in `_diag/Worker_*.log` inside the container — that is where the
/// *why* of a failure is, and it costs no GitHub round trip.
pub fn worker_log(name: &str, tail: usize) -> Result<String, String> {
    docker(&["exec", name, "sh", "-c", &worker_script(tail)])
}

/// The job in flight on a runner, with its elapsed time filled in.
pub fn container_job(name: &str) -> Option<GhaJobDto> {
    let text = lifecycle_log(name, 60).ok()?;
    let mut job = current_job(&text)?;
    job.running_for_secs = elapsed_secs(&job.started, chrono::Utc::now().naive_utc());
    Some(job)
}

/// The text the log viewer (and the `gha-logs` op) shows.
///
/// `source`: [`LOG_LIFECYCLE`], [`LOG_WORKER`] or `both`. The header per runner always
/// goes in, so a viewer window can say *which* runner it is looking at.
pub fn logs(
    cfg: &crate::config::AppConfig,
    id: Option<&str>,
    tail: usize,
    source: &str,
) -> Result<String, String> {
    let specs = pick(cfg, id)?;
    let tail = tail.clamp(1, 5000);
    let source = source.trim().to_ascii_lowercase();
    let (want_lifecycle, want_worker) = match source.as_str() {
        LOG_LIFECYCLE => (true, false),
        LOG_WORKER => (false, true),
        // `both`, and anything else: better to show too much than to stay blank.
        _ => (true, true),
    };
    let mut out = String::new();
    for spec in specs {
        let name = spec.container();
        let state = match container_facts(&name) {
            Some(f) => format!("{} ({} restarts)", f.state, f.restart_count),
            None => "does not exist".into(),
        };
        let job = container_job(&name)
            .map(|j| match j.running() {
                true => format!(
                    "job «{}» for {}",
                    j.name,
                    j.running_for_secs
                        .map(humanize_secs)
                        .unwrap_or_else(|| "?".into())
                ),
                false => format!(
                    "no job in flight (last: «{}» {})",
                    j.name,
                    j.result.unwrap_or_else(|| "?".into())
                ),
            })
            .unwrap_or_else(|| "no jobs yet".into());
        out.push_str(&format!("== {name} · {} · {state} · {job}\n", spec.repo));
        if want_lifecycle {
            out.push_str(
                &lifecycle_log(&name, tail).unwrap_or_else(|e| format!("(lifecycle: {e})")),
            );
            out.push('\n');
        }
        if want_worker {
            out.push_str(&worker_log(&name, tail).unwrap_or_else(|e| format!("(worker: {e})")));
            out.push('\n');
        }
    }
    Ok(out)
}

/// Every declared runner with a job in flight right now.
///
/// The watcher's half: only the runners that are up are asked for their log.
pub fn live_jobs(cfg: &crate::config::AppConfig) -> Vec<(String, GhaJobDto)> {
    let mut out = Vec::new();
    for spec in &cfg.gha_runners {
        let name = spec.container();
        if !container_facts(&name).map(|f| f.running()).unwrap_or(false) {
            continue;
        }
        if let Some(job) = container_job(&name) {
            out.push((spec.id.clone(), job));
        }
    }
    out
}

/// Which of those jobs are **new** since the last look.
///
/// Pure, because the rule matters: the runner's log is re-read every few seconds, so
/// "still running" must open the viewer exactly once, not once per tick.
pub fn new_jobs(
    seen: &std::collections::HashMap<String, String>,
    live: &[(String, GhaJobDto)],
) -> Vec<(String, GhaJobDto)> {
    live.iter()
        .filter(|(id, job)| {
            job.running() && seen.get(id).map(String::as_str) != Some(job.started.as_str())
        })
        .map(|(id, job)| (id.clone(), job.clone()))
        .collect()
}

/// The viewer (`scripts/gha-local.ps1`), next to the runner assets.
///
/// `ghaDir` points at `docker/gha-local`, so the repo root is two levels up. `EDGEPASS_GHA_SCRIPT`
/// overrides it for an install that keeps the assets elsewhere.
pub fn script_path(cfg: &crate::config::AppConfig) -> PathBuf {
    if let Ok(env) = std::env::var("EDGEPASS_GHA_SCRIPT") {
        let e = env.trim();
        if !e.is_empty() {
            return PathBuf::from(e);
        }
    }
    let dir = dir_path(cfg);
    let root = dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dir.clone());
    root.join("scripts").join("gha-local.ps1")
}

/// Opens the log viewer window for one runner (the script's `-Watch` mode).
///
/// The launcher itself stays hidden; the window it starts is the visible one, and it
/// keeps reading after the job ends (`-NoExit`) so the result can be read in place.
pub fn open_viewer(cfg: &crate::config::AppConfig, id: &str) -> Result<(), String> {
    let script = script_path(cfg);
    if !script.is_file() {
        let nobody_conf = cfg.gha_dir.trim().is_empty()
            && std::env::var("EDGEPASS_GHA_DIR").map(|v| v.trim().is_empty()).unwrap_or(true)
            && std::env::var("EDGEPASS_GHA_SCRIPT").map(|v| v.trim().is_empty()).unwrap_or(true);
        return Err(if nobody_conf {
            "the runner viewer is not configured: set ghaDir (config) or EDGEPASS_GHA_DIR".into()
        } else {
            format!(
                "cannot find the runner viewer at {} (set ghaDir or EDGEPASS_GHA_SCRIPT)",
                script.display()
            )
        });
    }
    let esc = |s: String| s.replace('\'', "''");
    let command = format!(
        "Start-Process -FilePath powershell -ArgumentList '-NoProfile','-NoExit','-File','{}','-Watch','-Id','{}'",
        esc(script.display().to_string()),
        esc(id.to_string())
    );
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &command]);
    hide(&mut cmd);
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open the viewer: {e}"))
}

/// `GW` off: stop the runner container. Docker Desktop is deliberately untouched —
/// stopping it would take every other stack on the machine with it.
pub fn stop(cfg: &crate::config::AppConfig, id: Option<&str>) -> Result<String, String> {
    let specs = pick(cfg, id)?;
    let mut said = Vec::new();
    for spec in specs {
        let name = spec.container();
        let facts = container_facts(&name);
        match facts {
            None => said.push(format!("{name}: did not exist")),
            Some(f) if !f.running() => {
                // Clear the wreck either way: half-stopped is what the next click undoes.
                let _ = docker(&["rm", "-f", &name]);
                said.push(format!("{name}: was {}", f.state));
            }
            Some(_) => {
                // `docker stop` prints the container *name*, so the message is built
                // here: otherwise the toast reads "gha-local-x: gha-local-x".
                docker(&["stop", &name])?;
                said.push(format!("{name}: stopped"));
            }
        }
    }
    Ok(said.join(" · "))
}

/// Resolved `secretEnv` per runner id. The async caller fills it (only it can open the
/// vault), the sync half here just consumes it — same shape the MCP list uses.
pub type SecretMap = std::collections::HashMap<String, Result<Vec<(String, String)>, String>>;

/// Reads every runner's `secretEnv` from the vault.
///
/// Called by the action before [`ensure`], and only when some runner declares one: a
/// plain runner must not unseal anything just because a LED was clicked.
pub async fn resolve_all(cfg: &crate::config::AppConfig, token: &str) -> SecretMap {
    let mut out = SecretMap::new();
    for spec in &cfg.gha_runners {
        if spec.secret_env.is_empty() {
            continue;
        }
        let pairs = crate::mcp::resolve_env_for(cfg, token, &spec.id, &spec.secret_env).await;
        out.insert(spec.id.clone(), pairs);
    }
    out
}

/// `GW` on: make Docker Desktop answer, make the image exist, and make the runner
/// container exist *and work* — recreating it with a fresh registration token when it is
/// missing, stopped, or looping on a dead credential.
pub fn ensure(
    cfg: &crate::config::AppConfig,
    id: Option<&str>,
    secrets: &SecretMap,
) -> Result<String, String> {
    let specs = pick(cfg, id)?;
    let mut said = Vec::new();
    if let Some(msg) = ensure_docker(cfg)? {
        said.push(msg);
    }
    let dir = dir_path(cfg);
    if dir.as_os_str().is_empty() || !dir.is_dir() {
        // Without the assets folder there is no compose to fill: the how, not another machine's path.
        return Err("ghaDir is not set: point it at the directory holding the runner assets (ghaDir in the config or EDGEPASS_GHA_DIR)".into());
    }
    for spec in specs {
        said.push(ensure_runner(spec, &dir, secrets)?);
    }
    Ok(said.join(" · "))
}

/// Starts Docker Desktop if the engine is not answering, and waits for it.
///
/// `None` when it was already up (nothing to say).
pub fn ensure_docker(cfg: &crate::config::AppConfig) -> Result<Option<String>, String> {
    if docker_ok(&["info", "--format", "{{.ServerVersion}}"]) {
        return Ok(None);
    }
    let exe = desktop_exe(cfg);
    if !exe.is_file() {
        return Err(format!(
            "Docker is not responding and I cannot find Docker Desktop at {}",
            exe.display()
        ));
    }
    let mut cmd = Command::new(&exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map_err(|e| format!("could not start Docker Desktop: {e}"))?;

    let deadline = Instant::now() + DOCKER_BOOT_TIMEOUT;
    while Instant::now() < deadline {
        if docker_ok(&["info", "--format", "{{.ServerVersion}}"]) {
            return Ok(Some("Docker Desktop started".into()));
        }
        std::thread::sleep(Duration::from_secs(3));
    }
    Err(format!(
        "I started Docker Desktop but the engine did not answer within {}s",
        DOCKER_BOOT_TIMEOUT.as_secs()
    ))
}

/// `GW` alt in the raw: recreate the container even when it looks healthy — the case
/// where Docker is fine but GitHub sees the runner `offline` (zombie listener).
pub fn repair(cfg: &crate::config::AppConfig, secrets: &SecretMap) -> Result<String, String> {
    let specs = pick(cfg, None)?;
    let mut said = Vec::new();
    if let Some(msg) = ensure_docker(cfg)? {
        said.push(msg);
    }
    for spec in specs {
        let _ = docker(&["rm", "-f", &spec.container()]);
        said.push(recreate(spec, secrets, "forced")?);
    }
    Ok(said.join(" · "))
}

fn ensure_runner(
    spec: &GhaRunnerSpec,
    dir: &PathBuf,
    secrets: &SecretMap,
) -> Result<String, String> {
    let name = spec.container();
    if spec.repo.trim().is_empty() {
        return Err(format!("{}: repo missing (owner/repo)", spec.id));
    }

    if !docker_ok(&["image", "inspect", &spec.image_or_default()]) {
        let dockerfile = dir.join("Dockerfile");
        if !dockerfile.is_file() {
            return Err(format!(
                "missing image {} and cannot find {} (set ghaDir)",
                spec.image_or_default(),
                dockerfile.display()
            ));
        }
        let out = docker(&[
            "build",
            "-t",
            &spec.image_or_default(),
            &dir.display().to_string(),
        ])?;
        crate::log_info!("gha build {}: {}", spec.image_or_default(), last_line(&out));
    }

    let facts = container_facts(&name);
    let healthy_now = facts.as_ref().map(ContainerFacts::running).unwrap_or(false)
        && !container_log_shows_credential_error(&name);
    if healthy_now {
        return Ok(format!("{name}: was already up"));
    }

    // A container that is there but not working is removed first: half-created state is
    // what breaks the next `docker run` (name in use, half-written work dir).
    let why = match facts.as_ref() {
        None => String::new(),
        Some(_) if container_log_shows_credential_error(&name) => "credential expired".into(),
        Some(f) if f.running() => "restarting".into(),
        Some(f) => f.state.clone(),
    };
    if facts.is_some() {
        docker(&["rm", "-f", &name])?;
    }
    recreate(spec, secrets, &why)
}

/// Removes nothing: mints a token and runs the container. Callers decide about the old
/// one (so `ensure` never removes a healthy container to "fix" it).
fn recreate(spec: &GhaRunnerSpec, secrets: &SecretMap, why: &str) -> Result<String, String> {
    let name = spec.container();
    // Secrets only ever travel as environment: the value goes into the `docker`
    // child's own env, the argv carries the bare key.
    let mut envs = spec.env_keys();
    envs.push(("RUNNER_TOKEN".into(), registration_token(&spec.repo)?));
    if !spec.secret_env.is_empty() {
        let pairs = match secrets.get(&spec.id) {
            Some(Ok(pairs)) => pairs.clone(),
            Some(Err(e)) => return Err(format!("{}: {e}", spec.id)),
            None => {
                return Err(format!(
                    "{}: the action did not resolve its secrets before starting",
                    spec.id
                ))
            }
        };
        envs.extend(pairs);
    }

    let args = run_args(spec, &envs);
    let out = docker_with_env(&args, &envs)?;
    let created = out.trim();
    let created = if created.is_empty() { &name } else { created };
    Ok(match why.is_empty() {
        true => format!("{created}: created"),
        false => format!("{created}: recreated ({why})"),
    })
}

/// `docker run -d …` for one runner.
///
/// `envs` are the variable *names* plus the values: the names go to argv as bare
/// `-e KEY`, the values to the child's environment. That is the whole reason this is a
/// separate, pure function — the rule is testable and the secret never lands in argv.
pub fn run_args(spec: &GhaRunnerSpec, envs: &[(String, String)]) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        spec.container(),
        "--restart".to_string(),
        "unless-stopped".to_string(),
    ];
    for (key, _) in envs {
        args.push("-e".to_string());
        args.push(key.clone());
    }
    args.push("-v".to_string());
    args.push(format!("{}:/tmp/runner-work", spec.volume()));
    if let Some(mount) = spec.socket_mount() {
        args.push("-v".to_string());
        args.push(mount);
    }
    for mount in &spec.mounts {
        let mount = mount.trim();
        if mount.is_empty() {
            continue;
        }
        args.push("-v".to_string());
        args.push(mount.to_string());
    }
    args.push(spec.image_or_default());
    args
}

/// The registration token for a repo, minted on demand and short-lived (~1 h).
///
/// It is a secret: it goes straight into the child's environment, never into a log line.
pub fn registration_token(repo: &str) -> Result<String, String> {
    let mut cmd = Command::new("gh");
    cmd.args([
        "api",
        "-X",
        "POST",
        &format!("repos/{}/actions/runners/registration-token", repo.trim()),
        "--jq",
        ".token",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("could not run gh: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "gh could not mint the runner token for {repo}: {}",
            err.trim()
        ));
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if token.is_empty() {
        return Err(format!("gh returned an empty token for {repo}"));
    }
    Ok(token)
}

fn pick<'a>(
    cfg: &'a crate::config::AppConfig,
    id: Option<&str>,
) -> Result<Vec<&'a GhaRunnerSpec>, String> {
    match id {
        Some(want) => cfg
            .gha_runners
            .iter()
            .find(|s| s.id == want)
            .map(|s| vec![s])
            .ok_or_else(|| {
                format!(
                    "unknown runner: '{want}' · available: {}",
                    cfg.gha_runners
                        .iter()
                        .map(|s| s.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }),
        None if cfg.gha_runners.is_empty() => Err(
            "no declared runners: add one with scripts\\gha-local.ps1 -Add -Repo owner/repo"
                .into(),
        ),
        None => Ok(cfg.gha_runners.iter().collect()),
    }
}

/// The runner assets' dir is wherever the user points (`ghaDir` in config or
/// `EDGEPASS_GHA_DIR`) or **nothing** — a fresh install has no runners, and the
/// actions say exactly what is missing instead of baking this repo's layout in.
fn dir_path(cfg: &crate::config::AppConfig) -> PathBuf {
    let raw = cfg.gha_dir.trim();
    if !raw.is_empty() {
        return PathBuf::from(raw);
    }
    if let Ok(env) = std::env::var("EDGEPASS_GHA_DIR") {
        let e = env.trim();
        if !e.is_empty() {
            return PathBuf::from(e);
        }
    }
    PathBuf::new()
}

fn desktop_exe(cfg: &crate::config::AppConfig) -> PathBuf {
    let raw = cfg.docker_desktop_exe.trim();
    if !raw.is_empty() {
        return PathBuf::from(raw);
    }
    if let Ok(env) = std::env::var("EDGEPASS_DOCKER_DESKTOP") {
        let e = env.trim();
        if !e.is_empty() {
            return PathBuf::from(e);
        }
    }
    PathBuf::from(DEFAULT_DESKTOP)
}

fn docker(args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("docker");
    cmd.args(args);
    hide(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("could not run docker: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        return Err(if err.is_empty() {
            format!("docker {} failed", args.first().unwrap_or(&"?"))
        } else {
            err.to_string()
        });
    }
    Ok(stdout)
}

/// Like [`docker`], but with `values` in the child's environment: the argv carries the
/// bare keys, the secrets never do.
fn docker_with_env(args: &[String], values: &[(String, String)]) -> Result<String, String> {
    let mut cmd = Command::new("docker");
    cmd.args(args);
    for (key, value) in values {
        cmd.env(key, value);
    }
    hide(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("could not run docker: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "docker {} failed: {}",
            args.first().map(String::as_str).unwrap_or("?"),
            err.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn docker_ok(args: &[&str]) -> bool {
    let mut cmd = Command::new("docker");
    cmd.args(args);
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    hide(&mut cmd);
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

fn hide(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// `docker inspect`, parsed into the two facts the LED needs.
pub fn container_facts(name: &str) -> Option<ContainerFacts> {
    let mut cmd = Command::new("docker");
    cmd.args([
        "inspect",
        name,
        "--format",
        "{{.State.Status}}|{{.RestartCount}}",
    ]);
    hide(&mut cmd);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    facts_from_inspect(&text)
}

/// The parse half of [`container_facts`], pure so the format is pinned by a test.
pub fn facts_from_inspect(text: &str) -> Option<ContainerFacts> {
    let (state, restarts) = text.trim().split_once('|')?;
    Some(ContainerFacts {
        state: state.trim().to_string(),
        restart_count: restarts.trim().parse().unwrap_or(0),
    })
}

/// Does the container's log carry the failure that leaves runs queued forever?
///
/// The runner says `404 Not Found` while registering (or `Not configured. Run config.sh`
/// when it never got configured) — the fix is a fresh registration token, not a restart.
pub fn container_log_shows_credential_error(name: &str) -> bool {
    let mut cmd = Command::new("docker");
    cmd.args(["logs", "--tail", "40", name]);
    hide(&mut cmd);
    let out = match cmd.output() {
        Ok(out) => out,
        Err(_) => return false,
    };
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    credential_error_from_logs(&text)
}

/// The parse half, pure: which lines mean "the credential died".
pub fn credential_error_from_logs(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "404 not found",
        "runner-registration",
        "not configured. run config.sh",
        "the runner registration failed",
        "cannot connect to the server",
        // The twin case: the token is valid but GitHub no longer accepts this runner
        // version. The way out is the same — recreate with a newer image.
        "out of date and can no longer register",
        // Half-done configuration (`.runner` without `.credentials`): the entrypoint
        // recovers on its own, but if an old log shows it, the LED names the cause.
        "already configured",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// Is Docker Desktop's UI up? (The engine answering is a separate question.)
pub fn desktop_process_running() -> bool {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("tasklist");
        cmd.args(["/FI", "IMAGENAME eq Docker Desktop.exe", "/NH"]);
        hide(&mut cmd);
        match cmd.output() {
            Ok(out) => String::from_utf8_lossy(&out.stdout)
                .to_ascii_lowercase()
                .contains("docker desktop.exe"),
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> GhaRunnerSpec {
        GhaRunnerSpec {
            id: "cb2".into(),
            name: String::new(),
            repo: "Xhuk/ContinuityBridge2.0".into(),
            labels: String::new(),
            image: String::new(),
            work_volume: String::new(),
            socket_mount: String::new(),
            mounts: Vec::new(),
            secret_env: Vec::new(),
        }
    }

    #[test]
    fn a_spec_without_names_gets_derived_ones() {
        let s = spec();
        assert_eq!(s.container(), "gha-local-cb2");
        assert_eq!(s.volume(), "gha-local-work-cb2");
        // Empty fields fall back to the product defaults, not to nothing: a
        // hand-written `config.json` should only need id + repo.
        assert_eq!(s.image_or_default(), DEFAULT_IMAGE);
        assert_eq!(s.labels_or_default(), DEFAULT_LABELS);
    }

    #[test]
    fn an_explicitly_empty_socket_means_no_engine_access() {
        let mut s = spec();
        s.socket_mount = "".into();
        let args = run_args(&s, &[]);
        assert!(!args.iter().any(|a| a.contains("docker.sock")), "{args:?}");

        s.socket_mount = "//var/run/docker.sock://var/run/docker.sock".into();
        let args = run_args(&s, &[]);
        assert!(
            args.iter()
                .any(|a| a == "//var/run/docker.sock://var/run/docker.sock"),
            "{args:?}"
        );
    }

    /// Extra `mounts` go through as-is (the VPS key is the real case) and an empty
    /// entry does not produce a `-v ""` that would break the `docker run`.
    #[test]
    fn extra_mounts_are_passed_through_and_empty_ones_ignored() {
        let mut s = spec();
        s.socket_mount = DEFAULT_SOCKET.into();
        s.mounts = vec![
            r"D:\keys\vps:/keys/vps:ro".into(),
            "   ".into(),
        ];
        let args = run_args(&s, &[]);
        let joined = args.join(" ");
        assert!(
            joined.contains(r"-v D:\keys\vps:/keys/vps:ro"),
            "{joined}"
        );
        assert!(!joined.contains(r#"-v ""#), "{joined}");
        assert_eq!(
            args.iter().filter(|a| a.as_str() == "-v").count(),
            3,
            "work + socket + one real mount: {joined}"
        );
    }

    #[test]
    fn serde_fills_the_generic_defaults() {
        let s: GhaRunnerSpec =
            serde_json::from_str(r#"{"id":"cb2","repo":"Xhuk/ContinuityBridge2.0"}"#).unwrap();
        assert_eq!(s.name, "");
        assert_eq!(s.labels, DEFAULT_LABELS);
        assert_eq!(s.image, DEFAULT_IMAGE);
        assert_eq!(s.socket_mount, DEFAULT_SOCKET);
        assert!(s.secret_env.is_empty());
        assert_eq!(s.container(), "gha-local-cb2");
    }

    #[test]
    fn the_argv_never_carries_a_secret_value() {
        let mut s = spec();
        s.socket_mount = DEFAULT_SOCKET.into();
        s.secret_env = vec![crate::mcp::McpSecretEnv {
            key: "CR_PAT".into(),
            namespace: String::new(),
            path: "integrations/ghcr".into(),
            field: "token".into(),
        }];
        let envs = vec![
            ("REPO_URL".to_string(), "https://github.com/x/y".to_string()),
            ("RUNNER_TOKEN".to_string(), "sup3r-s3cret".to_string()),
            ("CR_PAT".to_string(), "ghp_also-secret".to_string()),
        ];
        let args = run_args(&s, &envs);
        let line = args.join(" ");
        assert!(!line.contains("sup3r-s3cret"), "{line}");
        assert!(!line.contains("ghp_also-secret"), "{line}");
        assert!(!line.contains('='), "no -e KEY=value: {line}");
        for key in ["RUNNER_TOKEN", "CR_PAT", "REPO_URL"] {
            assert!(
                line.contains(&format!("-e {key} ")),
                "missing -e {key}: {line}"
            );
        }
        // The rest of the contract: detached, restarting, work volume, engine socket.
        assert!(line.starts_with("run -d --name gha-local-cb2 --restart unless-stopped"));
        assert!(line.contains("-v gha-local-work-cb2:/tmp/runner-work"));
        assert!(line.contains("-v /var/run/docker.sock:/var/run/docker.sock"));
        assert!(line.ends_with(DEFAULT_IMAGE));
    }

    #[test]
    fn inspect_and_logs_are_parsed_where_it_matters() {
        assert_eq!(
            facts_from_inspect("running|0"),
            Some(ContainerFacts {
                state: "running".into(),
                restart_count: 0
            })
        );
        assert!(facts_from_inspect("running|0").unwrap().running());
        assert_eq!(facts_from_inspect("restarting|7").unwrap().restart_count, 7);
        assert!(!facts_from_inspect("exited|1").unwrap().running());
        assert_eq!(facts_from_inspect("garbage"), None);
    }

    #[test]
    fn the_credential_loop_is_recognised_and_health_is_not_alarmist() {
        let bad = "2026-01-01 ERROR: Request to https://api.github.com/.../actions/runner-registration failed with 404 Not Found";
        assert!(credential_error_from_logs(bad));
        assert!(credential_error_from_logs("Not configured. Run config.sh."));
        // Valid token but an old runner version: same diagnosis, same way out.
        assert!(credential_error_from_logs(
            "ERROR: Your runner version is out of date and can no longer register with GitHub."
        ));

        let fine = "√ Connected to GitHub\n2026-01-01 Listening for Jobs";
        assert!(!credential_error_from_logs(fine));
    }

    #[test]
    fn a_runner_is_only_reported_as_on_when_it_really_is() {
        let s = spec();
        // It cannot be alive without Docker, without the image, or with a broken log.
        assert!(!runner_message(&s, false, true, None, false, None).contains(" up"));
        assert!(runner_message(&s, false, true, None, false, None).contains("Docker Desktop"));
        assert!(runner_message(&s, true, false, None, false, None).contains("missing image"));

        let up = ContainerFacts {
            state: "running".into(),
            restart_count: 0,
        };
        assert!(runner_message(&s, true, true, Some(&up), false, None).contains(" up"));
        assert!(runner_message(&s, true, true, Some(&up), true, None).contains("expired"));

        let looped = ContainerFacts {
            state: "restarting".into(),
            restart_count: 4,
        };
        assert!(runner_message(&s, true, true, Some(&looped), false, None).contains("4 restarts"));
    }

    /// "Just to know it is running": the LED says it, with the job name and how long
    /// it has been going.
    #[test]
    fn the_led_says_which_job_is_running() {
        let s = spec();
        let up = ContainerFacts {
            state: "running".into(),
            restart_count: 0,
        };
        let running = GhaJobDto {
            name: "Checkout + backend (Go)".into(),
            result: None,
            started: "2026-10-14 22:03:02Z".into(),
            finished: None,
            running_for_secs: Some(150),
        };
        let said = runner_message(&s, true, true, Some(&up), false, Some(&running));
        assert!(said.contains("job «Checkout + backend (Go)»"), "{said}");
        assert!(said.contains("for 2m 30s"), "{said}");

        let done = GhaJobDto {
            result: Some("Failed".into()),
            finished: Some("2026-10-14 22:15:13Z".into()),
            running_for_secs: None,
            ..running
        };
        let said = runner_message(&s, true, true, Some(&up), false, Some(&done));
        assert!(said.contains("last job"), "{said}");
        assert!(said.contains("Failed"), "{said}");
    }

    /// The runner's lifecycle log: which job runs now and how the previous ones ended.
    /// The runner's own wording fixes it, so the parsing comes with tests.
    #[test]
    fn the_lifecycle_log_tells_which_job_is_running() {
        let log = "\
2026-10-14 22:02:56Z: Running job: Who runs and with what
2026-10-14 22:03:01Z: Job Who runs and with what completed with result: Succeeded
2026-10-14 22:03:02Z: Running job: Checkout + backend (Go)
";
        let jobs = jobs_from_log(log);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].result.as_deref(), Some("Succeeded"));
        assert!(jobs[0].finished.is_some());

        let current = current_job(log).expect("one is running");
        assert_eq!(current.name, "Checkout + backend (Go)");
        assert!(current.running());
        assert_eq!(current.running_for_secs, None, "container_job fills it");

        // Cut before the second `Running job`: nothing is in flight.
        let older = &log[..log.find("2026-10-14 22:03:02Z").unwrap()];
        assert!(current_job(older).is_none());
    }

    #[test]
    fn a_failed_job_is_a_result_not_a_running_one() {
        let log = "2026-10-14 22:00:00Z: Running job: build\n2026-10-14 22:00:05Z: Job build completed with result: Failed\n";
        assert!(current_job(log).is_none());
        assert_eq!(jobs_from_log(log)[0].result.as_deref(), Some("Failed"));
    }

    #[test]
    fn garbage_in_the_log_does_not_invent_jobs() {
        assert!(jobs_from_log("").is_empty());
        assert!(jobs_from_log("√ Connected to GitHub\nListening for Jobs").is_empty());
        // A line with a timestamp but no known verb is ignored.
        assert!(jobs_from_log("2026-10-14 22:00:00Z: something odd").is_empty());
        // A repeated job is not confused with the one that already finished.
        let log = "2026-10-14 22:00:00Z: Running job: x\n2026-10-14 22:00:01Z: Job x completed with result: Succeeded\n2026-10-14 22:00:02Z: Running job: x\n";
        let jobs = jobs_from_log(log);
        assert_eq!(jobs.len(), 2);
        assert!(jobs[1].running(), "the second x is still running");
    }

    #[test]
    fn elapsed_and_humanize_are_readable() {
        let now =
            chrono::NaiveDateTime::parse_from_str("2026-10-14 22:05:30Z", "%Y-%m-%d %H:%M:%SZ")
                .unwrap();
        assert_eq!(elapsed_secs("2026-10-14 22:03:00Z", now), Some(150));
        assert_eq!(elapsed_secs("not a stamp", now), None);
        assert_eq!(humanize_secs(45), "45s");
        assert_eq!(humanize_secs(150), "2m 30s");
        assert_eq!(humanize_secs(3700), "1h 1m");
    }

    #[test]
    fn the_worker_log_command_survives_the_container_quoting() {
        // The script travels as a single argument to `sh -c` inside the container: if the
        // quotes break, the viewer shows nothing and nobody finds out.
        let script = worker_script(200);
        assert!(
            script.contains("ls -t /home/runner/actions-runner/_diag/Worker_*.log"),
            "{script}"
        );
        assert!(script.contains("tail -n 200 \"$f\""), "{script}");
        assert!(script.contains("if [ -n \"$f\" ]"), "{script}");
    }

    /// The viewer opens once per job, not once per watcher tick.
    #[test]
    fn the_watcher_announces_each_job_once() {
        let mut seen = std::collections::HashMap::new();
        let job = GhaJobDto {
            name: "build".into(),
            result: None,
            started: "2026-10-14 22:03:02Z".into(),
            finished: None,
            running_for_secs: Some(5),
        };
        let live = vec![("my-app".to_string(), job.clone())];

        let first = new_jobs(&seen, &live);
        assert_eq!(first.len(), 1, "the first one is announced");
        seen.insert("my-app".to_string(), job.started.clone());
        assert!(
            new_jobs(&seen, &live).is_empty(),
            "the same job is not repeated"
        );

        // Another job (or another start of the same name) is announced again.
        let later = GhaJobDto {
            started: "2026-10-14 22:20:00Z".into(),
            ..job.clone()
        };
        let live = vec![("my-app".to_string(), later)];
        assert_eq!(new_jobs(&seen, &live).len(), 1);

        // And a finished job opens nothing.
        let done = GhaJobDto {
            result: Some("Succeeded".into()),
            ..job
        };
        let live = vec![("my-app".to_string(), done)];
        assert!(new_jobs(&seen, &live).is_empty());
    }

    #[test]
    fn the_viewer_and_the_assets_are_where_the_config_says() {
        let mut cfg = crate::config::AppConfig::default();
        cfg.gha_dir = r"D:\other\docker\gha-local".into();
        assert_eq!(
            script_path(&cfg),
            PathBuf::from(r"D:\other\scripts\gha-local.ps1"),
            "two levels above ghaDir"
        );
    }

    /// With two declared runners, the summary must still say what runs: that was
    /// exactly the case where it got lost ("2 runners up" and nothing else).
    #[test]
    fn the_summary_still_names_a_running_job_with_more_than_one_runner() {
        let mut cfg = crate::config::AppConfig::default();
        let mut a = spec();
        a.id = "one".into();
        a.repo = "Xhuk/one".into();
        let mut b = spec();
        b.id = "two".into();
        b.repo = "Xhuk/two".into();
        cfg.gha_runners = vec![a, b];

        let healthy = |id: &str, job: Option<GhaJobDto>| GhaRunnerStatusDto {
            id: id.into(),
            name: format!("gha-local-{id}"),
            repo: format!("Xhuk/{id}"),
            image: DEFAULT_IMAGE.into(),
            image_present: true,
            state: "running".into(),
            restart_count: 0,
            credential_error: false,
            healthy: true,
            job,
            message: format!("gha-local-{id} up"),
        };
        let running = GhaJobDto {
            name: "Checkout + backend (Go)".into(),
            result: None,
            started: "2026-10-14 23:50:00Z".into(),
            finished: None,
            running_for_secs: Some(90),
        };
        let st = GhaStatusDto {
            docker_up: true,
            desktop_running: true,
            desktop_exe: "x".into(),
            dir: "y".into(),
            runners: vec![healthy("one", None), healthy("two", Some(running))],
            message: String::new(),
        };
        assert!(all_healthy(&st));

        // The same aggregation `status` does, without touching Docker.
        let running_txt: Vec<String> = st
            .runners
            .iter()
            .filter_map(|r| {
                r.job
                    .as_ref()
                    .filter(|j| j.running())
                    .map(|j| format!("job «{}» on {}", j.name, r.id))
            })
            .collect();
        assert_eq!(running_txt, vec!["job «Checkout + backend (Go)» on two"]);
    }
}
