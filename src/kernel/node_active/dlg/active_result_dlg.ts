import templateContent from './active_result_dlg.template?raw';  
import { WizzardDlg } from '../components/wizzard-dlg';
import { MdFilledButton } from '@material/web/button/filled-button.js';
import { ActiveWizzardData,do_active } from '../active_lib';
import Handlebars from 'handlebars';

class ActiveResultDlg extends HTMLElement {
    constructor() {
      super();
    }

    connectedCallback() {
      const wizzard_data = (document.getElementById('active-wizzard') as WizzardDlg).wizzard_data as ActiveWizzardData;

      const template = document.createElement('template');
      const template_compiled = Handlebars.compile(templateContent);
      template.innerHTML = template_compiled(wizzard_data);
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        let btn_end = shadow.getElementById('btn_end') as MdFilledButton;
        let target_url = `http://${wizzard_data.sn_user_name}.web3.buckyos.io/`;
        if (wizzard_data.use_self_domain) {
            target_url = `http://${wizzard_data.self_domain}/`;
        }
        
        btn_end.addEventListener('click',() => {
            alert("即将跳转到您的Personal Server主页,默认用户名密码是admin/admin");
            window.location.href = target_url
        });
    }

}
customElements.define("active-result-dlg", ActiveResultDlg);
