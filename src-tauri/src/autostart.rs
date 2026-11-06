//! edgePass "autostart": Scheduled Task at user logon (not a Windows Service).
//!
//! GUI apps cannot live in Session 0. We wait for the OpenBao service, then
//! launch the widget once — same contract as `scripts/install-autostart.ps1`.

use serde::Serialize;
use std::fs;

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

    let data = dirs::data_dir()
        .ok_or_else(|| "no data dir".to_string())?
        .join("edgePass");
    fs::create_dir_all(&data).map_err(|e| e.to_string())?;
    let wrapper = data.join("start-with-openbao.ps1");

    let exe_ps = exe_s.replace('\'', "''");
    let script = format!(
        r#"$ErrorActionPreference = "SilentlyContinue"
$log = Join-Path $env:LOCALAPPDATA "edgePass\autostart.log"
function Write-Log($m) {{ "$(Get-Date -Format s) $m" | Out-File -FilePath $log -Append -Encoding UTF8 }}

for ($i = 0; $i -lt 30; $i++) {{
  $svc = Get-Service -Name OpenBao -ErrorAction SilentlyContinue
  if ($svc -and $svc.Status -eq "Running") {{ break }}
  Start-Sleep -Seconds 2
}}
Start-Sleep -Seconds 2

if (Get-Process -Name "edgepass" -ErrorAction SilentlyContinue) {{
  Write-Log "edgePass ya estaba en ejecucion; no se lanza otra instancia"
  exit 0
}}
Write-Log "lanzando edgePass"
Start-Process -FilePath '{exe_ps}'
"#
    );
    fs::write(&wrapper, script).map_err(|e| format!("write wrapper: {e}"))?;

    let wrapper_s = wrapper.display().to_string().replace('\'', "''");
    let ps = format!(
        r#"$ErrorActionPreference = 'Stop'
$arg = '-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "{wrapper_s}"'
$action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $arg
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero)
Register-ScheduledTask -TaskName '{TASK_NAME}' -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null
'OK'
"#
    );

    let out = run_hidden(
        "powershell.exe",
        &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps],
    )?;
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
        message: format!("OK · arranca al logon (espera OpenBao) · {TASK_NAME}"),
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
