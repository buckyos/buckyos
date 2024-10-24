import templateContent from './config_zone_id_dlg.template?raw';  
import BuckyCheckBox from '../components/checkbox';
import WizzardDlg from '../components/wizzard-dlg/index';
import { GatewayType,ActiveWizzardData,generate_key_pair,check_bucky_username,isValidDomain,generate_zone_config_jwt } from '../active_lib';

class ConfigZoneIdDlg extends HTMLElement {
    constructor() {
      super();
    }


    async get_data_from_ui(wizzard_data:ActiveWizzardData) : boolean {
        let shadow = this.shadowRoot;
        let chk_use_buckyos_name = shadow.getElementById('chk_use_buckyos_name') as BuckyCheckBox;
        let chk_use_self_name = shadow.getElementById('chk_use_self_name') as BuckyCheckBox;
        let txt_name = shadow.getElementById('txt_name');
        
        if (txt_name.value.length < 5) {
            txt_name.error = true;
            txt_name.errorText = "名字长度不能小于5";
            return false;
        }

        if (chk_use_buckyos_name.checked){
            
            let txt_bucky_sn_token = shadow.getElementById('txt_bucky_sn_token');
            if (!await check_bucky_username(txt_name.value)) {
                txt_name.error = true;
                txt_name.errorText = "名字已被使用";
                return false;
            }
            wizzard_data.sn_user_name = txt_name.value;
            wizzard_data.sn_active_code = txt_bucky_sn_token.value;
        }
       
        if (chk_use_self_name.checked){
            let txt_domain = shadow.getElementById('txt_domain');
            if (!isValidDomain(txt_domain.value)) {
                txt_domain.error = true;
                txt_domain.errorText = "域名格式不正确";
                return false;
            }
            wizzard_data.use_self_domain = true;
            wizzard_data.self_domain = txt_domain.value;
        }
        
        return true;
    }

    connectedCallback() {
        const activeWizzard = document.getElementById('active-wizzard') as WizzardDlg;
        var wizzard_data:ActiveWizzardData = activeWizzard.wizzard_data;
        if(wizzard_data.owner_public_key != ""){
            console.log("generate owner key pair");
            generate_key_pair().then((data) => {
                wizzard_data.owner_public_key = data[0];
                wizzard_data.owner_private_key = data[1];
                console.log("generate owner key pair success",wizzard_data.owner_public_key,wizzard_data.owner_private_key);
            });
        }

        const template = document.createElement('template');
        template.innerHTML = templateContent;
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        const chk_use_buckyos_name = shadow.getElementById('chk_use_buckyos_name') as BuckyCheckBox;
        const chk_use_self_domain = shadow.getElementById('chk_use_self_name') as BuckyCheckBox;
        const txt_bucky_sn_token = shadow.getElementById('txt_bucky_sn_token');
        const txt_name = shadow.getElementById('txt_name');
        const txt_domain = shadow.getElementById('txt_domain');

        txt_name.addEventListener('change', (event) => {
            txt_name.error = false;
            txt_name.errorText = "";
            let new_name = txt_name.value;
            if (new_name.length > 6) {
                let sn = "";
                if (wizzard_data.gatewy_type == GatewayType.BuckyForward){
                    sn = "web3.buckyos.io"   
                }
                
                generate_zone_config_jwt(txt_name.value,sn,wizzard_data.owner_private_key).then((zone_config_jwt) => {
                        shadow.getElementById('txt_zone_id_value').textContent = "DID="+zone_config_jwt+";";
                        wizzard_data.zone_config_jwt = zone_config_jwt;
                });
            }
        });

        txt_domain.addEventListener('change', (event) => {
            txt_domain.error = false;
            txt_domain.errorText = "";
        });
        if (wizzard_data.sn_active_code) {
            if(wizzard_data.sn_active_code.length > 0){
                txt_bucky_sn_token.value = wizzard_data.sn_active_code;
            }
        }

        chk_use_buckyos_name.addEventListener('click', (event) => {
            chk_use_self_domain.checked = !chk_use_buckyos_name.checked;
        });

        chk_use_self_domain.addEventListener('click', (event) => {
            chk_use_buckyos_name.checked = !chk_use_self_domain.checked;
        });


        this.shadowRoot.getElementById('copyButton').addEventListener('click', (event) => {
            event.preventDefault(); // 阻止默认行为
            var textToCopy = shadow.getElementById('txt_zone_id_value').textContent;
            navigator.clipboard.writeText(textToCopy).then(function() {
                alert('内容已复制到剪贴板');
            }).catch(function(err) {
                console.error('复制失败', err);
            });
        });
        let btn_next = this.shadowRoot.getElementById('btn_next');
        btn_next.addEventListener('click', (event) => {
            event.preventDefault();
            btn_next.disabled = true;
            this.get_data_from_ui(wizzard_data).then((result) => {
                btn_next.disabled = false;
                if (result) {
                    var config_system_dlg = document.createElement('config-system-dlg');
                    activeWizzard.pushDlg(config_system_dlg);
                }
            });
        });
    }

}
customElements.define("config-zone-id-dlg", ConfigZoneIdDlg);
