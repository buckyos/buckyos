import buckyos from 'buckyos';
// 定义事件类型
export interface LoginEventDetail {
    login_result: any;
    timestamp: number;
}

export interface AccountInfo {
    user_name: string;
    user_id: string;
    user_type: string;
    session_token: string;
}
  
// 定义自定义事件
export const LOGIN_EVENT = 'onLogin';

export async function doLogin(username:string, password:string,appId:string,source_url:string) {
    let login_nonce = Date.now();
    let password_hash = await buckyos.AuthClient.hash_password(username,password,login_nonce);
    console.log("password_hash: ", password_hash);
    localStorage.removeItem("account_info");
    
    try {
        let verify_hub_url = buckyos.get_verify_rpc_url();
        let rpc_client = new buckyos.kRPCClient(verify_hub_url,null,login_nonce);
        let account_info = await rpc_client.call("login", {
            type: "password",
            username: username,
            password: password_hash,
            appid: appId,
            source_url:source_url
        });
        console.log("login result: ", account_info);    

        return account_info;
    } catch (error) {
        console.error("login failed: ", error);
        throw error;
    }
}

export function get_session_account_info(): AccountInfo | null {
    let account_info = localStorage.getItem("account_info");
    if (account_info == null) {
        return null;
    }

    return JSON.parse(account_info) as AccountInfo;
}