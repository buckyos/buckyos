import { buckyos } from 'buckyos';

import { ActiveConfig, ActiveWizzardData, GatewayType, JsonValue } from "./src/types";

export let SN_BASE_HOST:string = "buckyos.ai";
export let SN_API_URL:string = "https://sn." + SN_BASE_HOST + "/kapi/sn";
export let WEB3_BASE_HOST:string = "web3." + SN_BASE_HOST;

/*
激活的流程说明
## 尽可能的不依赖krpc/active? 但是又想依赖krpc/active提供的类型安全

## 构造设备信息 （一次性构造)

## 构造zone信息 

## 进行签名

## SN注册用户 + Zone

## SN 用户绑定Zone

## SN 注册设备

*/

export async function init_active_lib(config: ActiveConfig) {
    SN_BASE_HOST = config.sn_base_host
    SN_API_URL = config.http_schema + "://sn." + SN_BASE_HOST + "/kapi/sn";
    WEB3_BASE_HOST = "web3." + SN_BASE_HOST;
}

export async function createInitialWizardData (initial?: Partial<ActiveWizzardData>): Promise<ActiveWizzardData> {
    let [device_public_key,device_private_key] = await generate_key_pair();
    let device_did = "did:dev:"+ device_public_key["x"];
    console.log("device_did",device_did);
    let result:ActiveWizzardData = {
        gatewy_type: GatewayType.BuckyForward,
        is_direct_connect: false,
        device_public_key: device_public_key,
        device_private_key: device_private_key,
        sn_active_code: "",
        sn_user_name: "",
        sn_url: SN_API_URL,
        web3_base_host: WEB3_BASE_HOST,
        use_self_domain: false,
        self_domain: "",
        admin_password_hash: "",
        friend_passcode: "",
        enable_guest_access: false,
        owner_public_key: {},
        owner_private_key: "",
        zone_config_jwt: "",
        port_mapping_mode: "full",
        rtcp_port: 2980,
        is_wallet_runtime: false,
        wallet_user_name: "",
        ...initial,
    };

    return result;
}
  
export async function register_sn_user(user_name:string,active_code:string,public_key:string,zone_config_jwt:string,user_domain:string|null) : Promise<boolean> {
    let rpc_client = new buckyos.kRPCClient(SN_API_URL);
    let params:JsonValue = {
        user_name:user_name,
        active_code:active_code,
        public_key:public_key,
        zone_config:zone_config_jwt
    };
    if (user_domain != null) {
        params["user_domain"] = user_domain;
    }
    console.log("register_sn_user params",params);
    let result = await rpc_client.call("register_user",params);
    let code = result["code"];
    return code == 0;
}



export async function register_sn_main_ood (user_name:string,device_name:string,device_did:string,mini_config_jwt:string,device_ip:string,device_info:string) : Promise<boolean> {
    let rpc_client = new buckyos.kRPCClient(SN_API_URL);
    let result = await rpc_client.call("register",{
        user_name:user_name,
        device_name:device_name,
        device_did:device_did,
        device_ip:device_ip,
        device_info:device_info,
        mini_config_jwt:mini_config_jwt
    });
    let code = result["code"];
    if (code == 0) {
        return true;
    }
    return false;
}

export async function check_sn_active_code(sn_active_code:string) : Promise<boolean> {
    let rpc_client = new buckyos.kRPCClient(SN_API_URL);
    let result = await rpc_client.call("check_active_code",{active_code:sn_active_code});
    let valid = result["valid"];
    return valid;
}

export async function check_bucky_username(check_bucky_username:string) : Promise<boolean> {
    let rpc_client = new buckyos.kRPCClient(SN_API_URL);
    let result = await rpc_client.call("check_username",{username:check_bucky_username});
    let valid = result["valid"];
    return valid;
}

export async function generate_key_pair():Promise<[JsonValue,string]> {
    let rpc_client = new buckyos.kRPCClient("/kapi/active");
    let result = await rpc_client.call("generate_key_pair",{});
    let public_key = result["public_key"]
    let private_key = result["private_key"]
    return [public_key,private_key];
}

//这个函数在调用的时候，其实在执行激活操作了，用户只有在不使用SN的情况下，才需要调用该函数
export async function generate_zone_txt_records(sn:string,owner_public_key:JsonValue,owner_private_key:string|null,device_public_key:JsonValue,rtcp_port:number,is_by_wallet:boolean):Promise<JsonValue|null> {
    let zone_boot_config:JsonValue;
    const now = Math.floor(Date.now() / 1000);
    if (sn == "") {
        zone_boot_config = {
            oods: ["ood1"],
            exp: now + 3600*24*365*10, 
            iat:now,
        };
    } else {
        zone_boot_config = {
            oods: ["ood1"],
            sn: sn,
            exp: now + 3600*24*365*10, 
            iat:now,
        };
    }
    let zone_boot_config_str =  JSON.stringify(zone_boot_config);

    let device_mini_config:JsonValue = {
        "n": "ood1",
        "x": device_public_key["x"],
        "exp": now + 3600*24*365*10, 
    }
    if (rtcp_port != 2980) {
        device_mini_config["p"] = rtcp_port;
    }
    let device_mini_config_str =  JSON.stringify(device_mini_config);

    if (is_by_wallet) {
        let will_sign_str:string[] = [
            zone_boot_config_str,
            device_mini_config_str
        ]
        let signed_results:string[]|null = await buckyos.walletSignWithActiveDid(will_sign_str);
        if (signed_results == null) {
            console.error("Failed to sign zone txt records");
            return null;
        }
        return {
            "BOOT": signed_results[0],
            "DEV": signed_results[1],
            "PKX": owner_public_key["x"],
        }
    } else {
        let rpc_client = new buckyos.kRPCClient("/kapi/active");
        let result = await rpc_client.call("generate_zone_txt_records",{
            zone_boot_config:zone_boot_config_str,
            device_mini_config:device_mini_config_str,
            private_key:owner_private_key   
        });
        result["PKX"] = owner_public_key["x"];

        return result;
    }
}

export function isValidDomain(domain: string): boolean {
    const domainRegex = /^(?!:\/\/)([a-zA-Z0-9-_]{1,63}\.)+[a-zA-Z]{2,}$/;
    return domainRegex.test(domain);
}

export async function get_thisdevice_info():Promise<JsonValue> {
    let rpc_client = new buckyos.kRPCClient("/kapi/active");
    let result = await rpc_client.call("get_device_info",{});
    let device_info = result["device_info"];
    return device_info;
}

export async function active_ood(wizzard_data:ActiveWizzardData,zone_name:string,
    owner_public_key:JsonValue,owner_private_key:string,device_public_key:JsonValue,device_private_key:string,
 ):Promise<boolean> {
    let rpc_client = new buckyos.kRPCClient("/kapi/active");
    let result = await rpc_client.call("do_active",{
        user_name:wizzard_data.sn_user_name,
        zone_name:zone_name,
        gateway_type:wizzard_data.gatewy_type,
        public_key:owner_public_key,
        private_key:owner_private_key,
        device_public_key:device_public_key,
        device_private_key:device_private_key,
        admin_password_hash:wizzard_data.admin_password_hash,
        guest_access:wizzard_data.enable_guest_access,
        friend_passcode:wizzard_data.friend_passcode,
        sn_url:wizzard_data.sn_url,
        sn_host:wizzard_data.web3_base_host
    });
    return result["code"] == 0;
}

export async function end_active():Promise<boolean> {
    let rpc_client = new buckyos.kRPCClient("/kapi/active");
    let result = await rpc_client.call("end_active",{});
    return true;
}

export async function do_active_by_wallet(data:ActiveWizzardData):Promise<boolean> {
    // Generate device key pair
    let [device_public_key,device_private_key] = await generate_key_pair();
    let device_did = "did:dev:"+ device_public_key["x"];
    console.log("ood device_did",device_did);

    let need_sn = false;
    if (data.gatewy_type == GatewayType.BuckyForward) {
        need_sn = true;
    }

    if (!data.use_self_domain) {
        need_sn = true;
    }

    // // Register SN user if needed
    // if (need_sn && !data.is_wallet_runtime) {
    //     let user_domain = null;
    //     if(data.use_self_domain) {
    //         user_domain = data.self_domain;
    //     }
    //     let register_sn_user_result = await register_sn_user(
    //         data.sn_user_name,
    //         data.sn_active_code,
    //         JSON.stringify(data.owner_public_key),
    //         data.zone_config_jwt,
    //         user_domain);

    //     if (!register_sn_user_result) {
    //         return false;
    //     }
    // }

    let zone_name = "";
    if (data.use_self_domain) {
        zone_name = data.self_domain;
    } else {
        zone_name = data.sn_user_name + "." + data.web3_base_host;
    }

    // Step 1: Call prepare_params_for_active_by_wallet to get unsigned data
    let rpc_client = new buckyos.kRPCClient("/kapi/active");
    let prepare_params:JsonValue = {
        user_name: data.sn_user_name,
        zone_name: zone_name,
        gateway_type: data.gatewy_type,
        public_key: data.owner_public_key,
        device_public_key: device_public_key,
        device_private_key: device_private_key,
        support_container: "true",
        sn_url: data.sn_url || ""
    };

    let prepare_result = await rpc_client.call("prepare_params_for_active_by_wallet", prepare_params);
    if (prepare_result["code"] != undefined && prepare_result["code"] != 0) {
        console.error("Failed to prepare params for wallet activation");
        return false;
    }

    let device_config_json = prepare_result["device_config"];
    let device_mini_config_json = prepare_result["device_mini_config"];
    let rpc_token_json = prepare_result["rpc_token"];
    let device_info_json = prepare_result["device_info"];
    let device_did_from_server = prepare_result["device_did"];

    // Step 2: Sign the data using wallet's signWithActiveDid
    // Note: signWithActiveDid should accept a JSON object and return a JWT string
    // TODO: This method needs to be implemented in the wallet/buckyos API
    // For now, we'll use a type assertion to indicate this is expected to exist
    let device_doc_jwt: string;
    let device_mini_doc_jwt: string;
    let user_rpc_token: string | null = null;

    try {
        // Sign device_config
        // @ts-ignore - signWithActiveDid will be implemented in wallet
        device_doc_jwt = await buckyos.signWithActiveDid(device_config_json);
        
        // Sign device_mini_config
        // @ts-ignore - signWithActiveDid will be implemented in wallet
        device_mini_doc_jwt = await buckyos.signWithActiveDid(device_mini_config_json);
        
        // Sign rpc_token if needed
        if (rpc_token_json != null && need_sn) {
            // @ts-ignore - signWithActiveDid will be implemented in wallet
            user_rpc_token = await buckyos.signWithActiveDid(rpc_token_json);
        }
    } catch (error) {
        console.error("Failed to sign data with wallet:", error);
        return false;
    }

    // Step 3: Call do_active_by_wallet with signed JWTs
    // Only pass essential parameters - other info will be extracted from JWTs
    let active_params:JsonValue = {
        device_doc_jwt: device_doc_jwt,
        device_mini_doc_jwt: device_mini_doc_jwt,
        device_private_key: device_private_key,
        zone_name: zone_name,
        owner_public_key: data.owner_public_key, // Still needed for JWT verification
        sn_url: data.sn_url || ""
    };

    // Optional parameters for SN registration
    if (user_rpc_token != null && need_sn) {
        active_params["user_rpc_token"] = user_rpc_token;
    }
    
    if (need_sn && device_info_json != null) {
        active_params["device_info"] = device_info_json;
    }

    let active_result = await rpc_client.call("do_active_by_wallet", active_params);
    let code = active_result["code"];
    return code == 0;
}

export async function do_active(data:ActiveWizzardData):Promise<boolean> {
    //generate device key pair
    let [device_public_key,device_private_key] = await generate_key_pair();
    let device_did = "did:dev:"+ device_public_key["x"];
    console.log("ood device_did",device_did);

    let need_sn = false;
    if (data.gatewy_type == GatewayType.BuckyForward) {
        need_sn = true;
    }

    if (!data.use_self_domain) {
        need_sn = true;
    }
    // register sn user
    if (need_sn) {
        let user_domain = null;
        if (data.sn_user_name == null || data.sn_user_name == "" || data.sn_active_code == null || data.sn_active_code == "") {
            return false;
        }
        if(data.use_self_domain) {
            user_domain = data.self_domain;
        }
        let register_sn_user_result = await register_sn_user(
            data.sn_user_name,
            data.sn_active_code,
            JSON.stringify(data.owner_public_key),
            data.zone_config_jwt,
            user_domain);

        if (!register_sn_user_result) {
            return false;
        }
    }
    let zone_name = "";
    if (data.use_self_domain) {
        zone_name = data.self_domain;
    } else {
        if (data.sn_user_name == null) {
            return false;
        }
        zone_name = data.sn_user_name + "." + data.web3_base_host;
    }

    if (data.owner_private_key == null) {
        return false;
    }

    let active_ood_result = await active_ood(
        data,
        zone_name,
        data.owner_public_key,
        data.owner_private_key,
        device_public_key,
        device_private_key,
    );

    if (!active_ood_result) {
        return false;
    }

    return true;
}
