//! edgePass "autostart": Scheduled Task at user logon (not a Windows Service).
//!
//! GUI apps cannot live in Session 0. The task runs the exe directly (native
//! action, no PowerShell wrapper) with an 8 s delay for the logon burst; a vault
//! that is still starting is a state the widget renders, not something to wait
//! for here.

use serde::Serialize;

use crate::winservice::run_hidden;

const TASK_NAME: &str = "edgePass";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutostartStatusDto {
    pub installed: bool,
    pub task_name: String,
    pub exe: String,
    pub message: String,
}

pub fn status() -> AutostartStatusDto {
    let exe = current_exe_display();
    match query_task() {
        Ok(true) => AutostartStatusDto {
            installed: true,
            task_name: TASK_NAME.into(),
            exe,
            message: format!("tarea '{TASK_NAME}' al iniciar sesión"),
        },
        Ok(false) => AutostartStatusDto {
            installed: false,
            task_name: TASK_NAME.into(),
            exe,
            message: "no instalado".into(),
        },
        Err(e) => AutostartStatusDto {
            installed: false,
            task_name: TASK_NAME.into(),
            exe,
            message: e,
        },
    }
}

pub fn install() -> Result<AutostartStatusDto, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if !exe.is_file() {
        return Err(format!("exe missing: {}", exe.display()));
    }
    let exe_s = exe.display().to_string();
    let exe_ps = exe_s.replace('\'', "''");

    // A native task action, not a PowerShell wrapper: fewer moving parts, no
    // `-ExecutionPolicy Bypass` for AV to flag, and the path Scoop exposes
    // (`...\apps\edgepass\current\edgepass.exe`) stays valid across updates.
    // The 8 s delay is for the logon burst, not for OpenBao: a vault that is
    // still starting is a state the widget already renders ("OpenBao offline"
    // plus the Svc button), so waiting for it here was not load-bearing.
    let ps = format!(
        r#"$ErrorActionPreference = 'Stop'
$action = New-ScheduledTaskAction -Execute '{exe_ps}'
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$trigger.Delay = 'PT8S'
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero)
Register-ScheduledTask -TaskName '{TASK_NAME}' -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null
'OK'
"#
    );

    let out = run_hidden("powershell.exe", &["-NoProfile", "-Command", &ps])?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        return Err(format!(
            "Register-ScheduledTask failed: {} {}",
            err.trim(),
            stdout.trim()
        ));
    }

    crate::log_info!("autostart instalado · task={TASK_NAME} exe={exe_s}");
    Ok(AutostartStatusDto {
        installed: true,
        task_name: TASK_NAME.into(),
        exe: exe_s,
        message: format!("OK · arranca al logon · {TASK_NAME}"),
    })
}

pub fn remove() -> Result<AutostartStatusDto, String> {
    let out = run_hidden(
        "powershell.exe",
        &[
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &format!(
                "Unregister-ScheduledTask -TaskName '{TASK_NAME}' -Confirm:$false -ErrorAction SilentlyContinue; 'OK'"
            ),
        ],
    )?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("remove task failed: {}", err.trim()));
    }
    crate::log_info!("autostart eliminado · task={TASK_NAME}");
    Ok(AutostartStatusDto {
        installed: false,
        task_name: TASK_NAME.into(),
        exe: current_exe_display(),
        message: "autostart quitado".into(),
    })
}

fn query_task() -> Result<bool, String> {
    let out = run_hidden(
        "schtasks.exe",
        &["/Query", "/TN", TASK_NAME, "/FO", "LIST"],
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    if out.status.success() && text.to_ascii_lowercase().contains("taskname") {
        return Ok(true);
    }
    Ok(false)
}

fn current_exe_display() -> String {
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "-".into())
}
