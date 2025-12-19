import templateContent from './final_check_dlg.template?raw';  
import { WizzardDlg } from '../components/wizzard-dlg';
import { MdFilledButton } from '@material/web/button/filled-button.js';
import { ActiveWizzardData,do_active } from '../active_lib';
import Handlebars from 'handlebars';
import i18next from '../i18n';

class FinalCheckDlg extends HTMLElement {
    constructor() {
      super();
    }

    async do_active(wizzard_data:ActiveWizzardData):Promise<boolean> {
        
        return await do_active(wizzard_data);
    }

    connectedCallback() {
        const wizzard_data = (document.getElementById('active-wizzard') as WizzardDlg).wizzard_data as ActiveWizzardData;

        const template = document.createElement('template');
        const template_compiled = Handlebars.compile(templateContent);
        template.innerHTML = template_compiled(wizzard_data);
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        //let txt_private_key = shadow.getElementById('txt_private_key') as HTMLElement;
        //txt_private_key.textContent = wizzard_data.owner_private_key;

        const next_btn = shadow.getElementById('btn_next') as MdFilledButton;
        next_btn.addEventListener('click', () => {
            next_btn.disabled = true;
            this.do_active(wizzard_data).then((result) => {
                next_btn.disabled = false;
                if (result) {
                    const activeWizzard = document.getElementById('active-wizzard') as WizzardDlg;
                    const active_result_dlg = document.createElement('active-result-dlg');
                    activeWizzard.pushDlg(active_result_dlg);
                    activeWizzard.disableBackButton();
                }
            }).catch((err) => {
                console.error('激活失败', err);
                next_btn.disabled = false;
                alert(i18next.t("error_activation_failed") + err);
            });
        });
    }

}
customElements.define("final-check-dlg", FinalCheckDlg);
