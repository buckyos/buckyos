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
import './components/bs-title-bar.ts';
import {buckyos} from 'buckyos';
import {i18next, updateElementAndShadowRoots} from './i18n';

//after dom loaded
window.onload = async () => {
    console.log("index.html.ts onload");
    i18next.on('initialized', () => {
        updateElementAndShadowRoots(document);
    });

    await buckyos.initBuckyOS("control_panel");
    let account_info = await buckyos.login(true);
    if (account_info == null) {
        alert("请先登录");
        window.location.href = "./login_index.html";
        return;
    }
}