import { buckyos } from 'buckyos';

import { ActiveConfig, ActiveWizzardData, EnabledFeatures, GatewayType, JsonValue } from "./src/types";

export let SN_BASE_HOST:string = "buckyos.ai";
export let SN_HOST:string = "sn." + SN_BASE_HOST;
export let SN_API_URL:string = "https://sn." + SN_BASE_HOST + "/kapi/sn";
export let SN_AUTH_API_URL:string = SN_API_URL + "/auth";
export let SN_DEVICEINFO_API_URL:string = SN_API_URL + "/deviceinfo";
export let SN_BNS_API_URL:string = "https://sn." + SN_BASE_HOST + "/kapi/bns";
export let BNS_EVM_CONFIG:JsonValue|null = null;
export let WEB3_BASE_HOST:string = "web3." + SN_BASE_HOST;
export let AI_PROVIDER_TUTORIAL_URL:string = "https://buckyos.ai";
export let TELEGRAM_BOT_API_TOKEN_TUTORIAL_URL:string = "https://core.telegram.org/bots/tutorial";
export let TELEGRAM_ACCOUNT_ID_TUTORIAL_URL:string = "https://core.telegram.org/api/bots/ids";

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
    SN_BASE_HOST = config.sn_base_host;
    SN_HOST = "sn." + SN_BASE_HOST;
    SN_API_URL = config.http_schema + "://sn." + SN_BASE_HOST + "/kapi/sn";
    SN_AUTH_API_URL = SN_API_URL + "/auth";
    SN_DEVICEINFO_API_URL = SN_API_URL + "/deviceinfo";
    SN_BNS_API_URL = config.http_schema + "://sn." + SN_BASE_HOST + "/kapi/bns";
    BNS_EVM_CONFIG = config.bns_evm || null;
    WEB3_BASE_HOST = "web3." + SN_BASE_HOST;
    AI_PROVIDER_TUTORIAL_URL =
        config.ai_provider_tutorial_url || AI_PROVIDER_TUTORIAL_URL;
    TELEGRAM_BOT_API_TOKEN_TUTORIAL_URL =
        config.telegram_bot_api_token_tutorial_url ||
        TELEGRAM_BOT_API_TOKEN_TUTORIAL_URL;
    TELEGRAM_ACCOUNT_ID_TUTORIAL_URL =
        config.telegram_account_id_tutorial_url ||
        TELEGRAM_ACCOUNT_ID_TUTORIAL_URL;
}

// 凭证类字段绝不进 console：SN token、私钥、密码 hash。
// 与服务端 active_server.rs 的日志脱敏名单（is_sensitive_param_key）保持同一纪律。
const SENSITIVE_LOG_KEYS = new Set([
    "sn_access_token",
    "sn_refresh_token",
    "private_key",
    "device_private_key",
    "owner_private_key",
    "bns_evm_private_key",
    "admin_password_hash",
    "friend_passcode",
    "pwd_hash",
]);

export function redact_for_log(value: unknown): unknown {
    if (Array.isArray(value)) {
        return value.map(redact_for_log);
    }
    if (value != null && typeof value === "object") {
        const redacted: JsonValue = {};
        for (const [key, child] of Object.entries(value)) {
            if (SENSITIVE_LOG_KEYS.has(key)) {
                const len = typeof child === "string" ? child.length : 0;
                redacted[key] = `[redacted:${len} chars]`;
            } else {
                redacted[key] = redact_for_log(child);
            }
        }
        return redacted;
    }
    return value;
}

export function resolveEnabledFeatures(
    snActiveCode: string | null | undefined,
    explicit?: Partial<EnabledFeatures> | null
): EnabledFeatures {
    const hasActiveCode =
        typeof snActiveCode === "string" && snActiveCode.trim().length > 0;
    return {
        llm_router: Boolean(explicit?.llm_router) || hasActiveCode,
    };
}

export async function createInitialWizardData (initial?: Partial<ActiveWizzardData>): Promise<ActiveWizzardData> {
    let [device_public_key,device_private_key] = await generate_key_pair();
    let owner_public_key:JsonValue = {};
    let owner_private_key:string = "";
    if (!initial?.is_wallet_runtime ) {
        console.log("generate_key_pair for owner");
        [owner_public_key,owner_private_key] = await generate_key_pair();
    }  else {
        owner_public_key = initial?.owner_public_key as JsonValue;
    }

    console.log("owner_public_key",owner_public_key);
    let result:ActiveWizzardData = {
        gatewy_type: GatewayType.BuckyForward,
        //is_direct_connect: false,
        device_public_key: device_public_key,
        device_private_key: device_private_key,
        sn_active_code: "",
        sn_user_name: "",
        sn_access_token: null,
        sn_refresh_token: null,
        //web3_base_host: WEB3_BASE_HOST,
        use_self_domain: false,
        self_domain: "",
        admin_password_hash: "",
        friend_passcode: "",
        enable_guest_access: false,
        owner_private_key: owner_private_key,
        bns_evm_private_key: initial?.bns_evm_private_key ?? null,
        owner_public_key: owner_public_key,
        port_mapping_mode: "full",
        rtcp_port: 2980,
        is_wallet_runtime: false,
        owner_user_name: "",
        ai_provider_config: {
            openai_api_token: "",
            claude_api_token: "",
            google_api_token: "",
            openrouter_api_token: "",
            glm_api_token: "",
        },
        jarvis_msg_tunnel_config: {
            telegram_bot_api_token: "",
            telegram_account_id: "",
        },
        ...initial,
        enabled_features: resolveEnabledFeatures(
            initial?.sn_active_code,
            initial?.enabled_features
        ),
    };
    console.log("createInitialWizardData result",redact_for_log(result));
    return result;
}

function is_need_sn(wizzard_data:ActiveWizzardData):boolean {
    if (wizzard_data.gatewy_type != GatewayType.WAN) {
        return true;
    }
    if (!wizzard_data.use_self_domain) {
        return true;
    }
    return false;
}

export function get_net_id_by_gateway_type(gateway_type:GatewayType,port_mapping_mode:string):string {
    if (gateway_type == GatewayType.WAN) {
        return "wan";
    }
    if (gateway_type == GatewayType.PortForward) {
        if (port_mapping_mode == "full") {
            return "wan_dyn";
        }
        if (port_mapping_mode == "rtcp_only") {
            return "portmap";
        }
    }

    return "nat";
}

// 参考 test_config.rs create_zone_boot_config_jwt
// 实现create zone boot config和create_device_mini_config
// 注意ood的net_id影响zone_boot_config,但不影响device_mini_config
function create_zone_boot_config(sn:string|null,ood_net_id:string|null):JsonValue {
    const now = Math.floor(Date.now() / 1000);
    let ood = "ood1";
    if (ood_net_id != null) {
        if (ood_net_id != "nat") {
            ood = ood + "@" + ood_net_id;
        }
    }
    let zone_boot_config:JsonValue = {
        "oods": [ood],
        "exp": now + 3600*24*365*10
    }
    if (sn != null) {
        if (sn != "") {
            zone_boot_config["sn"] = sn;
        }
    }
    return zone_boot_config;
}

// create device mini config
//test_config.rs create_device_mini_config_jwt
function create_device_mini_config(device_public_key:JsonValue,rtcp_port:number):JsonValue {
    const now = Math.floor(Date.now() / 1000);
    let device_mini_config:JsonValue = {
        "n": "ood1",
        "x": device_public_key["x"],
        "exp": now + 3600*24*365*10, 
    }
    if (rtcp_port != 2980) {
        device_mini_config["p"] = rtcp_port;
    }
    return device_mini_config;
}

function create_sn_rpc_client(api_url:string, sessionToken?:string) {
    return new buckyos.kRPCClient(api_url, sessionToken);
}

export type SnLoginResult = JsonValue & {
    access_token?: string;
    refresh_token?: string;
};

export type CheckBuckyUsernameResult = JsonValue & {
    valid: boolean;
    reason?: string;
    message?: string;
    normalized_name?: string;
};

export async function register_sn_user(user_name:string,pwd_hash:string,active_code:string): Promise<SnLoginResult> {
    let rpc_client = create_sn_rpc_client(SN_AUTH_API_URL);
    let result: JsonValue = await rpc_client.call("auth.register",{
        name:user_name,
        pwd_hash:pwd_hash,
        active_code:active_code
    });
    return result as SnLoginResult;
}

export async function login(user_name:string,pwd_hash:string,active_code:string):Promise<SnLoginResult> {
    let rpc_client = create_sn_rpc_client(SN_AUTH_API_URL);
    let result: JsonValue = await rpc_client.call("auth.login",{
        name:user_name,
        pwd_hash:pwd_hash,
        active_code:active_code
    });
    return result as SnLoginResult;
}

// export function hash_sn_password(pwd:string):string {
//     //todo:
// }

export async function login_by_password_and_activecode(user_name:string,pwd_hash:string,active_code:string):Promise<SnLoginResult> {
    return login(user_name, pwd_hash, active_code);
}

// 用 refresh_token（aud="sn-refresh"，1天）换一个新的 access token（aud="sn"，1小时）。
// refresh 失效（过期/被撤销）时返回 null，由调用方决定是否重新登录。
export async function refresh_sn_access_token(refresh_token:string):Promise<string|null> {
    let rpc_client = create_sn_rpc_client(SN_AUTH_API_URL);
    let result: JsonValue = await rpc_client.call("auth.refresh",{refresh_token:refresh_token});
    if (result["code"] !== undefined && result["code"] !== 0) {
        return null;
    }
    let access_token = result["access_token"];
    if (typeof access_token === "string" && access_token.length > 0) {
        return access_token;
    }
    return null;
}

// 激活最终提交前获取新鲜的 SN access token。
// access token 只有 1 小时有效期：SecurityStep 里注册/登录拿到的 token，到用户点
// 激活时可能早已过期，所以提交前总是重新换取——优先 refresh_token，失败或缺失
// （如钱包流程没有 SecurityStep）则用 SN 密码 hash 重新登录。
// 返回的 token 只用于本次激活 RPC：不写回持久化配置，也不要打进日志。
async function acquire_sn_access_token(
    sn_user_name:string|null,
    pwd_hash:string|null,
    refresh_token:string|null
):Promise<string> {
    if (refresh_token) {
        try {
            const fresh = await refresh_sn_access_token(refresh_token);
            if (fresh) {
                return fresh;
            }
            console.warn("SN auth.refresh rejected, fallback to re-login");
        } catch (error) {
            console.warn("SN auth.refresh failed, fallback to re-login:", error);
        }
    }
    if (!sn_user_name || !pwd_hash) {
        throw new Error("failed to get SN access token: no usable refresh_token and no SN credentials to re-login");
    }
    const login_result = await login(sn_user_name, pwd_hash, "");
    if (login_result.code !== undefined && login_result.code !== 0) {
        throw new Error("failed to get SN access token: SN re-login rejected");
    }
    if (!login_result.access_token) {
        throw new Error("failed to get SN access token: SN login response has no access_token");
    }
    return login_result.access_token;
}

// register_sn_main_ood 已删除：SN-Auth 两阶段设计下设备注册（device.register）
// 由激活服务端（active_server 的 sn_register_device_online）带 sn_access_token
// 完成，浏览器端不再直连 SN 注册设备。

export async function check_sn_active_code(sn_active_code:string) : Promise<boolean> {
    let rpc_client = create_sn_rpc_client(SN_AUTH_API_URL);
    let result: JsonValue = await rpc_client.call("auth.check_active_code",{active_code:sn_active_code});
    let valid = result["valid"];
    return valid;
}

export async function check_bucky_username(check_bucky_username:string) : Promise<CheckBuckyUsernameResult> {
    let rpc_client = create_sn_rpc_client(SN_AUTH_API_URL);
    let result: JsonValue = await rpc_client.call("auth.check_username",{name:check_bucky_username});
    return result as CheckBuckyUsernameResult;
}

export async function generate_key_pair():Promise<[JsonValue,string]> {
    let rpc_client = new buckyos.kRPCClient("/kapi/active");
    let result: JsonValue = await rpc_client.call("generate_key_pair",{});
    let public_key = result["public_key"]
    let private_key = result["private_key"]
    return [public_key,private_key];
}

function normalizeWalletSignResult(
    result: unknown
): { signatures: (string | null)[]; pwd_hash: string | null } | null {
    if (result == null) {
        return null;
    }

    if (Array.isArray(result)) {
        return {
            signatures: result as (string | null)[],
            pwd_hash: null,
        };
    }

    if (typeof result === "object") {
        const typed = result as { signatures?: unknown; pwd_hash?: unknown };
        return {
            signatures: Array.isArray(typed.signatures) ? (typed.signatures as (string | null)[]) : [],
            pwd_hash: typeof typed.pwd_hash === "string" && typed.pwd_hash.trim().length > 0
                ? typed.pwd_hash.trim()
                : null,
        };
    }

    return null;
}

//这个函数在调用的时候，其实在执行激活操作了，用户只有在不使用SN的情况下，才需要调用该函数
export async function generate_zone_txt_records(sn:string,
    owner_public_key:JsonValue,
    owner_private_key:string|null,
    device_public_key:JsonValue,
    net_id:string|null,
    rtcp_port:number,
    is_by_wallet:boolean):Promise<JsonValue|null> {
    let zone_boot_config = create_zone_boot_config(sn,net_id);
    let zone_boot_config_str =  JSON.stringify(zone_boot_config);

    let device_mini_config = create_device_mini_config(device_public_key,rtcp_port);
    let device_mini_config_str =  JSON.stringify(device_mini_config);

    if (is_by_wallet) {
        let will_sign_objects:JsonValue[] = [
            zone_boot_config,
            device_mini_config
        ]
        const signResult = normalizeWalletSignResult(
            await buckyos.walletSignWithActiveDid(will_sign_objects)
        );
        if (signResult == null || signResult.signatures.length < 2) {
            console.error("Failed to sign zone txt records");
            return null;
        }
        return {
            "BOOT": signResult.signatures[0],
            "DEV": signResult.signatures[1],
            "PKX": owner_public_key["x"],
        }
    } else {
        let rpc_client = new buckyos.kRPCClient("/kapi/active");
        let result: JsonValue = await rpc_client.call("generate_zone_txt_records",{
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
    let result: JsonValue = await rpc_client.call("get_device_info",{});
    let device_info = result["device_info"];
    return device_info;
}


export async function do_active_by_wallet(data:ActiveWizzardData):Promise<boolean> {

    let real_sn_host = "";
    let need_sn = false;
    if (data.gatewy_type == GatewayType.BuckyForward) {
        real_sn_host = SN_HOST;
        need_sn = true;
    }

    if (!data.use_self_domain) {
        need_sn = true;
        real_sn_host = SN_HOST;
    }

    let zone_name = "";
    if (data.use_self_domain) {
        zone_name = data.self_domain;
    } else {
        zone_name = data.sn_user_name + "." + WEB3_BASE_HOST;
    }

    // Step 1: Call prepare_params_for_active_by_wallet to get unsigned data
    let rpc_client = new buckyos.kRPCClient("/kapi/active");
    let prepare_params:JsonValue = {
        user_name: data.owner_user_name,
        zone_name: zone_name,
        net_id:get_net_id_by_gateway_type(data.gatewy_type,data.port_mapping_mode),
        public_key: data.owner_public_key,
        device_public_key: data.device_public_key,
        device_private_key: data.device_private_key,
        device_rtcp_port: data.rtcp_port,
        support_container: "true",
        sn_username: data.sn_user_name,
        sn_url: need_sn ? SN_API_URL : null
    };

    let prepare_result: JsonValue = await rpc_client.call("prepare_params_for_active_by_wallet", prepare_params);
    if (prepare_result["code"] != undefined && prepare_result["code"] != 0) {
        console.error("Failed to prepare params for wallet activation");
        return false;
    }

    let boot_config_json = create_zone_boot_config(real_sn_host,null);
    let mini_device_config_json = create_device_mini_config(data.device_public_key,data.rtcp_port);
    let device_config_json = prepare_result["device_config"];
    let device_info_json = prepare_result["device_info"];

    let signed_results:(string | null)[]|null = null;
    let wallet_pwd_hash:string|null = null;
    try {
        let will_sign_payloads:Record<string,unknown>[] = [
            boot_config_json,
            mini_device_config_json,
            device_config_json,
        ]
        const signResult = normalizeWalletSignResult(
            await buckyos.walletSignWithActiveDid(will_sign_payloads)
        );
        if (signResult == null || signResult.signatures.length < 3) {
            console.error("Failed to sign zone txt records");
            return false;
        }
        signed_results = signResult.signatures;
        wallet_pwd_hash = signResult.pwd_hash;
        console.log("wallet activation pwd_hash present:", !!wallet_pwd_hash);
        if (!wallet_pwd_hash) {
            throw new Error("missing password hash");
        }
    } catch (error) {
        console.error("Failed to sign data with wallet:", error);
        if (error instanceof Error && error.message === "missing password hash") {
            throw error;
        }
        return false;
    }
    //console.log("signed_results",signed_results);
    let boot_config_jwt = signed_results[0];
    console.log("boot_config_jwt",boot_config_jwt);
    let mini_device_config_jwt = signed_results[1];
    console.log("mini_device_config_jwt",mini_device_config_jwt);
    let device_config_jwt = signed_results[2];
    console.log("device_config_jwt",device_config_jwt);

    // 服务端在 sn_url 存在时强制要求 sn_access_token（SN-Auth 两阶段设计，
    // 自签 sn_device_proof 已删除）。钱包流程没有 SecurityStep，这里用钱包
    // 签名返回的 pwd_hash 登录 SN 账号换取 access token。
    let sn_access_token:string|null = null;
    if (need_sn) {
        sn_access_token = await acquire_sn_access_token(
            data.sn_user_name,
            wallet_pwd_hash,
            data.sn_refresh_token ?? null
        );
    }

    let active_params:JsonValue = {
        boot_config_jwt: boot_config_jwt,
        device_doc_jwt: device_config_jwt,
        device_mini_doc_jwt: mini_device_config_jwt,
        device_private_key: data.device_private_key,
        device_info: device_info_json,

        user_name:data.owner_user_name,
        zone_name: zone_name,
        is_self_domain: data.use_self_domain,
        public_key: data.owner_public_key, // Still needed for JWT verification
        admin_password_hash: wallet_pwd_hash,
        guest_access: data.enable_guest_access,
        friend_passcode: data.friend_passcode,
        ai_provider_config: data.ai_provider_config,
        jarvis_msg_tunnel_config: data.jarvis_msg_tunnel_config,
        sn_active_code: data.sn_active_code,
        enabled_features: resolveEnabledFeatures(
            data.sn_active_code,
            data.enabled_features
        ),

        sn_url: need_sn ? SN_API_URL : null,
        bns_url: need_sn ? SN_BNS_API_URL : null,
        bns_evm: BNS_EVM_CONFIG,
        bns_evm_private_key: data.bns_evm_private_key || null,
        sn_username: data.sn_user_name,
        sn_access_token: sn_access_token,
    };

    let active_result: JsonValue = await rpc_client.call("do_active_by_wallet", active_params);
    let code = active_result["code"];
    return code == 0;
}

export async function do_active(data:ActiveWizzardData):Promise<boolean> {
    let need_sn = is_need_sn(data);
    let sn_url:string|null = null;
    if (need_sn) {
        if (data.sn_user_name == null || data.sn_user_name == "") {
            return false;
        }
        sn_url = SN_API_URL;
    }
    let zone_name = "";
    if (data.use_self_domain) {
        zone_name = data.self_domain;
    } else {
        if (data.sn_user_name == null) {
            return false;
        }
        zone_name = data.sn_user_name + "." + WEB3_BASE_HOST;
    }

    if (data.owner_private_key == null) {
        return false;
    }

    // 服务端在 sn_url 存在时强制要求 sn_access_token（SN-Auth 两阶段设计）。
    // SecurityStep 里拿到的 token 可能因用户在向导停留过久而过期（1小时），
    // 提交前用 refresh_token 换新，兜底用管理密码 hash（与 SN 账号密码同源）重登。
    let sn_access_token:string|null = null;
    if (need_sn) {
        sn_access_token = await acquire_sn_access_token(
            data.sn_user_name,
            data.admin_password_hash || null,
            data.sn_refresh_token ?? null
        );
    }

    console.log("call do_active,param:",redact_for_log(data));
    let rpc_client = new buckyos.kRPCClient("/kapi/active");
    let result: JsonValue = await rpc_client.call("do_active",{
        user_name:data.owner_user_name,
        zone_name:zone_name,
        net_id:get_net_id_by_gateway_type(data.gatewy_type,data.port_mapping_mode),
        public_key:data.owner_public_key,
        private_key:data.owner_private_key,
        device_public_key:data.device_public_key,
        device_private_key:data.device_private_key,
        admin_password_hash:data.admin_password_hash,
        guest_access:data.enable_guest_access,
        friend_passcode:data.friend_passcode,
        device_rtcp_port:data.rtcp_port,
        ai_provider_config:data.ai_provider_config,
        jarvis_msg_tunnel_config:data.jarvis_msg_tunnel_config,
        sn_active_code:data.sn_active_code,
        enabled_features: resolveEnabledFeatures(
            data.sn_active_code,
            data.enabled_features
        ),
        sn_username:data.sn_user_name,
        sn_url:sn_url,
        sn_access_token:sn_access_token,
        bns_url: need_sn ? SN_BNS_API_URL : null,
        bns_evm: BNS_EVM_CONFIG,
        bns_evm_private_key: data.bns_evm_private_key || null
    });
    console.log("do_active result",result);
    return result["code"] == 0;
}
