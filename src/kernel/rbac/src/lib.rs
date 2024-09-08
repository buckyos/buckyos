
#![allow(dead_code)]
#![allow(unused)]

use std::collections::HashMap;
use std::sync::{Arc};
use log::*;
use tokio::sync::Mutex;
use casbin::{rhai::ImmutableString, CoreApi, DefaultModel, Enforcer, Filter, MemoryAdapter, MgmtApi};
use lazy_static::lazy_static;

pub const DEFAULT_MODEL: &str =  r#"
[request_definition]
r = sub,obj,act

[policy_definition]
p = sub, obj, act, eft

[role_definition]
g = _, _ # sub, role

[policy_effect]
e = priority(p.eft) || deny

[matchers]
m = (g(r.sub, p.sub) || r.sub == p.sub) && ((r.sub == keyGet3(r.obj, p.obj, p.sub) || keyGet3(r.obj, p.obj, p.sub) =="") && keyMatch3(r.obj,p.obj)) && regexMatch(r.act, p.act)

"#;

pub const DEFAULT_POLICY: &str = r#"

p, kernel, kv://*, read|write,allow
p, kernel, dfs://*, read|write,allow

p, owner, kv://*, read|write,allow
p, owner, dfs://*, read|write,allow


p, user, kv://*, read,allow
p, user, dfs://public/*,read|write,allow
p, user, dfs://homes/{user}/*, read|write,allow
p, app,  dfs://homes/*/apps/{app}/*, read|write,allow

p, limit, dfs://public/*, read,allow
p, guest, dfs://public/*, read,allow

g, alice, user
g, bob, user
g, app1, app
g, app2, app
"#;

pub const DEFAULT_POLICY2: &str = r#"
p, owner, kv://.+$, read|write,allow
p, owner, dfs://.+$, read|write,allow
p, owner, fs://[^/]+/.+$, read|write,allow

p, user, ^kv://.+$, read,allow
p, user, ^dfs://public/.+$,read|write,allow

p, app1, ^dfs://homes/[^/]+/apps/app1/[^/]+, read|write,allow

p, alice, ^dfs://homes/alice/.+$, read|write,allow
p, alice, ^kv://users/alice/.+$, read|write,allow

p, limit, dfs://public/[^/]+, read,allow
p, guest, dfs://public/[^/]+, read,allow

g, alice, user
g, bob, user
g, app1, app
g, app2, app
"#;

lazy_static!{
    static ref SYS_ENFORCE: Arc<Mutex<Option<Enforcer> > > = {
        Arc::new(Mutex::new(None))
    };
}
pub async fn create_enforcer(model_str:Option<&str>,policy_str:Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let model_str = model_str.unwrap_or(DEFAULT_MODEL);
    let policy_str = policy_str.unwrap_or(DEFAULT_POLICY);

    let m = DefaultModel::from_str(model_str).await?;
    let mut e = Enforcer::new(m, MemoryAdapter::default()).await?;
    for line in policy_str.lines() {
        let line = line.trim();
        if !line.is_empty() && !line.starts_with('#') {
            let rule: Vec<String> = line.split(',').map(|s| s.trim().to_string()).collect();
            if rule[0] == "p" {
                e.add_policy(rule[1..].to_vec()).await?;
            } else if rule[0] == "g" {
                e.add_grouping_policy(rule[1..].to_vec()).await?;
            }
        }
    }

    let mut enforcer = SYS_ENFORCE.lock().await;
    *enforcer = Some(e);
    Ok(())
}

//use default RBAC config to enforce the access control
//default acl config is stored in the memory,so it is not async function
//TODO :use system_config event to reload the config.
pub async fn enforce(userid:&str, appid:Option<&str>,res_path:&str,op_name:&str) -> bool {
    let enforcer = SYS_ENFORCE.lock().await;
    if enforcer.is_none() {
        error!("enforcer is not initialized");
        return false;
    }
    let enforcer = enforcer.as_ref().unwrap();

    
    let appid = appid.unwrap_or("kernel");
    let res2 = enforcer.enforce((appid, res_path, op_name)).unwrap();
    println!("enforce {},{},{}, result:{}",appid, res_path, op_name,res2);
    info!("enforce {},{},{}, result:{}",appid, res_path, op_name,res2);    
    if appid == "kernel" {
        return res2;
    }

    let res = enforcer.enforce((userid, res_path, op_name)).unwrap();
    println!("enforce {},{},{} result:{}",userid, res_path, op_name,res);
    info!("enforce {},{},{} result:{}",userid, res_path, op_name,res);
    return res2 && res;
}

//test
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::test;
    use std::collections::HashMap;
    use casbin::{rhai::ImmutableString, CoreApi, DefaultModel, Enforcer, Filter, MemoryAdapter, MgmtApi};
    
    #[test]
    async fn test_simple_enforce() -> Result<(), Box<dyn std::error::Error>> {
        // 定义模型配置
        let model_str = r#"
[request_definition]
r = sub,act, obj 

[policy_definition]
p = sub, obj, act, eft

[role_definition]
g = _, _

[policy_effect]
e = priority(p.eft) || deny

[matchers]
m = g(r.sub, p.sub) && keyMatch(r.obj, p.obj) && regexMatch(r.act, p.act)
        "#;
    
        // 定义策略配置
        let policy_str = r#"
        p, owner, kv://*, read|write,allow
        p, owner, dfs://*, read|write,allow
        p, owner, fs://$device_id:/, read,allow
    
        p, kernel_service, kv://*, read,allow
        p, kernel_service, dfs://*, read,allow
        p, kernel_service, fs://$device_id:/, read,allow
    
        p, frame_service, kv://*, read,allow
        p, frame_service, dfs://*, read,allow
        p, frame_service, fs://$device_id:/, read,allow
    
        p, sudo_user, kv://*, read|write,allow
        p, sudo_user, dfs://*, read|write,allow
    
    
        p, user, dfs://homes/:userid, read|write,allow
        p, user, dfs://public,read|write,allow
        
    
        p, limit_user, dfs://homes/:userid, read,allow
    
        p, guest, dfs://public, read,allow
        p, bob,dfs://public,write,deny
    
        g, alice, owner
        g, bob, user
        g, charlie, user
        g, app1, app_service 
        "#;
    
        // 使用字符串创建 Casbin 模型和策略适配器
        let m = DefaultModel::from_str(model_str).await?;
        // 创建一个空的内存适配器
        let mut e = Enforcer::new(m, MemoryAdapter::default()).await?;

        // 手动加载策略
        for line in policy_str.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                let rule: Vec<String> = line.split(',').map(|s| s.trim().to_string()).collect();
                if rule[0] == "p" {
                    println!("add policy {:?}", &rule);
                    e.add_policy(rule[1..].to_vec()).await?;
                    
                } else if rule[0] == "g" {
                    println!("add group policy {:?}", &rule);
                    e.add_grouping_policy(rule[1..].to_vec()).await?;
                }
            }
        }

    
        // 测试权限
        let alice_read_kv = e.enforce(("alice","write","kv://config")).unwrap();
        println!("Alice can write kv://config: {}", alice_read_kv); // true
        assert_eq!(alice_read_kv, true);
    
        Ok(())
    }

    #[test]
    async fn test_enforce() {
        create_enforcer(None,None).await.unwrap();
        let res = enforce("ood01", None, "kv://boot", "read").await;
        assert_eq!(res, true);
        assert_eq!(enforce("bob", None, "dfs://homes/alice/apps/app2", "read").await, true);
        assert_eq!(enforce("alice", Some("app1"), "dfs://homes/alice/apps/app2/data", "write").await, false);
    }

}