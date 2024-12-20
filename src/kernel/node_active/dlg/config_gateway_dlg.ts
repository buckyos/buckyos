import templateContent from './config_gateway_dlg.template?raw';  
import {BuckyCheckBox} from '../components/checkbox';
import {BuckyWizzardDlg} from '../components/wizzard-dlg';
import { GatewayType,ActiveWizzardData,check_sn_active_code } from '../active_lib';
import {MdOutlinedTextField} from '@material/web/textfield/outlined-text-field.js';
import {MdFilledButton} from '@material/web/button/filled-button.js';
import Handlebars from 'handlebars';
import i18next from '../i18n';


Handlebars.registerHelper('t', function(key, options) {
    const params = options && options.hash || {};

    let result = i18next.t(key, params);
    console.log(key,result);
    return result;
});

class ConfigGatewayDlg extends HTMLElement {
    constructor() {
      super();
    }

    get_data_from_ui(wizzard_data:ActiveWizzardData) : boolean {
        let shadow : ShadowRoot | null = this.shadowRoot;
        if (!shadow) {
            return false;
        }
        const chk_enable_bucky_forward = shadow.getElementById('chk_enable_bucky_forward') as BuckyCheckBox;
        //const chk_enable_port_forward = shadow.getElementById('chk_enable_port_forward') as BuckyCheckBox;
        var txt_bucky_sn_token = shadow.getElementById('txt_bucky_sn_token') as MdOutlinedTextField;
        if (chk_enable_bucky_forward.checked) {
            if (txt_bucky_sn_token.error) {
                return false;
            }

            if (txt_bucky_sn_token.value.length < 8) {
                alert("邀请码长度必须大于8位");
                return false;
            }

            wizzard_data.sn_active_code = txt_bucky_sn_token.value;
            wizzard_data.sn_url = "http://web3.buckyos.io/kapi/sn";
            wizzard_data.sn_host = "web3.buckyos.io";
            wizzard_data.gatewy_type = GatewayType.BuckyForward;
        } else {
            wizzard_data.gatewy_type = GatewayType.PortForward;
            wizzard_data.sn_url = "";
            wizzard_data.sn_host = "";
            wizzard_data.is_direct_connect = true;
        }

        return true;
    }

    async check_bucky_sn_token(sn_token:string) : Promise<boolean> {
        let result = await check_sn_active_code(sn_token);
        return result;
    }

    connectedCallback() {
        const template = document.createElement('template');
        const template_compiled = Handlebars.compile(templateContent);
        const params = {}
        template.innerHTML = template_compiled(params);
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        const chk_enable_bucky_forward = shadow.getElementById('chk_enable_bucky_forward') as BuckyCheckBox;
        const chk_enable_port_forward = shadow.getElementById('chk_enable_port_forward') as BuckyCheckBox;
        const txt_bucky_sn_token = shadow.getElementById('txt_bucky_sn_token') as MdOutlinedTextField;

        txt_bucky_sn_token.addEventListener('input', (event) => {
            let sn_token: string = txt_bucky_sn_token.value;
            txt_bucky_sn_token.error = false;
            if (sn_token.length > 6) {
                this.check_bucky_sn_token(sn_token).then((is_ok) => {
                    if (!is_ok) {
                        txt_bucky_sn_token.error = true;
                        txt_bucky_sn_token.errorText = "邀请码有误";
                    }
                });
            }
        });

        chk_enable_bucky_forward.addEventListener('click', () => {
            chk_enable_port_forward.checked = !chk_enable_bucky_forward.checked;
        });

        chk_enable_port_forward.addEventListener('click', () => {
            chk_enable_bucky_forward.checked = !chk_enable_port_forward.checked;
        });

        const next_btn = shadow.getElementById('btn_next') as MdFilledButton;
        var activeWizzard = document.getElementById('active-wizzard') as BuckyWizzardDlg;
        next_btn.addEventListener('click', () => {
            
            if (!activeWizzard) {
                return;
            }
            let wizzard_data:ActiveWizzardData = activeWizzard.wizzard_data;
            if (this.get_data_from_ui(wizzard_data)) {
                let config_zone_id_dlg = document.createElement('config-zone-id-dlg');
                //console.log(activeWizzard,activeWizzard.wizzard_data);
                BuckyWizzardDlg;
                activeWizzard.pushDlg(config_zone_id_dlg);
            } 
        });
    }
}

customElements.define("config-gateway-dlg", ConfigGatewayDlg);
