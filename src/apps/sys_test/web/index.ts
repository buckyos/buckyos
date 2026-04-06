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
import {buckyos} from 'buckyos';


async function login() : Promise<any> {
    //zone host name是当前host的上一级
    let account_info = await buckyos.login();
    if (!account_info) {
        throw new Error("login failed");
    }
    return account_info;
}

//after dom loaded
window.onload = async () => {
    await buckyos.initBuckyOS("sys-test");

    let login_button = document.getElementById('btn-login') as MdOutlinedButton;
    login_button.onclick = () => {
        console.log("do login");
        login().then((account_info) => {
            console.log("account_info: ", account_info);
        });

    }
}