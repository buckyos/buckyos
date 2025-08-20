import templateContent from './config_zone_id_dlg.template?raw';  
import {BuckyCheckBox} from '../components/checkbox/index';
import {WizzardDlg} from '../components/wizzard-dlg/index';
import {MdOutlinedTextField} from '@material/web/textfield/outlined-text-field.js';
import {MdFilledButton} from '@material/web/button/filled-button.js';
import {MdFilledTextField} from '@material/web/textfield/filled-text-field.js';
import { GatewayType,ActiveWizzardData,generate_key_pair,check_bucky_username,isValidDomain,generate_zone_boot_config_jwt,check_sn_active_code } from '../active_lib';
import i18next, { waitForI18n } from '../i18n';
import Handlebars from 'handlebars';

class ConfigZoneIdDlg extends HTMLElement {
    constructor() {
      super();
    }


    async get_data_from_ui(wizzard_data:ActiveWizzardData) : Promise<boolean> {
        let shadow : ShadowRoot | null = this.shadowRoot;
        if (!shadow) {
            return false;
        }

        const chk_use_buckyos_name = shadow.getElementById('chk_use_buckyos_name') as BuckyCheckBox;
        const chk_use_self_domain = shadow.getElementById('chk_use_self_name') as BuckyCheckBox;
        const txt_name = shadow.getElementById('txt_name') as MdOutlinedTextField;
        const txt_domain = shadow.getElementById('txt_domain') as MdOutlinedTextField;
        const txt_bucky_sn_token = shadow.getElementById('txt_bucky_sn_token') as MdOutlinedTextField;
        
        if (chk_use_buckyos_name.checked) {
            if (txt_name.error) {
                return false;
            }
            if (txt_name.value.length <= 4) {
                txt_name.error = true;
                txt_name.errorText = i18next.t("error_name_too_short");
                return false;
            }
            if (txt_bucky_sn_token.error) {
                return false;
            }
            wizzard_data.sn_user_name = txt_name.value;
            wizzard_data.use_self_domain = false;
        } else {
            if (txt_domain.error) {
                return false;
            }
            if (!isValidDomain(txt_domain.value)) {
                txt_domain.error = true;
                txt_domain.errorText = i18next.t("error_domain_format");
                return false;
            }
            wizzard_data.self_domain = txt_domain.value;
            wizzard_data.use_self_domain = true;
        }

        return true;
    }

    connectedCallback() {
        const activeWizzard = document.getElementById('active-wizzard') as WizzardDlg;
        var wizzard_data:ActiveWizzardData = activeWizzard.wizzard_data;
        if(wizzard_data.owner_public_key == ""){
            console.log("generate owner key pair");
            generate_key_pair().then((data) => {
                wizzard_data.owner_public_key = data[0];
                wizzard_data.owner_private_key = data[1];
                console.log("generate owner key pair success",wizzard_data.owner_public_key,wizzard_data.owner_private_key);
            }).catch((err) => {
                console.error("generate owner key pair error",err);
            });
        }

        const template = document.createElement('template');
        const template_compiled = Handlebars.compile(templateContent);
        const params = {
            use_buckyos_domain: i18next.t("use_buckyos_domain"),
            use_own_domain: i18next.t("use_own_domain")
        }
        template.innerHTML = template_compiled(params);
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        const chk_use_buckyos_name = shadow.getElementById('chk_use_buckyos_name') as BuckyCheckBox;
        const chk_use_self_domain = shadow.getElementById('chk_use_self_name') as BuckyCheckBox;
        const txt_bucky_sn_token = shadow.getElementById('txt_bucky_sn_token') as MdOutlinedTextField;
        const txt_name = shadow.getElementById('txt_name') as MdOutlinedTextField;
        const txt_domain = shadow.getElementById('txt_domain') as MdOutlinedTextField;
        const btn_next = shadow.getElementById('btn_next') as MdFilledButton;

        if (wizzard_data.sn_active_code) {
            if(wizzard_data.sn_active_code.length > 0){
                txt_bucky_sn_token.value = wizzard_data.sn_active_code;
            }
        }


        txt_name.addEventListener('input', (event) => {
            txt_name.error = false;
            txt_name.errorText = "";
            let new_name = txt_name.value;
            if (new_name.length > 4) {
                if (wizzard_data.gatewy_type == GatewayType.BuckyForward){
                    check_bucky_username(txt_name.value).then((result) => {
                        if (!result){
                            txt_name.error = true;
                            txt_name.errorText = i18next.t("error_name_taken");
                        }
                    });
                }
                const sn_url = new URL(wizzard_data.sn_url);

                generate_zone_boot_config_jwt(sn_url.hostname,wizzard_data.owner_private_key).then((zone_config_jwt) => {
                    let txt_zone_config = shadow.getElementById('txt_zone_id_value') as MdFilledTextField;
                    txt_zone_config.value = "DID="+zone_config_jwt+";";
                    wizzard_data.zone_config_jwt = zone_config_jwt;
                });
            }
        });

        txt_bucky_sn_token.addEventListener('input', (event) => {
            txt_bucky_sn_token.error = false;
            txt_bucky_sn_token.errorText = "";
            let sn_token = txt_bucky_sn_token.value;
            if (sn_token.length > 6) {
                check_sn_active_code(sn_token).then((is_ok) => {
                    if (!is_ok) {
                        txt_bucky_sn_token.error = true;
                        txt_bucky_sn_token.errorText = i18next.t("error_invite_code_invalid");
                    }
                });
            }
        });

        txt_domain.addEventListener('change', (event) => {
            txt_domain.error = false;
            txt_domain.errorText = "";
        });


        chk_use_buckyos_name.addEventListener('click', (event) => {
            chk_use_self_domain.checked = !chk_use_buckyos_name.checked;
        });

        chk_use_self_domain.addEventListener('click', (event) => {
            chk_use_buckyos_name.checked = !chk_use_self_domain.checked;
        });

        let copyButton = shadow.getElementById('copyButton') as HTMLAnchorElement;
        copyButton.addEventListener('click', (event) => {
            // 创建临时输入框
            const tempInput = document.createElement('textarea');
            let txt_zone_config = shadow.getElementById('txt_zone_id_value') as MdFilledTextField;
            const textToCopy = txt_zone_config.value;
            tempInput.value = textToCopy;
            document.body.appendChild(tempInput);
            
            // 选择并复制
            tempInput.select();
            document.execCommand('copy');
            
            // 移除临时元素
            document.body.removeChild(tempInput);
            
            alert(i18next.t("success_copied"));
        });


        btn_next.addEventListener('click', (event) => {
            //event.preventDefault();
            btn_next.disabled = true;
            this.get_data_from_ui(wizzard_data).then((result) => {
                btn_next.disabled = false;
                if (result) {
                    var config_system_dlg = document.createElement('config-system-dlg');
                    activeWizzard.pushDlg(config_system_dlg);
                }
            }).catch((err) => {
                btn_next.disabled = false;
                alert(err);
            });
        });
    }

}
customElements.define("config-zone-id-dlg", ConfigZoneIdDlg);
