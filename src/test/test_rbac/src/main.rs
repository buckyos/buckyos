use ::kRPC::*;
use buckyos_kit::*;
use name_lib::*;
use serde_json::json;

async fn test() -> std::result::Result<(), String> {
    let app_id = "buckyos_filebrowser";
    let other_app_id = "buckyos_systest";

    //读取etc下的私钥和用户信息
    let etc_dir = get_buckyos_system_etc_dir();
    let bucky_cli_dir = etc_dir.join(".buckycli");
    println!("buckycli dir {:?}", bucky_cli_dir);

    if !bucky_cli_dir.exists() {
        println!("bucky_cli_dir not exists");
        return Err("bucky_cli_dir not exists".to_string());
    }

    let user_config_file = bucky_cli_dir.join("user_config.json");
    let user_private_key_file = bucky_cli_dir.join("user_private_key.pem");
    if !user_config_file.exists() {
        println!("user config file not exists");
        return Err("user config file not exists".to_string());
    }
    if !user_private_key_file.exists() {
        println!("user private key file not exists");
        return Err("user private key file not exists".to_string());
    }

    let owner_config = OwnerConfig::load_owner_config(&user_config_file).map_err(|e| {
        println!("Failed to load owner config: {}", e);
        e.to_string()
    })?;

    let private_key = load_private_key(&user_private_key_file).map_err(|e| {
        println!("Failed to load private key: {}", e);
        e.to_string()
    })?;

    let user_name = owner_config.name.clone();
    println!("owner user name: {:?}", user_name);

    // admin(owner) + kernel(buckycli)
    println!("*************************");
    println!("Begin admin + kernel test");
    println!("*************************");
    let (session_token_str, _real_session_token) =
        RPCSessionToken::generate_jwt_token(&user_name, "buckycli", None, &private_key).map_err(
            |e| {
                println!("Failed to generate session token for admin + kernel: {}", e);
                e.to_string()
            },
        )?;

    println!("generate session token for admin + kernel success");

    let client = kRPC::new(
        "http://127.0.0.1:3200/kapi/system_config",
        Some(session_token_str),
    );

    println!("==> test GET system/rbac/policy via admin + kernel, should success");
    let result = client
        .call("sys_config_get", json!({"key": "system/rbac/policy"}))
        .await
        .map_err(|e| {
            println!("Failed to get system/rbac/policy via admin + kernel: {}", e);
            e.to_string()
        })?;

    if result.is_null() {
        println!("result is null");
        return Err("system/rbac/policy is null".to_string());
    }

    println!("<== test GET system/rbac/policy via admin + kernel, pass");

    println!("==> test CREATE system/test_rbac/set via admin + kernel, should failed");
    let _result = client
        .call(
            "sys_config_create",
            json!({"key": "system/test_rbac/set", "value": "test_rbac_set_value"}),
        )
        .await
        .map_err(|e| {
            println!(
                "Failed to create system/test_rbac/set via admin + kernel: {}",
                e
            );
            e.to_string()
        });
    if _result.is_ok() {
        println!("test CREATE system/test_rbac/set via admin + kernel should failed");
        return Err(
            "test CREATE system/test_rbac/set via admin + kernel should failed".to_string(),
        );
    }
    println!("<== test CREATE system/test_rbac/set via admin + kernel, pass");

    println!("==> test SET system/test_rbac/set via admin + kernel, should failed");
    let _result = client
        .call(
            "sys_config_set",
            json!({"key": "system/test_rbac/set", "value": "test_rbac_set_value"}),
        )
        .await
        .map_err(|e| {
            println!(
                "Failed to set system/test_rbac/set via admin + kernel: {}",
                e
            );
            e.to_string()
        });
    if _result.is_ok() {
        println!("test SET system/test_rbac/set via admin + kernel should failed");
        return Err("test SET system/test_rbac/set via admin + kernel should failed".to_string());
    }
    println!("<== test SET system/test_rbac/set via admin + kernel, pass");

    println!("==> test DELETE system/test_rbac/set via admin + kernel, should failed");
    let _result = client
        .call("sys_config_delete", json!({"key": "system/test_rbac/set"}))
        .await
        .map_err(|e| {
            println!(
                "Failed to delete system/test_rbac/set via admin + kernel: {}",
                e
            );
            e.to_string()
        });
    if _result.is_ok() {
        println!("test DELETE system/test_rbac/set via admin + kernel should failed");
        return Err(
            "test DELETE system/test_rbac/set via admin + kernel should failed".to_string(),
        );
    }
    println!("<== test DELETE system/test_rbac/set via admin + kernel, pass");

    println!(
        "==> test SET users/devtest/apps/sys-test/settings via admin + kernel, should success"
    );
    let _result = client
        .call(
            "sys_config_set",
            json!({"key": "users/devtest/apps/sys-test/settings", "value": "test_rbac_set_value"}),
        )
        .await
        .map_err(|e| {
            println!(
                "Failed to set users/devtest/apps/sys-test/settings via admin + kernel: {}",
                e
            );
            e.to_string()
        })?;
    println!("<== test SET users/devtest/apps/sys-test/settings via admin + kernel, pass");

    println!(
        "==> test GET users/devtest/apps/sys-test/settings via admin + kernel, should success"
    );
    let _result = client
        .call(
            "sys_config_get",
            json!({"key": "users/devtest/apps/sys-test/settings"}),
        )
        .await
        .map_err(|e| {
            println!(
                "Failed to get users/devtest/apps/sys-test/settings via admin + kernel: {}",
                e
            );
            e.to_string()
        })?;
    println!("<== test GET users/devtest/apps/sys-test/settings via admin + kernel, pass");

    println!("***********************");
    println!("End admin + kernel test");
    println!("***********************");

    //admin + app
    println!("**********************");
    println!("Begin admin + app test");
    println!("**********************");
    let (session_token_str, _real_session_token) =
        RPCSessionToken::generate_jwt_token(&user_name, app_id, None, &private_key).map_err(
            |e| {
                println!(
                    "Failed to generate session token: for admin + {} {}",
                    app_id, e
                );
                e.to_string()
            },
        )?;
    println!("generate session token for admin + {} success", app_id);
    let client = kRPC::new(
        "http://127.0.0.1:3200/kapi/system_config",
        Some(session_token_str),
    );

    println!(
        "==> test GET system/rbac/policy via admin + {}, should failed",
        app_id
    );
    let result = client
        .call("sys_config_get", json!({"key": "system/rbac/policy"}))
        .await;
    if result.is_ok() {
        println!(
            "test GET system/rbac/policy via admin + {} should failed",
            app_id
        );
        return Err(format!(
            "test GET system/rbac/policy via admin + {} should failed",
            app_id
        ));
    }
    println!(
        "<== test GET system/rbac/policy via admin + {}, pass",
        app_id
    );

    println!(
        "==> test SET system/test_rbac/set via admin + {}, should failed",
        app_id
    );
    let result = client
        .call(
            "sys_config_set",
            json!({"key": "system/test_rbac/set", "value": "test_rbac_set_value"}),
        )
        .await;
    if result.is_ok() {
        println!(
            "test SET system/test_rbac/set via admin + {} should failed",
            app_id
        );
        return Err(format!(
            "test SET system/test_rbac/set via admin + {} should failed",
            app_id
        ));
    }
    println!(
        "<== test SET system/test_rbac/set via admin + {}, pass",
        app_id
    );

    println!(
        "==> test SET users/devtest/apps/{}/settings via admin + {}, should success",
        app_id, app_id
    );
    let _result = client
        .call(
            "sys_config_set",
            json!({"key": format!("users/devtest/apps/{}/settings", app_id), "value": "test_rbac_set_value"}),
        )
        .await
        .map_err(|e| {
            println!(
                "Failed to set users/devtest/apps/{}/settings via admin + {}: {}",
                app_id, app_id, e
            );
            e.to_string()
        })?;
    println!(
        "<== test SET users/devtest/apps/{}/settings via admin + {}, pass",
        app_id, app_id
    );

    println!(
        "==> test GET users/devtest/apps/{}/settings via admin + {}, should success",
        app_id, app_id
    );
    let _result = client
        .call(
            "sys_config_get",
            json!({"key": format!("users/devtest/apps/{}/settings", app_id)}),
        )
        .await
        .map_err(|e| {
            println!(
                "Failed to get users/devtest/apps/{}/settings via admin + {}: {}",
                app_id, app_id, e
            );
            e.to_string()
        })?;
    println!(
        "<== test GET users/devtest/apps/{}/settings via admin + {}, pass",
        app_id, app_id
    );

    println!(
        "==> test SET users/devtest/apps/{}/config via admin + {}, should failed",
        app_id, app_id
    );
    let result = client
        .call(
            "sys_config_set",
            json!({"key": format!("users/devtest/apps/{}/config", app_id), "value": "test_rbac_set_value"}),
        )
        .await;
    if result.is_ok() {
        println!(
            "test SET users/devtest/apps/{}/config via admin + {} should failed",
            app_id, app_id
        );
        return Err(format!(
            "test SET users/devtest/apps/{}/config via admin + {} should failed",
            app_id, app_id
        ));
    }
    println!(
        "<== test SET users/devtest/apps/{}/config via admin + {}, pass",
        app_id, app_id
    );

    println!(
        "==> test SET users/devtest/apps/{}/info via admin + {}, should success",
        app_id, app_id
    );
    let _result = client
        .call(
            "sys_config_set",
            json!({"key": format!("users/devtest/apps/{}/info", app_id), "value": "test_rbac_set_value"}),
        )
        .await
        .map_err(|e| {
            println!(
                "Failed to set users/devtest/apps/{}/info via admin + {}: {}",
                app_id, app_id, e
            );
            e.to_string()
        })?;
    println!(
        "<== test SET users/devtest/apps/{}/info via admin + {}, pass",
        app_id, app_id
    );

    println!("********************");
    println!("End admin + app test");
    println!("********************");

    //appA write appB
    println!("**********************************");
    println!("Begin admin + appA Write appB test");
    println!("**********************************");

    println!(
        "==> test GET users/devtest/apps/{}/settings via admin + {}, should failed",
        other_app_id, app_id
    );
    let result = client
        .call(
            "sys_config_get",
            json!({"key": format!("users/devtest/apps/{}/settings", other_app_id)}),
        )
        .await;
    if result.is_ok() {
        println!(
            "test GET users/devtest/apps/{}/settings via admin + {} should failed",
            other_app_id, app_id
        );
        return Err(format!(
            "test GET users/devtest/apps/{}/settings via admin + {} should failed",
            other_app_id, app_id
        ));
    }
    println!(
        "<== test GET users/devtest/apps/{}/settings via admin + {}, pass",
        other_app_id, app_id
    );

    println!(
        "==> test SET users/devtest/apps/{}/settings via admin + {}, should failed",
        other_app_id, app_id
    );
    let result = client
        .call(
            "sys_config_set",
            json!({"key": format!("users/devtest/apps/{}/settings", other_app_id), "value": "test_rbac_set_value"}),
        )
        .await;
    if result.is_ok() {
        println!(
            "test SET users/devtest/apps/{}/settings via admin + {} should failed",
            other_app_id, app_id
        );
        return Err(format!(
            "test SET users/devtest/apps/{}/settings via admin + {} should failed",
            other_app_id, app_id
        ));
    }
    println!(
        "<== test SET users/devtest/apps/{}/settings via admin + {}, pass",
        other_app_id, app_id
    );

    println!("********************************");
    println!("End admin + appA Write appB test");
    println!("********************************");

    Ok(())
}

#[tokio::main]
async fn main() {
    let result = test().await;
    if result.is_err() {
        println!("test failed: {}", result.err().unwrap());
        std::process::exit(1);
    }
    println!("test success");
    std::process::exit(0);
}
