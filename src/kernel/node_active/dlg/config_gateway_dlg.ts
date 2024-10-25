import templateContent from './config_gateway_dlg.template?raw';  
import BuckyCheckBox from '../components/checkbox';
import WizzardDlg from '../components/wizzard-dlg/index';
import { GatewayType,ActiveWizzardData,check_sn_active_code } from '../active_lib';

class ConfigGatewayDlg extends HTMLElement {
    constructor() {
      super();
    }

    get_data_from_ui(shadow,wizzard_data:ActiveWizzardData) : boolean {
        const chk_enable_bucky_forward = shadow.getElementById('chk_enable_bucky_forward') as BuckyCheckBox;
        //const chk_enable_port_forward = shadow.getElementById('chk_enable_port_forward') as BuckyCheckBox;
        var txt_bucky_sn_token = shadow.getElementById('txt_bucky_sn_token');
        if (chk_enable_bucky_forward.checked) {
            if (txt_bucky_sn_token.error) {
                return false;
            }

            if (txt_bucky_sn_token.value.length < 8) {
                return false;
            }

            wizzard_data.sn_active_code = txt_bucky_sn_token.value;
            wizzard_data.gatewy_type = GatewayType.BuckyForward;
        } else {
            wizzard_data.gatewy_type = GatewayType.PortForward;
        }

        return true;
    }

    async check_bucky_sn_token(sn_token:string) : Promise<boolean> {
        let result = await check_sn_active_code(sn_token);
        return result;
    }

    connectedCallback() {
        const template = document.createElement('template');
        template.innerHTML = templateContent;
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        const chk_enable_bucky_forward = shadow.getElementById('chk_enable_bucky_forward') as BuckyCheckBox;
        const chk_enable_port_forward = shadow.getElementById('chk_enable_port_forward') as BuckyCheckBox;
        const txt_bucky_sn_token = shadow.getElementById('txt_bucky_sn_token');

        txt_bucky_sn_token.addEventListener('input', (event) => {
            var sn_token: string = txt_bucky_sn_token.value;
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

        

        const next_btn = shadow.getElementById('btn_next');
        next_btn.addEventListener('click', () => {
            const activeWizzard = document.getElementById('active-wizzard') as WizzardDlg;
            if (this.get_data_from_ui(shadow,activeWizzard.wizzard_data)) {
                
                var config_zone_id_dlg = document.createElement('config-zone-id-dlg');
                activeWizzard.pushDlg(config_zone_id_dlg);
            } else {
                alert("请输入正确的邀请码");
            }
        });
    }
}
customElements.define("config-gateway-dlg", ConfigGatewayDlg);
