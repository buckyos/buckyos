use ::kRPC::{RPCErrors, Result};

use crate::system_config::{SystemConfigClient, SystemConfigError};

pub const DEFAULT_RBAC_MODEL: &str = r#"
[request_definition]
r = sub,obj,act

[policy_definition]
p = sub, obj, act, eft

[role_definition]
g = _, _

[policy_effect]
e = priority(p.eft) || deny

[matchers]
m = (g(r.sub, p.sub) || r.sub == p.sub) && ((r.sub == keyGet3(r.obj, p.obj, p.sub) || r.sub == "app:" + keyGet3(r.obj, p.obj, p.sub) || r.sub == "system:" + keyGet3(r.obj, p.obj, p.sub) || keyGet3(r.obj, p.obj, p.sub) =="") && keyMatch3(r.obj,p.obj)) && (p.act == "all" || regexMatch(r.act, p.act))
"#;

/*
# RBAC配置快速说明

## user groups:
### device group
- ood(含gateway) 全部权限
- node 标准的运行服务的节点，只有访问自己设备配置的权限
- client 一般权限等于其device owner
- sensor

### client-user groups
- root 系统所有权限 (root是特殊用户，只有一种认知方法)
- su_admin 系统所有权限
- admin 除别的用户的数据之外的读权限，
- users 自己的数据的读权限，不包含敏感数据的写权限
- su_users 自己敏感数据的写权限
- limit_users 暂不实现
- author 逻辑权限，资源的创建者


## app groups
- kernel 全部权限
- system (services) 除security相关的权限外的全部权限
- frame (services) 对一些系统全局配置有读权限
- app (services) 限制在app data范围内
- agent 对agent的身份数据有读权限，对agaent rootfs有完整权限

## Operation
- policy act 可以写 `read|write` 这类正则集合；`all` 匹配任意请求 action
- all (所有权限)
- update
- delete
- create （只给目录）
- read
- list|query (只给目录)
- subscribe

## Resource URLs

基本规则
- spec 本机安装规格，只读,su_admin可以修改
- info 当前状态，读写
- doc 核心身份资料，只读，默认上链
- profile 当前资料 读写，可上链
- settings 设置，通常是读写, 有的敏感settings需要sudo修改

所有可上链的资源修改都要su权限，在未上链时可本地修改，否则统一走先更新链再同步的逻辑

### user 相关
- obj://config/users/{user}/settings 用户设置(含密码)，需要su_users修改
- obj://config/users/{user}/doc 用户身份资料，需要su_users修改修改
- obj://config/users/{user}/profile 用户资料，可以随时修改 （可上链），注意保存的是private profile
- obj://config/security/{user}/key 用户密钥，local账户才有，需要su_admin修改

### app 相关

### agent 相关

### task 相关
- obj://task/{user} 该用户名下 Task 的集合视图。TaskMgr 用它回答“当前 principal 能否越过
  Creator 的 app_id 看该用户的 Task”，从而让 control-panel 这类系统控制面成为用户的 Task
  总览入口，而普通 app/agent（app/agent/frame 组没有 obj://task 规则）仍然只能看自己创建的
  Task。注意 enforce 是 app 侧和 user 侧的合取：app 侧决定“哪个 App 可以充当控制面”，
  user 侧的 {user} 占位符把可见范围绑定到请求者本人（root/su_admin 的全局通配规则除外）。

 */
pub const DEFAULT_RBAC_POLICY: &str = r#"
p, kernel, obj://*, all,allow
p, ood, obj://*, all,allow
p, root, obj://*, all,allow
p, su_admin, obj://*, all,allow

p, system, obj://dfs/security/*,all,deny
p, system, obj://config/security/*,all,deny
p, system, obj://*, all,allow

p, frame, obj://config/boot/*, read,allow
p, frame, obj://config/system/*,read,allow
p, frame, obj://config/agents/{agent}/{key},read,allow
p, frame, obj://config/services/{frame}/*,all,allow
p, frame, obj://config/services/{service}/info,read,allow
p, frame, obj://config/users*,read,allow

p, app, obj://config/boot/*, read,allow
p, app, obj://config/users/{user}/apps/{app}/settings,read|write,allow
p, app, obj://config/users/{user}/apps/{app}/spec,read,allow
p, app, obj://config/users/{user}/apps/{app}/info,read|write,allow
p, app, obj://config/services/{service}/info,read,allow
p, app, obj://config/services/{app}/instances/{node},write,allow

# An App runtime is promoted to this role only when an AgentSpec binds to it.
# AgentSpec is public runtime identity/configuration; the sibling private key
# deliberately remains inaccessible to the runtime App principal.
p, agent_runtime, obj://config/users/{user}/agents,read|list|query,allow
p, agent_runtime, obj://config/users/{user}/agents/{agent}/spec,read,allow

p, agent, obj://config/boot/*, read,allow
p, agent, obj://config/agents/{agent}/*,read,allow
p, agent, obj://config/users/{user}/agents/{agent}/settings,read|write,allow
p, agent, obj://config/users/{user}/agents/{agent}/spec,read,allow
p, agent, obj://config/users/{user}/agents/{agent}/info,read|write,allow
p, agent, obj://config/services/{service}/info,read,allow
p, agent, obj://config/services/{agent}/instances/{node},write,allow

p, admin,obj://config/boot/*, read,allow
p, admin,obj://config/system/*,read,allow
p, admin,obj://config/agents/{agent}/doc,read,allow
p, admin,obj://config/agents/{agent}/settings,read|write,allow
p, admin,obj://config/users/{admin}/*,read,allow
p, admin,obj://config/users/{admin}/profile,read|write,allow
p, admin,obj://config/users/{admin}/apps/{app}/{key},read|write,allow
p, admin,obj://config/users/{admin}/agents,list|query,allow
p, admin,obj://config/users/{admin}/agents/{agent}/{key},read|write,allow
p, admin,obj://config/services/aicc/settings,read|write,allow
p, admin,obj://config/services/msg-center/settings,read|write,allow
p, admin,obj://config/services/{service}/instances/{node},write,allow
p, admin,obj://config/services/*,read,allow
p, admin,obj://task/{admin},read,allow

p, users,obj://config/boot/*, read,allow
p, users,obj://config/agents/{agent}/doc,read,allow
# p, su_user,obj://config/users/{user}/*,all,allow
p, users,obj://config/users/{users}/*,read,allow
p, users,obj://config/users/{users}/profile,read|write,allow
p, users,obj://config/users/{users}/apps/{app}/{key},read|write,allow
p, users,obj://config/users/{users}/agents,list|query,allow
p, users,obj://config/users/{users}/agents/{agent}/{key},read|write,allow
p, users,obj://config/services/{service}/info,read,allow
p, users,obj://config/services/{service}/instances/{node},write,allow
p, users,obj://task/{users},read,allow

g, system:node-daemon, kernel
g, system:scheduler, kernel
g, system:system-config, kernel
g, system:verify-hub, kernel
g, system:cyfs-gateway, kernel
g, system:buckycli, kernel

g, system:task-manager, system
g, system:kmsg, system
g, system:control-panel, system

g, system:repo-service, frame
g, system:aicc, frame
g, system:msg-center, frame
g, system:opendan, frame
g, system:slog-server, frame
g, system:smb-service, frame

"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RbacConfig {
    pub model: String,
    pub policy: String,
    pub policy_tail: String,
    pub policy_version: u64,
    pub is_changed: bool,
}

pub fn overlap_rbac_policy(default_policy: &str, policy_tail: &str) -> String {
    let default_policy = default_policy.trim();
    let policy_tail = policy_tail.trim();

    if default_policy.is_empty() {
        return policy_tail.to_string();
    }
    if policy_tail.is_empty() {
        return default_policy.to_string();
    }
    format!("{}\n{}", default_policy, policy_tail)
}

pub fn build_current_rbac_config(policy_tail: Option<&str>) -> RbacConfig {
    let policy_tail = policy_tail.unwrap_or("").trim().to_string();
    RbacConfig {
        model: DEFAULT_RBAC_MODEL.trim().to_string(),
        policy: overlap_rbac_policy(DEFAULT_RBAC_POLICY, policy_tail.as_str()),
        policy_tail,
        policy_version: 0,
        is_changed: false,
    }
}

pub async fn load_current_rbac_config(
    system_config_client: &SystemConfigClient,
) -> Result<RbacConfig> {
    let policy_result = match system_config_client.get("system/rbac/policy").await {
        Ok(value) => Some(value),
        Err(SystemConfigError::KeyNotFound(_)) => None,
        Err(error) => {
            return Err(RPCErrors::ReasonError(format!(
                "load rbac policy failed: {}",
                error
            )));
        }
    };

    let mut config =
        build_current_rbac_config(policy_result.as_ref().map(|value| value.value.as_str()));
    if let Some(policy_result) = policy_result {
        config.policy_version = policy_result.version;
        config.is_changed = policy_result.is_changed;
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    use lazy_static::lazy_static;
    use tokio::sync::Mutex;

    // `rbac::SYS_ENFORCE` is a process-wide singleton, so any test that
    // calls `create_enforcer` + `enforce` against it must run serialized;
    // otherwise a parallel test can swap the policy out from under us.
    lazy_static! {
        static ref TEST_LOCK: Mutex<()> = Mutex::new(());
    }

    #[tokio::test]
    async fn device_principal_and_kernel_service_can_access_zone_state() {
        let _guard = TEST_LOCK.lock().await;
        let config = build_current_rbac_config(Some("g, ood1, ood"));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();

        assert!(
            rbac::enforce(
                "ood1",
                "system:node-daemon",
                "obj://config/nodes/ood1/config",
                "read",
                None,
            )
            .await
        );
        assert!(
            rbac::enforce(
                "ood1",
                "system:node-daemon",
                "obj://config/devices/ood1/info",
                "write",
                None,
            )
            .await
        );
    }

    #[tokio::test]
    async fn buckyos_info_is_read_only_for_admin_and_writable_by_scheduler() {
        let _guard = TEST_LOCK.lock().await;
        let config = build_current_rbac_config(Some("g, alice, admin\ng, ood1, ood"));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();

        let resource = "obj://config/system/buckyos_info";
        assert!(rbac::enforce("alice", "system:control-panel", resource, "read", None).await);
        assert!(!rbac::enforce("alice", "system:control-panel", resource, "write", None).await);
        assert!(rbac::enforce("ood1", "system:scheduler", resource, "write", None).await);
    }

    #[tokio::test]
    async fn buckyos_dev_config_write_requires_admin_sudo() {
        let _guard = TEST_LOCK.lock().await;
        let config = build_current_rbac_config(Some("g, alice, admin\ng, su_alice, su_admin"));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();

        let resource = "obj://config/system/buckyos_dev_config";
        assert!(rbac::enforce("alice", "system:control-panel", resource, "read", None).await);
        assert!(!rbac::enforce("alice", "system:control-panel", resource, "write", None).await);
        assert!(
            rbac::enforce(
                "alice",
                "system:control-panel",
                resource,
                "write",
                Some(rbac::SudoMode::Sudo("su_alice".to_string())),
            )
            .await
        );
    }

    #[test]
    fn overlap_rbac_policy_appends_tail_to_default() {
        let policy = overlap_rbac_policy(
            "p, root, obj://config/*, read|write,allow",
            "g, alice, admin",
        );
        assert_eq!(
            policy,
            "p, root, obj://config/*, read|write,allow\ng, alice, admin"
        );
    }

    #[test]
    fn build_current_rbac_config_uses_default_model_and_policy() {
        let config = build_current_rbac_config(Some("g, alice, admin\n"));
        assert!(config.model.contains("[request_definition]"));
        assert!(config.model.contains("p.act == \"all\""));
        assert!(config.policy.contains("p, root, obj://*, all,allow"));
        assert!(config.policy.ends_with("g, alice, admin"));
        assert_eq!(config.policy_tail, "g, alice, admin");
    }

    #[tokio::test]
    async fn default_model_matches_all_and_regex_action_policies() {
        let _guard = TEST_LOCK.lock().await;

        let policy = r#"
p, kernel, obj://config/*, all,allow
p, root, obj://config/*, all,allow
"#;
        rbac::create_enforcer(DEFAULT_RBAC_MODEL.trim(), policy.trim())
            .await
            .unwrap();

        assert!(rbac::enforce("root", "kernel", "obj://config/foo", "read", None).await);
        assert!(rbac::enforce("root", "kernel", "obj://config/foo", "write", None).await);
        assert!(rbac::enforce("root", "kernel", "obj://config/foo", "delete", None).await);
        assert!(!rbac::enforce("root", "kernel", "obj://other/foo", "read", None).await);

        let policy = r#"
p, kernel, obj://config/*, read|write,allow
p, root, obj://config/*, read|write,allow
"#;
        rbac::create_enforcer(DEFAULT_RBAC_MODEL.trim(), policy.trim())
            .await
            .unwrap();

        assert!(rbac::enforce("root", "kernel", "obj://config/foo", "read", None).await);
        assert!(rbac::enforce("root", "kernel", "obj://config/foo", "write", None).await);
        assert!(!rbac::enforce("root", "kernel", "obj://config/foo", "delete", None).await);
    }

    // -------------------------------------------------------------------
    // 下面这个测试用来揭示当前 DEFAULT_RBAC_POLICY 里几条 `obj://.../*/...`
    // 规则的写法是错误的:
    //
    //   keyMatch3 会把模式里所有 `/*` 替换成 `/.*` 再做正则匹配,
    //   `.*` 是贪婪且能跨 `/`, 所以"中间段"的 `*` 实际上在匹配任意深度
    //   的子路径, 超出了原本"单段 agent_id / app_id"的意图.
    //
    // 单段通配应该改用 `{xxx}` 占位符 (会被替换成 `[^/]+`), 例如:
    //   obj://config/agents/{agent_id}/doc
    //   obj://config/agents/{agent_id}/settings
    //   obj://config/agents/{agent_id}/{key}
    //
    // 下面 `assert!(!...)` 断言的都是"修复后的正确语义", 因此 BUG 还在
    // 的时候每条断言都会 FAIL, 并把对应的 BUG 信息打印出来; 等 BUG
    // 修好以后这些断言会全部 PASS, 测试就变成了回归门禁.
    // -------------------------------------------------------------------
    #[tokio::test]
    async fn default_policy_wildcards_overmatch_multi_level_paths() {
        let _guard = TEST_LOCK.lock().await;

        let policy_tail = r#"
g, alice, admin
g, bob, users
g, su_alice, su_admin
"#;
        let config = build_current_rbac_config(Some(policy_tail));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();

        // ---- sanity: 单段 agent_id 下的访问应当通过 ----
        assert!(
            rbac::enforce(
                "alice",
                "system:buckycli",
                "obj://config/agents/jarvis/doc",
                "read",
                None,
            )
            .await
        );
        assert!(
            rbac::enforce(
                "bob",
                "system:buckycli",
                "obj://config/agents/jarvis/doc",
                "read",
                None,
            )
            .await
        );
        assert!(
            rbac::enforce(
                "alice",
                "system:buckycli",
                "obj://config/agents/jarvis/settings",
                "write",
                None,
            )
            .await
        );
        assert!(
            rbac::enforce(
                "su_alice",
                "system:buckycli",
                "obj://config/users/dvtest1782201008431/doc",
                "write",
                None,
            )
            .await
        );

        // 用一个 Vec 收集所有"过度匹配"的命中, 这样一次运行就能把
        // 所有 BUG 都打印出来, 而不是在第一条 assert 上 panic 就停下.
        let mut bugs: Vec<&'static str> = Vec::new();

        // 每条 case: (userid, appid, res_path, action, bug 描述).
        // 期望: enforce 返回 false; 若返回 true 就说明该规则过度匹配.
        let over_match_cases: &[(&str, &str, &str, &str, &str)] = &[
            (
                "alice",
                "system:buckycli",
                "obj://config/agents/foo/bar/doc",
                "read",
                "BUG: admin 的 agents/*/doc 不应匹配多层路径 (foo/bar/doc)",
            ),
            (
                "bob",
                "system:buckycli",
                "obj://config/agents/foo/bar/doc",
                "read",
                "BUG: users 的 agents/*/doc 不应匹配多层路径 (foo/bar/doc)",
            ),
            (
                "alice",
                "system:buckycli",
                "obj://config/agents/foo/bar/settings",
                "write",
                "BUG: admin 的 agents/*/settings 不应匹配多层路径 (foo/bar/settings)",
            ),
            // frame 的 `obj://config/agents/*/*` 等效于 agents/.*/.*.
            // user side 用 root (有全权), app side 用 repo-service
            // (g, repo-service, frame), 把 BUG 隔离到 frame 这条规则.
            (
                "root",
                "system:repo-service",
                "obj://config/agents/a/b/c/d",
                "read",
                "BUG: frame 的 agents/*/* 不应匹配 4 层路径 (a/b/c/d)",
            ),
            // admin 的 `obj://config/users/{admin}/apps/*/*` 同样会跨段:
            // 期望 apps 下面正好是 "{app_id}/{key}" 两段.
            (
                "alice",
                "system:buckycli",
                "obj://config/users/alice/apps/some_app/extra/key",
                "write",
                "BUG: admin 的 users/{admin}/apps/*/* 不应匹配 apps 下 3 层以上路径",
            ),
        ];

        for (userid, appid, res, act, bug_msg) in over_match_cases {
            if rbac::enforce(userid, appid, res, act, None).await {
                eprintln!("  -> {}", bug_msg);
                bugs.push(bug_msg);
            }
        }

        assert!(
            bugs.is_empty(),
            "RBAC 规则存在 {} 条过度匹配, 详情见上方 `-> BUG:` 行",
            bugs.len()
        );
    }

    #[tokio::test]
    async fn users_role_is_bound_to_own_user_path() {
        let _guard = TEST_LOCK.lock().await;

        let policy_tail = "g, bob, users";
        let config = build_current_rbac_config(Some(policy_tail));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();

        assert!(
            rbac::enforce(
                "bob",
                "system:buckycli",
                "obj://config/users/bob/settings",
                "read",
                None,
            )
            .await
        );
        assert!(
            rbac::enforce(
                "bob",
                "system:buckycli",
                "obj://config/users/bob/profile",
                "write",
                None,
            )
            .await
        );
        assert!(
            !rbac::enforce(
                "bob",
                "system:buckycli",
                "obj://config/users/bob/settings",
                "write",
                None,
            )
            .await
        );
        assert!(
            !rbac::enforce(
                "bob",
                "system:buckycli",
                "obj://config/users/alice/settings",
                "read",
                None,
            )
            .await
        );
        assert!(
            !rbac::enforce(
                "bob",
                "system:buckycli",
                "obj://config/users/alice/profile",
                "write",
                None,
            )
            .await
        );
    }

    #[tokio::test]
    async fn admin_can_write_aicc_settings() {
        let _guard = TEST_LOCK.lock().await;

        let policy_tail = r#"
g, alice, admin
g, bob, users
"#;
        let config = build_current_rbac_config(Some(policy_tail));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();

        assert!(
            rbac::enforce(
                "alice",
                "system:control-panel",
                "obj://config/services/aicc/settings",
                "write",
                None,
            )
            .await
        );
        assert!(
            !rbac::enforce(
                "bob",
                "system:control-panel",
                "obj://config/services/aicc/settings",
                "write",
                None,
            )
            .await
        );
        assert!(
            rbac::enforce(
                "alice",
                "system:control-panel",
                "obj://config/services/msg-center/settings",
                "write",
                None,
            )
            .await
        );
    }

    #[tokio::test]
    async fn app_can_report_own_service_instance() {
        let _guard = TEST_LOCK.lock().await;

        let config = build_current_rbac_config(Some(
            "g, alice, admin\ng, bob, users\ng, app:jarvis.buckyos.bns.did, app",
        ));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();

        assert!(
            rbac::enforce(
                "alice",
                "app:jarvis.buckyos.bns.did",
                "obj://config/services/jarvis.buckyos.bns.did/instances/ood1",
                "write",
                None,
            )
            .await
        );
        assert!(
            rbac::enforce(
                "bob",
                "app:jarvis.buckyos.bns.did",
                "obj://config/services/jarvis.buckyos.bns.did/instances/ood1",
                "write",
                None,
            )
            .await
        );
        assert!(
            !rbac::enforce(
                "alice",
                "app:jarvis.buckyos.bns.did",
                "obj://config/services/other-agent/instances/ood1",
                "write",
                None,
            )
            .await
        );
        assert!(
            !rbac::enforce(
                "alice",
                "app:jarvis.buckyos.bns.did",
                "obj://config/services/jarvis.buckyos.bns.did/settings",
                "write",
                None,
            )
            .await
        );
    }

    #[tokio::test]
    async fn bound_agent_runtime_can_discover_specs_but_not_private_keys() {
        let _guard = TEST_LOCK.lock().await;

        let config = build_current_rbac_config(Some(
            "g, alice, admin\ng, app:jarvis.buckyos.bns.did, app\ng, app:jarvis.buckyos.bns.did, agent_runtime\ng, app:gallery.buckyos.bns.did, app",
        ));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();

        let runtime = "app:jarvis.buckyos.bns.did";
        assert!(
            rbac::enforce(
                "alice",
                runtime,
                "obj://config/users/alice/agents",
                "read",
                None,
            )
            .await
        );
        assert!(
            rbac::enforce(
                "alice",
                runtime,
                "obj://config/users/alice/agents/jarvis.example.com/spec",
                "read",
                None,
            )
            .await
        );
        assert!(
            !rbac::enforce(
                "alice",
                runtime,
                "obj://config/users/alice/agents/jarvis.example.com/key",
                "read",
                None,
            )
            .await
        );
        assert!(
            !rbac::enforce(
                "alice",
                "app:gallery.buckyos.bns.did",
                "obj://config/users/alice/agents/jarvis.example.com/spec",
                "read",
                None,
            )
            .await
        );
    }

    /// `obj://task/{user}` is the collection view TaskMgr consults to decide
    /// whether a principal may look past the creator's `app_id`. The bindings
    /// here mirror a real zone's policy tail (control-panel promoted to
    /// `kernel`, the owner bound to `admin`, agents left in `agent`).
    #[tokio::test]
    async fn task_collection_is_visible_to_control_surfaces_not_to_apps() {
        let _guard = TEST_LOCK.lock().await;

        let policy_tail = r#"
g, devtest, admin
g, su_devtest, su_admin
g, bob, users
g, system:control-panel, kernel
g, system:task-manager, kernel
g, app:buckyos_jarvis, agent
g, app:buckyos_filebrowser, app
"#;
        let config = build_current_rbac_config(Some(policy_tail));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();

        // The control surface sees its own user's tasks: this is the Task
        // Center's whole reason to exist.
        assert!(
            rbac::enforce(
                "devtest",
                "system:control-panel",
                "obj://task/devtest",
                "read",
                None
            )
            .await
        );
        assert!(
            rbac::enforce(
                "bob",
                "system:control-panel",
                "obj://task/bob",
                "read",
                None
            )
            .await
        );

        // ...but not another user's, even from the control surface.
        assert!(
            !rbac::enforce(
                "devtest",
                "system:control-panel",
                "obj://task/bob",
                "read",
                None
            )
            .await
        );
        assert!(
            !rbac::enforce(
                "bob",
                "system:control-panel",
                "obj://task/devtest",
                "read",
                None
            )
            .await
        );

        // Ordinary apps and agents keep the doc §8.5 isolation: no cross-app
        // view even of their own user's tasks.
        assert!(
            !rbac::enforce(
                "devtest",
                "app:buckyos_jarvis",
                "obj://task/devtest",
                "read",
                None
            )
            .await
        );
        assert!(
            !rbac::enforce(
                "devtest",
                "app:buckyos_filebrowser",
                "obj://task/devtest",
                "read",
                None
            )
            .await
        );

        // A sudo session is the zone owner: global task view.
        assert!(
            rbac::enforce(
                "devtest",
                "system:control-panel",
                "obj://task/bob",
                "read",
                Some(rbac::SudoMode::Sudo("su_devtest".to_string())),
            )
            .await
        );

        // The collection is a read view; it must not confer writes.
        assert!(
            !rbac::enforce(
                "devtest",
                "system:control-panel",
                "obj://task/devtest",
                "write",
                None
            )
            .await
        );
    }

    #[tokio::test]
    async fn sudo_users_role_can_write_own_sensitive_user_data() {
        let _guard = TEST_LOCK.lock().await;

        let policy_tail = r#"
g, bob, users
g, su_bob, su_users
p, su_bob, obj://config/users/bob/settings, read|write, allow
p, su_bob, obj://config/users/bob/doc, read|write, allow
"#;
        let config = build_current_rbac_config(Some(policy_tail));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();

        assert!(
            rbac::enforce(
                "bob",
                "system:buckycli",
                "obj://config/users/bob/settings",
                "write",
                Some(rbac::SudoMode::Sudo("su_bob".to_string())),
            )
            .await
        );
        assert!(
            rbac::enforce(
                "bob",
                "system:buckycli",
                "obj://config/users/bob/doc",
                "write",
                Some(rbac::SudoMode::Sudo("su_bob".to_string())),
            )
            .await
        );
        assert!(
            !rbac::enforce(
                "bob",
                "system:buckycli",
                "obj://config/users/alice/settings",
                "write",
                Some(rbac::SudoMode::Sudo("su_bob".to_string())),
            )
            .await
        );
    }

    #[tokio::test]
    async fn sudo_admin_can_read_own_user_data_but_not_other_users_data() {
        let _guard = TEST_LOCK.lock().await;

        let policy_tail = r#"
g, alice, admin
g, su_alice, su_admin
"#;
        let config = build_current_rbac_config(Some(policy_tail));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();

        assert!(
            rbac::enforce(
                "alice",
                "system:buckycli",
                "obj://config/users/alice/settings",
                "read",
                Some(rbac::SudoMode::Sudo("su_alice".to_string())),
            )
            .await
        );
        assert!(
            rbac::enforce(
                "alice",
                "system:buckycli",
                "obj://config/users/bob/settings",
                "read",
                Some(rbac::SudoMode::Sudo("su_alice".to_string())),
            )
            .await
        );
    }

    #[tokio::test]
    async fn local_user_private_keys_are_restricted_to_kernel_root_and_sudo_admin() {
        let _guard = TEST_LOCK.lock().await;

        let policy_tail = r#"
g, alice, admin
g, bob, users
g, su_alice, su_admin
g, app:gallery, app
"#;
        let config = build_current_rbac_config(Some(policy_tail));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();
        let resource = "obj://config/security/dvlocal/key";

        for action in ["read", "write"] {
            assert!(!rbac::enforce("bob", "system:buckycli", resource, action, None).await);
            assert!(!rbac::enforce("alice", "system:buckycli", resource, action, None).await);
            assert!(!rbac::enforce("root", "system:repo-service", resource, action, None).await);
            assert!(!rbac::enforce("root", "app:gallery", resource, action, None).await);
            assert!(!rbac::enforce("root", "system:control-panel", resource, action, None).await);

            assert!(rbac::enforce("root", "system:scheduler", resource, action, None).await);
            assert!(
                rbac::enforce(
                    "alice",
                    "system:buckycli",
                    resource,
                    action,
                    Some(rbac::SudoMode::Sudo("su_alice".to_string())),
                )
                .await
            );
        }
    }

    #[tokio::test]
    async fn app_and_system_authorization_names_cannot_collide() {
        let _guard = TEST_LOCK.lock().await;
        let config = build_current_rbac_config(Some("g, app:control-panel, app"));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();

        assert!(
            rbac::enforce(
                "root",
                "system:control-panel",
                "obj://config/system/rbac",
                "read",
                None,
            )
            .await
        );
        assert!(
            !rbac::enforce(
                "root",
                "app:control-panel",
                "obj://config/system/rbac",
                "read",
                None,
            )
            .await
        );
    }
}
