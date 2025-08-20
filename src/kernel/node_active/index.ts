

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
import "./components/language-switcher";
import {BuckyWizzardDlg} from './components/wizzard-dlg/index';

import "./dlg/config_gateway_dlg";
import "./dlg/config_zone_id_dlg";
import "./dlg/config_system_dlg";
import "./dlg/final_check_dlg";
import "./dlg/active_result_dlg";

import {GatewayType, ActiveWizzardData,SN_API_URL,set_sn_api_url} from './active_lib';
import i18next, { waitForI18n } from './i18n';
import Handlebars from 'handlebars';

function update_i18n() {

    function updateElementAndShadowRoots(root: Document | Element | ShadowRoot) {

        root.querySelectorAll('[data-i18n]').forEach(element => {
            const key = element.getAttribute('data-i18n');
            const options = element.getAttribute('data-i18n-options');
            
            if (key?.startsWith('[html]')) {
                const actualKey = key.replace('[html]', '');
                const translatedText = i18next.t(actualKey, JSON.parse(options || '{}'));
                element.innerHTML = typeof translatedText === 'string' ? translatedText : String(translatedText);
            } else if (key) {
                const translatedText = i18next.t(key, JSON.parse(options || '{}'));
                element.textContent = typeof translatedText === 'string' ? translatedText : String(translatedText);
            }
        });

        // 更新placeholder
        root.querySelectorAll('[data-i18n-placeholder]').forEach(element => {
            const key = element.getAttribute('data-i18n-placeholder');
            if (key) {
                const translatedText = i18next.t(key);
                if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
                    element.placeholder = typeof translatedText === 'string' ? translatedText : String(translatedText);
                }
            }
        });

        // 更新label
        root.querySelectorAll('[data-i18n-label]').forEach(element => {
            const key = element.getAttribute('data-i18n-label');
            if (key) {
                const translatedText = i18next.t(key);
                if (element instanceof HTMLElement && 'label' in element) {
                    (element as any).label = typeof translatedText === 'string' ? translatedText : String(translatedText);
                }
            }
        });

        // 更新value
        root.querySelectorAll('[data-i18n-value]').forEach(element => {
            const key = element.getAttribute('data-i18n-value');
            if (key) {
                const translatedText = i18next.t(key);
                if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
                    element.value = typeof translatedText === 'string' ? translatedText : String(translatedText);
                }
            }
        });

        root.querySelectorAll('*').forEach(element => {
            if (element.shadowRoot) {
                updateElementAndShadowRoots(element.shadowRoot);
            }
        });
    }

    // 从文档根节点开始遍历
    updateElementAndShadowRoots(document);
}

//after dom loaded
window.onload = async () => {
    // 等待i18n初始化完成
    await waitForI18n();
    
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
        sn_url : SN_API_URL,
        web3_base_host : "",
    }
    
    const activeWizzard = document.getElementById('active-wizzard') as BuckyWizzardDlg;
    activeWizzard.pushDlg(document.createElement('config-gateway-dlg'));
    activeWizzard.wizzard_data = wizzard_data;

    i18next.on('initialized', function(options:any) {
        update_i18n();
        //test i18n alert
        //const i18n_text = i18next.t("alert_text");
        //alert(i18n_text);
    });

    // 监听语言切换事件
    i18next.on('languageChanged', function(lng: string) {
        console.log('Language changed to:', lng);
        update_i18n();
    });

    // 监听自定义语言切换事件
    window.addEventListener('languageChanged', function(event: Event) {
        const customEvent = event as CustomEvent;
        console.log('Custom language change event received:', customEvent.detail);
        update_i18n();
        
    });


}