use buckyos_api::node_control::{lookup_os_user, OsUser};
use buckyos_api::{atomic_write, ComputerAccess, ExecutorRunAs, OwnerAgentsPolicy};
use buckyos_kit::get_buckyos_system_etc_dir;
use log::error;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const HOST_ACCOUNT_FILE: &str = "host_account.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostAccountFile {
    account: String,
}

fn node_daemon_is_linux_root() -> bool {
    #[cfg(target_os = "linux")]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn host_account_path() -> PathBuf {
    get_buckyos_system_etc_dir().join(HOST_ACCOUNT_FILE)
}

fn login_accounts(passwd: &str) -> Vec<String> {
    passwd
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let name = fields.first()?;
            let uid = fields.get(2)?.parse::<u32>().ok()?;
            let shell = fields.get(6)?;
            let can_log_in =
                !shell.is_empty() && !shell.ends_with("/nologin") && !shell.ends_with("/false");
            let is_person_or_root = uid == 0 || (1000..65534).contains(&uid);
            (can_log_in && is_person_or_root).then(|| name.to_string())
        })
        .collect()
}

pub(crate) fn available_accounts() -> Vec<String> {
    if !node_daemon_is_linux_root() {
        return Vec::new();
    }
    std::fs::read_to_string("/etc/passwd")
        .map(|passwd| login_accounts(&passwd))
        .unwrap_or_default()
}

pub(crate) fn save(account: &str) -> Result<(), String> {
    let path = host_account_path();
    if account.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("remove {} failed: {error}", path.display()))
            }
            _ => Ok(()),
        };
    }
    let body = serde_json::to_vec_pretty(&HostAccountFile {
        account: account.to_string(),
    })
    .map_err(|error| error.to_string())?;
    atomic_write(&path, &body)
}

pub(crate) fn load() -> Option<OsUser> {
    if !node_daemon_is_linux_root() {
        return None;
    }
    let path = host_account_path();
    let raw = std::fs::read(&path).ok()?;
    let file: HostAccountFile = match serde_json::from_slice(&raw) {
        Ok(file) => file,
        Err(error) => {
            error!("invalid {}: {error}", path.display());
            return None;
        }
    };
    let account = lookup_os_user(&file.account);
    if account.is_none() {
        error!(
            "{} names an unknown OS account: {}",
            path.display(),
            file.account
        );
    }
    account
}

pub(crate) fn runs_as_node_daemon(account: Option<&OsUser>) -> bool {
    account.is_some_and(|account| account.uid == 0)
}

pub(crate) fn owner_agents_policy(account: &OsUser) -> OwnerAgentsPolicy {
    let home = account.home.to_string_lossy().into_owned();
    let is_root = runs_as_node_daemon(Some(account));
    OwnerAgentsPolicy {
        baseline: None,
        enabled: true,
        workdir_roots: if is_root {
            vec![home, "/".to_string()]
        } else {
            vec![home]
        },
        env: Default::default(),
        ops: Vec::new(),
        computer: ComputerAccess::None,
        run_as: (!is_root).then_some(ExecutorRunAs {
            uid: account.uid,
            gid: account.gid,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::ExecutorSettings;

    fn account(uid: u32, gid: u32, home: &str) -> OsUser {
        OsUser {
            name: "someone".to_string(),
            uid,
            gid,
            home: PathBuf::from(home),
        }
    }

    fn settings_with(policy: OwnerAgentsPolicy) -> ExecutorSettings {
        ExecutorSettings {
            enabled: true,
            owner_agents: Some(policy),
            ..Default::default()
        }
    }

    #[test]
    fn only_root_and_people_who_can_log_in_are_offered() {
        let passwd = "\
root:x:0:0:root:/root:/bin/bash
daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin
sync:x:4:65534:sync:/bin:/bin/sync
_apt:x:100:65534::/nonexistent:/usr/sbin/nologin
lzc:x:1000:1000::/home/lzc:/bin/bash
locked:x:1001:1001::/home/locked:/usr/sbin/nologin
frozen:x:1002:1002::/home/frozen:/bin/false
noshell:x:1003:1003::/home/noshell:
nobody:x:65534:65534:nobody:/nonexistent:/bin/bash
broken line
";
        assert_eq!(login_accounts(passwd), vec!["root", "lzc"]);
    }

    #[test]
    fn root_runs_as_the_daemon_and_may_work_anywhere() {
        let root = account(0, 0, "/root");
        let policy = owner_agents_policy(&root);
        assert!(runs_as_node_daemon(Some(&root)));
        assert_eq!(policy.run_as, None);
        assert_eq!(policy.workdir_roots, vec!["/root", "/"]);
        assert_eq!(policy.computer, ComputerAccess::None);
        settings_with(policy).validate().unwrap();
    }

    #[test]
    fn a_normal_account_is_switched_to_and_starts_in_its_home() {
        let lzc = account(1000, 1000, "/home/lzc");
        let policy = owner_agents_policy(&lzc);
        assert!(!runs_as_node_daemon(Some(&lzc)));
        let run_as = policy.run_as.clone().unwrap();
        assert_eq!((run_as.uid, run_as.gid), (1000, 1000));
        assert_eq!(policy.workdir_roots, vec!["/home/lzc"]);
        settings_with(policy).validate().unwrap();
    }

    #[test]
    fn no_chosen_account_leaves_the_node_switching_users() {
        assert!(!runs_as_node_daemon(None));
    }
}
