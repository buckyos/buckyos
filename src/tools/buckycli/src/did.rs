use buckyos_kit::get_buckyos_system_etc_dir;
use clap::ArgMatches;
use jsonwebtoken::EncodingKey;
use ndn_lib::named_obj_to_jwt;

pub async fn sign_json_data(matches: &ArgMatches, private_key: Option<(&str, &EncodingKey)>) {
    let json = matches.get_one::<String>("json").unwrap();
    println!("data: {} ", json);
    let _ = serde_json::from_str::<serde_json::Value>(json);
    let json = serde_json::to_value(&json)
        .map_err(|e| {
            println!("serde_json::to_value error {}", e);
            e
        })
        .unwrap();

    // private_key的来源是 user_private_key.pem文件，这个文件可能为空
    if let Some((kid, private_key)) = private_key {
        // check json data valid
        let result = named_obj_to_jwt(&json, &private_key, Some(kid.to_string()))
            .map_err(|e| {
                println!("named_obj_to_jwt error {}", e);
                e
            })
            .unwrap();
        println!("named_obj_to_jwt {}", result);
    } else {
        // 没有 user_private_key.pem文件，从start config里面读取
        println!("empty user_private_key.pem file!");
        let start_params_file_path = get_buckyos_system_etc_dir().join("start_config.json");
        let start_params_str = tokio::fs::read_to_string(start_params_file_path)
            .await
            .unwrap();
        let start_params: serde_json::Value = serde_json::from_str(&start_params_str).unwrap();
        let user_private_key = start_params["private_key"].as_str().unwrap();
        let user_private_key = user_private_key.trim();
        println!("user_private_key: {}", user_private_key);

        let private_key = EncodingKey::from_ed_pem(user_private_key.as_bytes())
            .map_err(|e| {
                println!("EncodingKey::from_ed_pem error {}", e);
                e
            })
            .unwrap();
        let result = named_obj_to_jwt(&json, &private_key, Some("ood".to_string()))
            .map_err(|e| {
                println!("named_obj_to_jwt error {}", e);
                e
            })
            .unwrap();
        println!("named_obj_to_jwt: {}", result);
    }
}
