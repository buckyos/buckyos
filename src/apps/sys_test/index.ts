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
import '@material/web/textfield/filled-text-field.js';
import { MdOutlinedButton } from '@material/web/button/outlined-button.js';
import buckyos from 'buckyos';


async function login() : Promise<string> {
    //zone host name是当前host的上一级
    let zone_host_name = window.location.hostname.split('.').slice(1).join('.');
    console.log("zone_host_name: ", zone_host_name);
    let auth_client = new buckyos.AuthClient(zone_host_name, "sys_test", null, null);
    let bucky_token = await auth_client.login();
    return bucky_token;
}

//after dom loaded
window.onload = async () => {
    let login_button = document.getElementById('btn-login') as MdOutlinedButton;
    login_button.onclick = () => {
        console.log("do login");
        login().then((bucky_token) => {
            console.log("bucky_token: ", bucky_token);
        });

    }
}