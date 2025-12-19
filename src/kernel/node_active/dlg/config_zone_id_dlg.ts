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
        const self_domain_setup = shadow.getElementById('self_domain_setup') as HTMLElement;
        const txt_dns_ns_tip = shadow.getElementById('txt_dns_ns_tip') as HTMLElement;
        const txt_boot_record = shadow.getElementById('txt_boot_record') as MdFilledTextField;
        const txt_pkx_record = shadow.getElementById('txt_pkx_record') as MdFilledTextField;
        const txt_dev_record = shadow.getElementById('txt_dev_record') as MdFilledTextField;
        const txt_records_container = shadow.getElementById('txt_records_container') as HTMLElement;
        const btn_generate_txt_records = shadow.getElementById('btn_generate_txt_records') as MdFilledButton;

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
            // 当域名改变时，隐藏已生成的TXT记录，需要重新生成
            if (chk_use_self_domain.checked) {
                txt_records_container.style.display = 'none';
                // 清空TXT记录内容
                txt_boot_record.value = '';
                txt_pkx_record.value = '';
                txt_dev_record.value = '';
            }
        });


        // 更新自有域名设置的显示/隐藏
        const updateSelfDomainSetupVisibility = () => {
            if (chk_use_self_domain.checked) {
                self_domain_setup.style.display = 'block';
                // 更新NS记录提示文本
                const sn_host_base = wizzard_data.web3_base_host || 'web3.buckyos.ai';
                txt_dns_ns_tip.textContent = i18next.t('dns_ns_record', { sn_host_base: sn_host_base }) || `设置NS记录为 sn.${sn_host_base}`;
                // 隐藏TXT记录容器，等待用户点击按钮生成
                txt_records_container.style.display = 'none';
            } else {
                self_domain_setup.style.display = 'none';
                txt_records_container.style.display = 'none';
            }
        };

        // 更新TXT记录
        const updateTxtRecords = async () => {
            if (!wizzard_data.owner_private_key || wizzard_data.owner_private_key === '') {
                alert(i18next.t('error_private_key_not_ready') || '私钥尚未生成，请稍候再试');
                return;
            }

            // 检查域名是否已输入
            if (!txt_domain.value || txt_domain.value.trim() === '') {
                txt_domain.error = true;
                txt_domain.errorText = i18next.t('error_domain_required') || '请先输入域名';
                return;
            }

            // 禁用按钮，显示加载状态
            btn_generate_txt_records.disabled = true;
            btn_generate_txt_records.textContent = i18next.t('generating_txt_records') || '正在生成...';

            try {
                // 生成BOOT记录
                const sn_url = new URL(wizzard_data.sn_url || 'https://sn.buckyos.ai');
                const boot_jwt = await generate_zone_boot_config_jwt(sn_url.hostname, wizzard_data.owner_private_key);
                txt_boot_record.value = `DID=${boot_jwt};`;
                wizzard_data.zone_config_jwt = boot_jwt;

                // PKX和DEV记录暂时显示占位符，等待SN生成
                // 这些记录应该由SN自动生成，这里先显示提示信息
                txt_pkx_record.value = i18next.t('txt_record_placeholder') || '(请等待SN生成)';
                txt_dev_record.value = i18next.t('txt_record_placeholder') || '(请等待SN生成)';

                // 显示TXT记录容器
                txt_records_container.style.display = 'block';
            } catch (err) {
                console.error('Failed to generate TXT records:', err);
                alert(i18next.t('error_generate_txt_records_failed') || '生成TXT记录失败，请重试');
                txt_boot_record.value = i18next.t('txt_record_placeholder') || '(生成失败)';
                txt_pkx_record.value = i18next.t('txt_record_placeholder') || '(生成失败)';
                txt_dev_record.value = i18next.t('txt_record_placeholder') || '(生成失败)';
            } finally {
                // 恢复按钮状态
                btn_generate_txt_records.disabled = false;
                btn_generate_txt_records.textContent = i18next.t('generate_txt_records_button') || '生成TXT记录';
            }
        };

        // 添加生成TXT记录按钮的点击事件
        btn_generate_txt_records.addEventListener('click', async (event) => {
            await updateTxtRecords();
        });

        chk_use_buckyos_name.addEventListener('click', (event) => {
            chk_use_self_domain.checked = !chk_use_buckyos_name.checked;
            updateSelfDomainSetupVisibility();
        });

        chk_use_self_domain.addEventListener('click', (event) => {
            chk_use_buckyos_name.checked = !chk_use_self_domain.checked;
            updateSelfDomainSetupVisibility();
        });

        // 初始化显示状态
        updateSelfDomainSetupVisibility();

        // 为TXT记录添加复制功能
        const addCopyFunction = (element: MdFilledTextField, label: string) => {
            element.addEventListener('click', (event) => {
                if (element.value && element.value !== i18next.t('txt_record_placeholder')) {
                    // 创建临时输入框
                    const tempInput = document.createElement('textarea');
                    tempInput.value = element.value;
                    document.body.appendChild(tempInput);
                    
                    // 选择并复制
                    tempInput.select();
                    document.execCommand('copy');
                    
                    // 移除临时元素
                    document.body.removeChild(tempInput);
                    
                    alert(i18next.t("success_copied"));
                }
            });
        };

        addCopyFunction(txt_boot_record, 'BOOT');
        addCopyFunction(txt_pkx_record, 'PKX');
        addCopyFunction(txt_dev_record, 'DEV');


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
