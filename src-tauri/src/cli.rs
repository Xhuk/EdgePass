//! `edgepass` on the command line: what makes "clone it and write one config
//! file" literally true.
//!
//! The window is still the product — `edgepass` with no arguments opens it. With
//! a subcommand the binary does one thing, prints it and exits: `init` writes the
//! config, `validate` says whether it means anything, `doctor` walks the chain
//! and names the link that is missing.
//!
//! Two Windows details worth knowing:
//!
//! - The release binary is built with `windows_subsystem = "windows"`, so it has
//!   no console of its own. `attach_console` borrows the caller's; without it a
//!   `println!` from a terminal would go nowhere.
//! - Nothing here touches the vault, the pipe or the window. A CLI that could
//!   unseal or read a secret would be a second, unaudited door into the same
//!   room; these commands only read config and process state.

use crate::config::{self, AppConfig};
use crate::mcp;
use std::path::{Path, PathBuf};

/// What the binary was asked to do.
pub enum Command {
    /// No arguments: open the widget.
    Window,
    Init { dir: Option<PathBuf>, force: bool },
    Validate { file: Option<PathBuf> },
    Doctor,
    Migrate { file: Option<PathBuf> },
    Help,
    Version,
}

/// The subcommands we answer. Anything else opens the window (a stray argument
/// from a shortcut must not turn into "unknown command, exit 2" at logon).
pub fn parse(args: &[String]) -> Command {
    let mut it = args.iter().skip(1);
    let Some(first) = it.next() else {
        return Command::Window;
    };
    let rest: Vec<String> = it.cloned().collect();
    let flag = |name: &str| rest.iter().any(|a| a == name);
    let value = |name: &str| -> Option<PathBuf> {
        rest.iter()
            .position(|a| a == name)
            .and_then(|i| rest.get(i + 1))
            .map(PathBuf::from)
    };
    match first.as_str() {
        "init" => Command::Init {
            dir: value("--dir"),
            force: flag("--force"),
        },
        "validate" => Command::Validate {
            file: value("--config").or_else(|| {
                rest.iter()
                    .find(|a| !a.starts_with("--"))
                    .map(PathBuf::from)
            }),
        },
        "doctor" => Command::Doctor,
        "migrate" => Command::Migrate {
            file: value("--config"),
        },
        "-h" | "--help" | "help" => Command::Help,
        "-V" | "--version" | "version" => Command::Version,
        _ => Command::Window,
    }
}

pub fn help() -> &'static str {
    "\
edgepass — widget de secretos sobre OpenBao

Uso:
  edgepass                       abre el widget
  edgepass init [--dir DIR]      escribe config.yaml (no toca una que ya exista)
  edgepass validate [ARCHIVO]    valida el esquema, los LEDs y los servidores MCP
  edgepass doctor                recorre la cadena y dice qué eslabón falta
  edgepass migrate [--config F]  convierte config.json a config.yaml
  edgepass --version

Variables de entorno: EDGEPASS_* (ver docs/CONFIG.md).
OpenBao es opcional para abrir el widget y obligatorio para las features de
secreto: sin vault alcanzable, los LEDs que lo necesitan quedan ámbar."
}

#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE};

    // Sólo cuando no hay ninguna salida. Si alguien redirigió (`| more`, `> out`,
    // CI) el handle es válido y AttachConsole se la robaría al padre: la salida
    // terminaría en la ventana de la consola en vez de en el pipe.
    let stdout = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if stdout.is_null() || stdout == INVALID_HANDLE_VALUE {
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

#[cfg(not(windows))]
fn attach_console() {}

/// The CLI entry point: `Some(result)` when the process should answer and exit,
/// `None` when it should open the window.
pub fn cli_main(args: &[String]) -> Option<Result<(), String>> {
    match self::parse(args) {
        Command::Window => None,
        cmd => {
            attach_console();
            Some(self::run(&cmd))
        }
    }
}

/// Runs one CLI command. `Ok(())` = the process should exit 0.
pub fn run(cmd: &Command) -> Result<(), String> {
    match cmd {
        Command::Window => Ok(()),
        Command::Help => {
            println!("{}", help());
            Ok(())
        }
        Command::Version => {
            println!("edgepass {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Init { dir, force } => init(dir.as_deref(), *force),
        Command::Validate { file } => validate(file.as_deref()),
        Command::Migrate { file } => migrate(file.as_deref()),
        Command::Doctor => doctor(),
    }
}

/// Where `init` writes: explicit `--dir`, else the data dir.
fn init_target(dir: Option<&Path>) -> Result<PathBuf, String> {
    match dir {
        Some(d) => {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
            Ok(d.join("config.yaml"))
        }
        None => Ok(config::data_dir()?.join("config.yaml")),
    }
}

/// Writes a config a fresh machine can open. Deliberately minimal: the vault is
/// the one thing a new user has to point somewhere, and even that has a default
/// (loopback), because OpenBao is optional to *open* the widget.
fn init(dir: Option<&Path>, force: bool) -> Result<(), String> {
    let path = init_target(dir)?;
    if path.exists() && !force {
        return Err(format!(
            "ya existe {} · usá --force para sobrescribirla",
            path.display()
        ));
    }
    let cfg = AppConfig::default();
    config::save_config_at(&cfg, &path)?;
    println!("config escrita en {}", path.display());
    println!();
    println!("Siguiente paso:");
    println!("  edgepass doctor      # qué falta (OpenBao, servidores MCP, túnel)");
    if path.file_name().map(|n| n != "config.yaml").unwrap_or(false) {
        println!("  (el widget abrirá esta carpeta, no ese archivo: movela a {})", config::data_dir()?.join("config.yaml").display());
    }
    Ok(())
}

/// Reads a config (the user's own by default) and validates it the way the
/// widget does at boot — same `validate` functions, so a green run here means a
/// green boot there.
fn validate(file: Option<&Path>) -> Result<(), String> {
    let path = match file {
        Some(p) => p.to_path_buf(),
        None => config::config_path()?,
    };
    if !path.exists() {
        return Err(format!(
            "no existe {} · corré `edgepass init`",
            path.display()
        ));
    }
    let cfg = config::load_config_at(&path)?;
    println!("archivo    {}", path.display());
    println!("formato    {}", if is_yaml(&path) { "YAML" } else { "JSON" });
    println!("versión    {}", cfg.config_version);

    // The LEDs: ids against the published catalog, the count, the labels.
    let toggles = crate::toggles::validate(cfg.toggles.clone())?;
    println!("LEDs       {}", toggles.len());
    for t in &toggles {
        println!("  {:<4} {} → {} / {}", t.label, t.probe, t.on, t.off);
    }

    // The MCP list: a row without a command is the classic half-configured row.
    if cfg.mcp_servers.is_empty() {
        println!("MCP        (ninguno)");
    } else {
        println!("MCP        {}", cfg.mcp_servers.len());
        for s in &cfg.mcp_servers {
            let tunnel = if s.tunnel.is_empty() {
                String::new()
            } else {
                format!(" · túnel {}", s.tunnel)
            };
            println!("  {:<12} {}{}", s.label, s.command, tunnel);
        }
    }
    println!(
        "vault      {} (mount {}{})",
        cfg.bao_addr,
        cfg.mount,
        if cfg.auto_unseal { " · auto-unseal" } else { "" }
    );
    println!();
    println!("OK");
    Ok(())
}

fn is_yaml(path: &Path) -> bool {
    matches!(
        path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()),
        Some(ext) if ext == "yaml" || ext == "yml"
    )
}

/// `config.json` → `config.yaml`. The JSON is left in place: a migration that
/// deletes the only copy of someone's settings is not a migration.
fn migrate(file: Option<&Path>) -> Result<(), String> {
    let dir = config::data_dir()?;
    let source = file
        .map(|p| p.to_path_buf())
        .or_else(|| {
            let json = dir.join("config.json");
            json.exists().then_some(json)
        })
        .ok_or_else(|| format!("no hay config.json que migrar en {}", dir.display()))?;
    let target = dir.join("config.yaml");
    let cfg = config::load_config_at(&source)?;
    if target.exists() {
        return Err(format!(
            "ya existe {} · movelo o borralo primero",
            target.display()
        ));
    }
    config::save_config_at(&cfg, &target)?;
    println!("{} → {}", source.display(), target.display());
    println!(
        "El .json queda intacto; el widget abrirá el .yaml desde el próximo arranque."
    );
    Ok(())
}

/// The chain, link by link, with the fix next to each broken one.
///
/// This is the headless version of `scripts/verify-mcp.ps1`: what a new user
/// needs is not a dashboard but a sentence — *this* is missing, run *that*.
fn doctor() -> Result<(), String> {
    let path = config::config_path()?;
    let mut problems = 0usize;
    let mut say = |ok: bool, name: &str, detail: String| {
        if !ok {
            problems += 1;
        }
        println!("  [{}] {:<28} {}", if ok { "OK  " } else { "FALLA" }, name, detail);
    };

    println!("edgePass doctor");
    println!("-----------------------------------------");

    let cfg = match config::load_config_at(&path) {
        Ok(c) => {
            say(
                true,
                "configuración",
                format!("{} (v{})", path.display(), c.config_version),
            );
            c
        }
        Err(e) => {
            say(false, "configuración", format!("{e} · corré `edgepass init`"));
            println!("-----------------------------------------");
            println!("{problems} problema(s).");
            return Ok(());
        }
    };

    // 1. El vault: opcional para abrir, obligatorio para secretos.
    let seal = {
        // Its own single-threaded runtime: the CLI must not depend on Tauri's
        // async runtime being up, because it never starts an app.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("runtime: {e}"))?;
        rt.block_on(crate::unseal::seal_status(&cfg))
    };
    if !seal.reachable {
        say(
            false,
            "OpenBao alcanzable",
            format!(
                "no responde en {} · opcional para abrir el widget, obligatorio para secretos (https://openbao.org)",
                cfg.bao_addr
            ),
        );
    } else {
        say(true, "OpenBao alcanzable", cfg.bao_addr.clone());
        say(
            !seal.sealed,
            "vault abierto",
            if seal.sealed {
                "sellado: abrí con el LED KV o `edgepass` → KV".to_string()
            } else {
                "unsealed".to_string()
            },
        );
    }

    // 2. Cada fila MCP: binario, proceso y — si declara túnel — conector.
    let status = mcp::status_now(&cfg);
    if status.total == 0 {
        say(true, "servidores MCP", "(ninguno configurado)".to_string());
    } else {
        for row in &status.servers {
            say(
                row.running,
                &format!("mcp {}", row.spec.label),
                row.detail.clone(),
            );
            if !row.spec.tunnel.is_empty() {
                say(
                    row.tunnel_running,
                    &format!("túnel {}", row.spec.tunnel),
                    if row.tunnel_running {
                        "conector arriba".to_string()
                    } else {
                        format!("parado · `cloudflared tunnel run {}`", row.spec.tunnel)
                    },
                );
            }
        }
    }

    // 3. Los LEDs: que el vocabulario exista y que la config no pida un id raro.
    match crate::toggles::validate(cfg.toggles.clone()) {
        Ok(list) => say(true, "LEDs", format!("{} en el header", list.len())),
        Err(e) => say(false, "LEDs", e),
    }

    println!("-----------------------------------------");
    if problems == 0 {
        println!("Todo en orden.");
    } else {
        println!("{problems} problema(s) — ver arriba.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        let mut v = vec!["edgepass".to_string()];
        v.extend(line.split_whitespace().map(|s| s.to_string()));
        v
    }

    #[test]
    fn no_arguments_opens_the_window() {
        assert!(matches!(parse(&args("")), Command::Window));
    }

    #[test]
    fn a_stray_argument_is_not_an_error() {
        // A shortcut with a leftover flag must still open the widget at logon.
        assert!(matches!(parse(&args("--no-sandbox")), Command::Window));
    }

    #[test]
    fn the_subcommands_and_their_flags_parse() {
        assert!(matches!(
            parse(&args("init --dir /tmp/x --force")),
            Command::Init { force: true, .. }
        ));
        match parse(&args("validate /tmp/config.yaml")) {
            Command::Validate { file } => {
                assert_eq!(file.unwrap().to_string_lossy(), "/tmp/config.yaml")
            }
            _ => panic!("not validate"),
        }
        assert!(matches!(parse(&args("doctor")), Command::Doctor));
        assert!(matches!(parse(&args("--help")), Command::Help));
        assert!(matches!(parse(&args("-V")), Command::Version));
    }
}
