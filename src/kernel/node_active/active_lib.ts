import {buckyos} from 'buckyos';

export enum GatewayType {
    BuckyForward = "BuckyForward",
    PortForward = "PortForward",
}

type JsonValue = Record<string, any>;

export type ActiveWizzardData = {
    gatewy_type : GatewayType;
    is_direct_connect : boolean;

    sn_active_code : string;
    sn_user_name : string;
    sn_url :string;
    sn_host : string;

    use_self_domain : boolean;
    self_domain : string;

    admin_password_hash : string;
    friend_passcode:string;
    enable_guest_access : boolean;

    owner_public_key : string;
    owner_private_key : string;
    zone_config_jwt : string;


}

export let SN_API_URL:string = "https://web3.buckyos.ai/kapi/sn";

export function set_sn_api_url(url:string) {
    SN_API_URL = url;
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
    let result = await rpc_client.call("register_user",params);
    let code = result["code"];
    if (code == 0) {
        return true;
    }
    return false;
}



export async function register_sn_main_ood (user_name:string,device_name:string,device_did:string,device_ip:string,device_info:string) : Promise<boolean> {
    let rpc_client = new buckyos.kRPCClient(SN_API_URL);
    let result = await rpc_client.call("register",{
        user_name:user_name,
        device_name:device_name,
        device_did:device_did,
        device_ip:device_ip,
        device_info:device_info
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

export async function generate_zone_boot_config_jwt(sn:string,owner_private_key:string):Promise<string> {
    let rpc_client = new buckyos.kRPCClient("/kapi/active");
    const now = Math.floor(Date.now() / 1000);
    let zone_boot_config:JsonValue;
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

    let zoen_boot_config_str =  JSON.stringify(zone_boot_config);
    let result = await rpc_client.call("generate_zone_boot_config",{
        zone_boot_config:zoen_boot_config_str,
        private_key:owner_private_key   
    });
    let zone_boot_config_jwt = result["zone_boot_config_jwt"];
    return zone_boot_config_jwt;
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
    owner_public_key:string,owner_private_key:string,device_public_key:JsonValue,device_private_key:string,
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
        sn_host:wizzard_data.sn_host
    });
    return result["code"] == 0;
}

export async function end_active():Promise<boolean> {
    let rpc_client = new buckyos.kRPCClient("/kapi/active");
    let result = await rpc_client.call("end_active",{});
    return true;
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
        zone_name = data.sn_user_name + "." + data.sn_host;
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

