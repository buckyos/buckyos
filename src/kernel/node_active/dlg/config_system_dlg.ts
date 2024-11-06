import templateContent from './config_system_dlg.template?raw';  
import { MdOutlinedTextField } from '@material/web/textfield/outlined-text-field.js';
import { MdFilledButton } from '@material/web/button/filled-button.js';
import {BuckyCheckBox} from '../components/checkbox/index';
import { WizzardDlg } from '../components/wizzard-dlg/index';
import { ActiveWizzardData} from '../active_lib';
import buckyos from 'buckyos';

class ConfigSystemDlg extends HTMLElement {
    constructor() {
      super();
    }

    async get_data_from_ui(wizzard_data:ActiveWizzardData) : Promise<boolean> {
        let shadow:ShadowRoot | null = this.shadowRoot;
        if (!shadow) {
            return false;
        }
        
        let txt_admin_password = shadow.getElementById('txt_admin_password') as MdOutlinedTextField;
        let txt_password_again = shadow.getElementById('txt_password_again') as MdOutlinedTextField;
        if (txt_admin_password.value != txt_password_again.value){
            txt_password_again.error = true; 
            txt_password_again.errorText = "两次输入的密码不一致";
            return false;
        }
        let txt_friend_code = shadow.getElementById('txt_friend_code') as MdOutlinedTextField;
        if (txt_admin_password.value.length < 8){
            txt_admin_password.error = true; 
            txt_admin_password.errorText = "密码长度不能小于8";
            return false;
        }

        
        if (txt_friend_code.value.length > 0){
            if (txt_friend_code.value.length < 6){
                txt_friend_code.error = true; 
                txt_friend_code.errorText = "好友访问码长度不能小于6";
                return false;
            }
        }
        wizzard_data.admin_password_hash = await buckyos.AuthClient.hash_password(wizzard_data.sn_user_name,txt_admin_password.value);
        wizzard_data.friend_passcode = txt_friend_code.value;
        let chk_enable_guest = shadow.getElementById('chk_enable_guest') as BuckyCheckBox;
        wizzard_data.enable_guest_access = chk_enable_guest.checked;
        console.log(wizzard_data);
        return true;
    }

    connectedCallback() {
        let wizzard_data:ActiveWizzardData = (document.getElementById('active-wizzard') as WizzardDlg).wizzard_data;
        
        const template = document.createElement('template');
        template.innerHTML = templateContent;
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        let txt_admin_password = shadow.getElementById('txt_admin_password') as MdOutlinedTextField;
        let txt_password_again = shadow.getElementById('txt_password_again') as MdOutlinedTextField;
        let txt_friend_code = shadow.getElementById('txt_friend_code') as MdOutlinedTextField;
        txt_admin_password.addEventListener('input', () => {
            txt_admin_password.error = false;
            txt_admin_password.errorText = "";
        });
        txt_password_again.addEventListener('input', () => {
            txt_password_again.error = false;
            txt_password_again.errorText = "";
        });
        txt_friend_code.addEventListener('input', () => {
            txt_friend_code.error = false;
            txt_friend_code.errorText = "";
        });


        const next_btn = shadow.getElementById('btn_next') as MdFilledButton;
        next_btn.addEventListener('click', () => {
            next_btn.disabled = true;
            this.get_data_from_ui(wizzard_data).then((result) => {
                next_btn.disabled = false;
                if (result){
                    const activeWizzard = document.getElementById('active-wizzard') as WizzardDlg;
                    var final_check_dlg = document.createElement('final-check-dlg');
                    activeWizzard.pushDlg(final_check_dlg);
                }
            });
        });
    }
}
customElements.define("config-system-dlg", ConfigSystemDlg);
