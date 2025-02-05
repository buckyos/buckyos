import '@material/web/icon/icon.js';
import '@material/web/iconbutton/icon-button.js';
import '@material/web/iconbutton/filled-icon-button.js';
import '@material/web/iconbutton/filled-tonal-icon-button.js';
import '@material/web/iconbutton/outlined-icon-button.js';

import '@material/web/button/filled-button.js';
import '@material/web/button/outlined-button.js';
import '@material/web/checkbox/checkbox.js';
import '@material/web/radio/radio.js';
import '@material/web/textfield/outlined-text-field.js';
import { MdOutlinedTextField } from '@material/web/textfield/outlined-text-field.js';
import '@material/web/textfield/filled-text-field.js';
import { MdOutlinedButton } from '@material/web/button/outlined-button.js';
import {buckyos} from 'buckyos';


async function doLogin(username:string, password:string,appId:string,source_url:string) {
    let login_nonce = Date.now();
    let password_hash = await buckyos.AuthClient.hash_password(username,password,login_nonce);
    console.log("password_hash: ", password_hash);

    try {
        let verify_hub_url = buckyos.get_verify_rpc_url();
        let rpc_client = new buckyos.kRPCClient(verify_hub_url,null,login_nonce);
        let result = await rpc_client.call("login", {
            type: "password",
            username: username,
            password: password_hash,
            appid: appId,
            source_url:source_url
        });
        return result;
    } catch (error) {
        console.error("login failed: ", error);
        throw error;
    }
}

//after dom loaded
window.onload = async () => {
    const parsedUrl = new URL(window.location.href);
    const source_url = document.referrer;
    
    var url_appid:string|null = parsedUrl.searchParams.get('client_id');
    //console.log("url_appid: ", url_appid);
    await buckyos.initBuckyOS(url_appid);
    //console.log(zone_host);

    if (url_appid == null) {
       alert("client_id(appid) is null");
       window.close();
       return;
    }

    (document.getElementById('appid') as HTMLSpanElement).innerText = url_appid;

    let login_button = document.getElementById('btn-login') as MdOutlinedButton;
    login_button.onclick = () => {

        let username = (document.getElementById('txt-username') as MdOutlinedTextField).value;
        if (username == null || username == "") {
            alert("username is null");
            return;
        }
        let password = (document.getElementById('txt-password') as MdOutlinedTextField).value;
        if (password == null || password == "") {
            alert("password is null");
            return;
        }
        login_button.disabled = true;
        //console.log("do login");
        
        buckyos.doLogin(username, password).then((account_info) => {
            let account_info_json = JSON.stringify(account_info);
            window.opener.postMessage({ token: account_info_json }, '*');
            window.close();
        })
        .catch((error) => {
            alert("login failed: " + error);
        })
        .finally(() => {
            login_button.disabled = false;
        });
    }
}
