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

//after dom loaded
window.onload = async () => {
    let login_button = document.getElementById('btn-login') as MdOutlinedButton;
    login_button.onclick = () => {
        console.log("login");
        let authClient = new buckyos.AuthClient({
            ssoUrl: "http://127.0.0.1:8080",
            clientId: "control_panel",
            redirectUri: "http://127.0.0.1:8080/control_panel",
        });
        authClient.login().then((token) => {
            console.log("login success, token: ", token);
        }).catch((error) => {
            console.error("login failed, error: ", error);
        });
    }
}