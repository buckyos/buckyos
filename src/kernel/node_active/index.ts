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
import "./components/checkbox/index";
import {BuckyWizzardDlg} from './components/wizzard-dlg/index';

import "./dlg/config_gateway_dlg";
import "./dlg/config_zone_id_dlg";
import "./dlg/config_system_dlg";
import "./dlg/final_check_dlg";
import "./dlg/active_result_dlg";

import {GatewayType, ActiveWizzardData} from './active_lib';

//after dom loaded
window.onload = async () => {
    const wizzard_data : ActiveWizzardData = {
        is_direct_connect : false,
        sn_active_code : "",
        sn_user_name : "",
        gatewy_type : GatewayType.BuckyForward,
        use_self_domain : false,
        self_domain : "",
        admin_password_hash : "",
        friend_passcode : "",
        enable_guest_access : false,
        owner_public_key : "",
        owner_private_key : "",
        zone_config_jwt : "",
        sn_url : "",
        sn_host : "",
    }
    
    const activeWizzard = document.getElementById('active-wizzard') as BuckyWizzardDlg;
    activeWizzard.wizzard_data = wizzard_data;
    //activeWizzard.pushDlg(document.createElement('config-gateway-dlg'));
}