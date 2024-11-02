import nacl from 'tweetnacl';
import nacl_util from 'tweetnacl-util';
import buckyos from 'buckyos';
nacl.util = nacl_util;

console.log(buckyos);

// 首先需要引入必要的库
// <script src="https://cdnjs.cloudflare.com/ajax/libs/tweetnacl-js/1.0.3/nacl.min.js"></script>
// <script src="https://cdnjs.cloudflare.com/ajax/libs/tweetnacl-util/0.15.1/nacl-util.min.js"></script>
 
class BrowserEdDSAJWT {
    constructor() {
        this.encoder = new TextEncoder();
    }

    // 生成密钥对
    generateKeyPair() {
        const keyPair = nacl.sign.keyPair();
        return {
            publicKey: nacl.util.encodeBase64(keyPair.publicKey),
            privateKey: nacl.util.encodeBase64(keyPair.secretKey)
        };
    }

    // Base64URL 编码
    base64UrlEncode(str) {
        return str.replace(/\+/g, '-')
                  .replace(/\//g, '_')
                  .replace(/=/g, '');
    }

    // 创建 JWT header
    createHeader() {
        const header = {
            alg: 'EdDSA',
            typ: 'JWT'
        };
        return this.base64UrlEncode(btoa(JSON.stringify(header)));
    }

    // 创建 JWT payload
    createPayload(claims) {
        return this.base64UrlEncode(btoa(JSON.stringify(claims)));
    }

    // 签名 JWT
    sign(message, privateKey) {
        const secretKey = nacl.util.decodeBase64(privateKey);
        const messageUint8 = this.encoder.encode(message);
        const signature = nacl.sign.detached(messageUint8, secretKey);
        return this.base64UrlEncode(nacl.util.encodeBase64(signature));
    }

    // 验证签名
    verify(message, signature, publicKey) {
        try {
            const signatureUint8 = nacl.util.decodeBase64(signature);
            const messageUint8 = this.encoder.encode(message);
            const publicKeyUint8 = nacl.util.decodeBase64(publicKey);
            return nacl.sign.detached.verify(messageUint8, signatureUint8, publicKeyUint8);
        } catch (e) {
            console.log(e);
            return false;
        }
    }

    // 创建 JWT
    createJWT(payload, privateKey) {
        const header = this.createHeader();
        const encodedPayload = this.createPayload(payload);
        const message = `${header}.${encodedPayload}`;
        const signature = this.sign(message, privateKey);
        return `${message}.${signature}`;
    }

    // 验证 JWT
    verifyJWT(token, publicKey) {
        const parts = token.split('.');
        if (parts.length !== 3) {
            throw new Error('Invalid token format');
        }

        const [header, payload, signature] = parts;
        const message = `${header}.${payload}`;

        // 验证签名
        if (!this.verify(message, signature, publicKey)) {
            throw new Error('Invalid signature');
        }

        // 解码载荷
        const decodedPayload = JSON.parse(atob(payload.replace(/-/g, '+').replace(/_/g, '/')));
        
        // 验证过期时间
        if (decodedPayload.exp && decodedPayload.exp < Date.now() / 1000) {
            throw new Error('Token has expired');
        }

        return {
            header: JSON.parse(atob(header.replace(/-/g, '+').replace(/_/g, '/'))),
            payload: decodedPayload
        };
    }
}

// 使用示例
export async function demo_jwt() {
    const jwt = new BrowserEdDSAJWT();
    
    // 生成密钥对
    const { publicKey, privateKey } = jwt.generateKeyPair();
    console.log('Public Key:', publicKey);
    console.log('Private Key:', privateKey);

    // 创建 JWT
    const payload = {
        my_test_name: true,
        exp: Math.floor(Date.now() / 1000) + 7200 // 2小时后过期
    };

    const token = jwt.createJWT(payload, privateKey);
    console.log('JWT:', token);

    // 验证 JWT
    try {
        const verified = jwt.verifyJWT(token, publicKey);
        console.log('JWT 验证成功');
        console.log('Header:', verified.header);
        console.log('Payload:', verified.payload);
    } catch (error) {
        console.error('JWT 验证失败:', error.message);
    }
}


export enum GatewayType {
    BuckyForward = "BuckyForward",
    PortForward = "PortForward",
}

type JsonValue = Record<string, any>;

export type ActiveWizzardData = {
    sn_active_code : string;
    sn_user_name : string;
    gatewy_type : GatewayType;
    use_self_domain : boolean;
    self_domain : string;
    admin_password_hash : string;
    friend_passcode:string;
    enable_guest_access : boolean;

    owner_public_key : string;
    owner_private_key : string;
    zone_config_jwt : string;
    sn_url :string;
    sn_host : string;

}

export async function register_sn_user(user_name:string,active_code:string,public_key:string,zone_config_jwt:string) : Promise<boolean> {
    let rpc_client = new buckyos.kRPCClient("http://web3.buckyos.io/kapi/sn");
    let result = await rpc_client.call("register_user",{user_name:user_name,active_code:active_code,public_key:public_key,zone_config:zone_config_jwt});
    let code = result["code"];
    if (code == 0) {
        return true;
    }
    return false;
}

export async function register_sn_main_ood (user_name:string,device_name:string,device_did:string,device_ip:string,device_info:string) : Promise<boolean> {
    let rpc_client = new buckyos.kRPCClient("http://web3.buckyos.io/kapi/sn");
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
    let rpc_client = new buckyos.kRPCClient("http://web3.buckyos.io/kapi/sn");
    let result = await rpc_client.call("check_active_code",{active_code:sn_active_code});
    let valid = result["valid"];
    return valid;
}

export async function check_bucky_username(check_bucky_username:string) : Promise<boolean> {
    let rpc_client = new buckyos.kRPCClient("http://web3.buckyos.io/kapi/sn");
    let result = await rpc_client.call("check_username",{username:check_bucky_username});
    let valid = result["valid"];
    return valid;
}

export async function generate_key_pair():Promise<[string,JsonValue]> {
    let rpc_client = new buckyos.kRPCClient("/kapi/active");
    let result = await rpc_client.call("generate_key_pair",{});
    console.log(result);
    let public_key = result["public_key"]
    let private_key = result["private_key"]
    return [public_key,private_key];
}

export async function generate_zone_config_jwt(zone_short_id:string,sn:string,owner_private_key:string):Promise<string> {
    let rpc_client = new buckyos.kRPCClient("/kapi/active");
    const now = Math.floor(Date.now() / 1000);
    let zone_config = {
        did: "did:bns:"+zone_short_id,
        oods: ["ood1"],
        sn: sn,
        exp: now + 3600*24*365*10, 
    };
    let zoen_config_str =  JSON.stringify(zone_config);
    let result = await rpc_client.call("generate_zone_config",{
        zone_config:zoen_config_str,
        private_key:owner_private_key   
    });
    let zone_config_jwt = result["zone_config_jwt"];
    return zone_config_jwt;
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

export async function active_ood(user_name:string,zone_name:string,gateway_type:GatewayType,
    owner_public_key:string,owner_private_key:string,device_public_key:JsonValue,device_private_key:string,
    admin_password_hash:string,enable_guest_access:boolean,friend_passcode:string):Promise<boolean> {
    let rpc_client = new buckyos.kRPCClient("/kapi/active");
    let result = await rpc_client.call("do_active",{
        user_name:user_name,
        zone_name:zone_name,
        gateway_type:gateway_type,
        public_key:owner_public_key,
        private_key:owner_private_key,
        device_public_key:device_public_key,
        device_private_key:device_private_key,
        admin_password_hash:admin_password_hash,
        guest_access:enable_guest_access,
        friend_passcode:friend_passcode,
        sn_url:"http://web3.buckyos.io/kapi/sn",
        sn_host:"web3.buckyos.io"
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

    // register sn user
    let register_sn_user_result = await register_sn_user(
        data.sn_user_name,
        data.sn_active_code,
        JSON.stringify(data.owner_public_key),
        data.zone_config_jwt);
    if (!register_sn_user_result) {
        return false;
    }
    let zone_name = data.use_self_domain ? data.self_domain : data.sn_user_name + ".web3.buckyos.io";
    //switch (data.gateway_type) {
    //    case GatewayType.BuckyForward:
            // do_aactive at ood
    //}
            // do_aactive at ood
    let active_ood_result = await active_ood(
        data.sn_user_name,
        zone_name,
        data.gatewy_type,
        data.owner_public_key,
        data.owner_private_key,
        device_public_key,
        device_private_key,
        data.admin_password_hash,
        data.enable_guest_access,
        data.friend_passcode
    );
    if (!active_ood_result) {
        return false;
    }

    //get device info
    //let device_info = await get_thisdevice_info();
    // register ood to sn(move to active ood implement?)
    //let register_ood_result =await register_sn_main_ood(
    //    data.sn_user_name,
    //    "ood1",
    //    device_did,
    //    device_info["ip"],
    //    JSON.stringify(device_info));
    //if (!register_ood_result) {
    //    return false;
    //}
    return true;
}

