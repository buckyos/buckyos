import templateContent from './config_gateway_dlg.template?raw';  
import {BuckyCheckBox} from '../components/checkbox';
import {BuckyWizzardDlg} from '../components/wizzard-dlg';
import { GatewayType,ActiveWizzardData,check_sn_active_code,set_sn_api_url,SN_API_URL,WEB3_BASE_HOST } from '../active_lib';
import {MdOutlinedTextField} from '@material/web/textfield/outlined-text-field.js';
import {MdFilledButton} from '@material/web/button/filled-button.js';
import Handlebars from 'handlebars';
import i18next, { waitForI18n } from '../i18n';


Handlebars.registerHelper('t', function(key, options) {
    const params = options && options.hash || {};

    let result = i18next.t(key, params);
    //console.log(key,result);
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
        const chk_enable_port_forward = shadow.getElementById('chk_enable_port_forward') as BuckyCheckBox;
        var txt_bucky_sn_token = shadow.getElementById('txt_bucky_sn_token') as MdOutlinedTextField;
        var txt_bucky_sn_url = shadow.getElementById('txt_bucky_sn_url') as MdOutlinedTextField;
        const port_mapping_mode = shadow.getElementById('port_mapping_mode') as HTMLSelectElement;
        const txt_rtcp_port = shadow.getElementById('txt_rtcp_port') as MdOutlinedTextField;
        
        if (txt_bucky_sn_url.value.length > 0) {
            const url = new URL(txt_bucky_sn_url.value);
            const host = url.host;  // 包含端口号
            wizzard_data.sn_url = txt_bucky_sn_url.value;
            wizzard_data.web3_base_host = host;
        } else {
            wizzard_data.sn_url = SN_API_URL;
            wizzard_data.web3_base_host = WEB3_BASE_HOST;
        }
        set_sn_api_url(wizzard_data.sn_url);

        if (chk_enable_bucky_forward.checked) {
            if (txt_bucky_sn_token.error) {
                return false;
            }

            if (txt_bucky_sn_token.value.length < 8) {
                alert(i18next.t("error_invite_code_too_short"));
                return false;
            }

            wizzard_data.sn_active_code = txt_bucky_sn_token.value;
            wizzard_data.gatewy_type = GatewayType.BuckyForward;
        } else if (chk_enable_port_forward.checked) {
            wizzard_data.gatewy_type = GatewayType.PortForward;
            wizzard_data.is_direct_connect = true;
            // 根据端口映射模式设置相应的配置
            // port_mapping_mode.value 可以是 "full" 或 "rtcp_only"
            if (port_mapping_mode.value === 'rtcp_only') {
                // 读取RTCP端口值，如果没有输入则使用默认值2980
                const rtcp_port_str = txt_rtcp_port.value.trim();
                const rtcp_port = rtcp_port_str ? parseInt(rtcp_port_str, 10) : 2980;
                // 验证端口号范围
                if (isNaN(rtcp_port) || rtcp_port < 1 || rtcp_port > 65535) {
                    alert(i18next.t("error_invalid_port") || "RTCP端口号无效，请输入1-65535之间的数字");
                    return false;
                }
                // 这里可以根据需要将rtcp_port存储到wizzard_data中
                // 例如：wizzard_data.rtcp_port = rtcp_port;
            }
        } else {
            // 默认使用 BuckyForward
            wizzard_data.gatewy_type = GatewayType.BuckyForward;
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
        const params = {
            invite_code_placeholder: i18next.t("invite_code_placeholder"),
            custom_sn_placeholder: i18next.t("custom_sn_placeholder"),
            rtcp_port_placeholder: i18next.t("rtcp_port_placeholder")
        }
        template.innerHTML = template_compiled(params);
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        const chk_enable_bucky_forward = shadow.getElementById('chk_enable_bucky_forward') as BuckyCheckBox;
        const chk_enable_port_forward = shadow.getElementById('chk_enable_port_forward') as BuckyCheckBox;
        const txt_bucky_sn_token = shadow.getElementById('txt_bucky_sn_token') as MdOutlinedTextField;
        const direct_connect_options = shadow.getElementById('direct_connect_options') as HTMLElement;
        const port_mapping_mode = shadow.getElementById('port_mapping_mode') as HTMLSelectElement;
        const port_mapping_hint_text = shadow.getElementById('port_mapping_hint_text') as HTMLElement;
        const rtcp_port_container = shadow.getElementById('rtcp_port_container') as HTMLElement;
        const txt_rtcp_port = shadow.getElementById('txt_rtcp_port') as MdOutlinedTextField;

        // 更新端口映射提示信息
        const updatePortMappingHint = () => {
            if (port_mapping_mode.value === 'full') {
                port_mapping_hint_text.setAttribute('data-i18n', 'port_mapping_full_hint');
                const hintText = i18next.t('port_mapping_full_hint') || '所有的流量都不会通过SN中转并直接到达该设备';
                port_mapping_hint_text.textContent = hintText;
                port_mapping_hint_text.style.whiteSpace = 'normal';
                // 隐藏RTCP端口输入框
                rtcp_port_container.style.display = 'none';
            } else if (port_mapping_mode.value === 'rtcp_only') {
                port_mapping_hint_text.setAttribute('data-i18n', 'port_mapping_rtcp_only_hint');
                const hintText = i18next.t('port_mapping_rtcp_only_hint') || '来自浏览器的流量会通过SN中转\n来自客户端的rtcp流量可以直达该设备\n比如你在另一台笔记本上安装buckyos desktop并通过buckyos desktop访问该设备的服务，不需要经过SN中转。';
                // 将换行符转换为 <br> 标签
                const lines = hintText.split('\n');
                port_mapping_hint_text.innerHTML = lines.map(line => `<div>${line}</div>`).join('');
                port_mapping_hint_text.style.whiteSpace = 'normal';
                // 显示RTCP端口输入框
                rtcp_port_container.style.display = 'block';
            }
        };

        // 显示/隐藏直连模式选项
        const updateDirectConnectVisibility = () => {
            if (chk_enable_port_forward.checked) {
                direct_connect_options.style.display = 'block';
            } else {
                direct_connect_options.style.display = 'none';
            }
        };

        txt_bucky_sn_token.addEventListener('input', (event) => {
            let sn_token: string = txt_bucky_sn_token.value;
            txt_bucky_sn_token.error = false;
            if (sn_token.length > 6) {
                this.check_bucky_sn_token(sn_token).then((is_ok) => {
                    if (!is_ok) {
                        txt_bucky_sn_token.error = true;
                        txt_bucky_sn_token.errorText = i18next.t("error_invite_code_invalid");
                    }
                });
            }
        });

        chk_enable_bucky_forward.addEventListener('click', () => {
            chk_enable_port_forward.checked = !chk_enable_bucky_forward.checked;
            updateDirectConnectVisibility();
        });

        chk_enable_port_forward.addEventListener('click', () => {
            chk_enable_bucky_forward.checked = !chk_enable_port_forward.checked;
            updateDirectConnectVisibility();
        });

        port_mapping_mode.addEventListener('change', () => {
            updatePortMappingHint();
        });

        // 更新下拉列表选项的翻译文本
        const updateSelectOptions = () => {
            const options = port_mapping_mode.querySelectorAll('option');
            options.forEach(option => {
                const i18nKey = option.getAttribute('data-i18n');
                if (i18nKey) {
                    option.textContent = i18next.t(i18nKey) || option.textContent;
                }
            });
        };

        // 初始化
        updateDirectConnectVisibility();
        updateSelectOptions();
        updatePortMappingHint();

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
