use crate::Mode;
use anyhow::{bail, ensure, Context, Result};
use buckyos_api::{
    check_app_sdk, generate_node_daemon_login_token, load_local_device_config,
    load_local_device_private_key, load_local_node_identity_config, AppServiceSpec,
    SystemConfigClient, MINIMUM_APP_SDK,
};
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::time::Duration;

async fn conflicts(root: &Path, minimum: &str) -> Result<Vec<String>> {
    check_app_sdk(None, Some(minimum)).map_err(anyhow::Error::msg)?;
    let identity = load_local_node_identity_config(&root.join("etc/node_identity.json"))
        .map_err(anyhow::Error::msg)?;
    let (_, device) = load_local_device_config(&identity, true).map_err(anyhow::Error::msg)?;
    let key = load_local_device_private_key(&identity.device_did).map_err(anyhow::Error::msg)?;
    let token = generate_node_daemon_login_token(&device, &key).map_err(anyhow::Error::msg)?;
    let client = SystemConfigClient::new(None, Some(&token));
    let mut issues = Vec::new();
    for user in client.list("users").await? {
        let apps = format!("users/{user}/apps");
        for app in client.list(&apps).await? {
            let prefix = format!("{apps}/{app}");
            let children = client.list(&prefix).await?;
            if !children.iter().any(|c| c == "spec") {
                if children.iter().any(|c| c == "install_record") {
                    let value = client.get(&format!("{prefix}/install_record")).await?;
                    let record: buckyos_api::InstallRecord = serde_json::from_str(&value.value)
                        .context(format!("cannot read installation record for {user}/{app}"))?;
                    use buckyos_api::InstallRecordState::*;
                    ensure!(
                        matches!(record.state, Failed | RolledBack),
                        "installed AppDoc missing for {user}/{app}"
                    );
                }
                continue;
            }
            let value = client.get(&format!("{prefix}/spec")).await?;
            let spec: AppServiceSpec = serde_json::from_str(&value.value)
                .context(format!("cannot read installed AppDoc for {user}/{app}"))?;
            ensure!(
                spec.owner_user_id == user && spec.app_instance_id.app_id().as_str() == app,
                "installed AppDoc identity mismatch at {prefix}"
            );
            if spec.is_installed() {
                if let Err(reason) =
                    check_app_sdk(spec.app_doc.sdk_version.as_deref(), Some(minimum))
                {
                    issues.push(format!("{user}/{app}: {reason}"));
                }
            }
        }
    }
    issues.sort();
    Ok(issues)
}

pub fn ask(message: &str) -> Result<bool> {
    eprintln!("{message}");
    if io::stdin().is_terminal() {
        eprint!("Continue the update? [y/N] ");
        io::stderr().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        return Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"));
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_YESNO,
        };
        let caption: Vec<u16> = "BuckyOS Node Updater\0".encode_utf16().collect();
        let body: Vec<u16> = format!("{message}\nContinue the update?\0")
            .encode_utf16()
            .collect();
        return Ok(unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                body.as_ptr(),
                caption.as_ptr(),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            )
        } == IDYES);
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", "on run argv", "-e", "display dialog (item 1 of argv) buttons {\"Cancel\", \"Continue\"} default button \"Cancel\" with title \"BuckyOS Node Updater\"", "-e", "end run", "--", message])
            .output()?;
        return Ok(output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("button returned:Continue"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::io::BufRead;
        let mut tty = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .map_err(|_| {
                anyhow::anyhow!("interactive SDK confirmation needs a controlling terminal")
            })?;
        write!(tty, "{message}\nContinue the update? [y/N] ")?;
        tty.flush()?;
        let mut answer = String::new();
        io::BufReader::new(tty).read_line(&mut answer)?;
        Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
    }
}

pub fn decide(
    issues: &[String],
    mode: Mode,
    confirm: impl FnOnce(&str) -> Result<bool>,
) -> Result<()> {
    if issues.is_empty() {
        return Ok(());
    }
    let message = format!(
        "The following apps cannot load after the update:\n{}",
        issues.join("\n")
    );
    if mode == Mode::Silent {
        bail!("{message}");
    }
    ensure!(
        confirm(&message)?,
        "SDK compatibility update cancelled\n{message}"
    );
    Ok(())
}

pub fn installed_conflicts(root: &Path) -> Result<Vec<String>> {
    let Some(floor) = MINIMUM_APP_SDK else {
        return Ok(Vec::new());
    };
    check_app_sdk(None, Some(floor)).map_err(anyhow::Error::msg)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(60), conflicts(root, floor)).await
    })
    .context("timed out reading installed AppDocs")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_floor_and_interaction_agree() {
        assert!(check_app_sdk(Some("0.9.0"), Some("0.10.0")).is_err());
        assert!(check_app_sdk(Some("0.10.0-beta.1"), Some("0.10.0")).is_err());
        assert!(check_app_sdk(Some("v0.10.0+dev"), Some("0.10.0")).is_ok());
        assert!(check_app_sdk(None, Some("0.10.0")).is_ok());
        assert!(check_app_sdk(Some("legacy"), None).is_ok());
        let issues = vec!["alice/demo: SDK too old".into()];
        assert!(decide(&issues, Mode::Silent, |_| panic!("silent prompted")).is_err());
        assert!(decide(&issues, Mode::Interactive, |_| Ok(false)).is_err());
        assert!(decide(&issues, Mode::Interactive, |msg| Ok(
            msg.contains("alice/demo")
        ))
        .is_ok());
    }
}
