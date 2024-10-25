import templateContent from './active_result_dlg.template?raw';  
import { end_active } from '../active_lib';
import { ActiveWizzardData,do_active } from '../active_lib';
import Handlebars from 'handlebars';

class ActiveResultDlg extends HTMLElement {
    constructor() {
      super();
    }

    connectedCallback() {
      const wizzard_data = (document.getElementById('active-wizzard') as WizzardDlg).wizzard_data as ActiveWizzardData;

      const template = document.createElement('template');
      const compiled = Handlebars.compile(templateContent);
      template.innerHTML = compiled(wizzard_data);
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        let btn_end = shadow.getElementById('btn_end');
        btn_end.addEventListener('click',() => {
          btn_end.disabled = true;
          end_active().then((success) => {
            btn_end.disabled = false;
            if (success) {
              alert("启动成功,即将跳转到Personal Server");
              window.location.href = `http://${wizzard_data.sn_user_name}.web3.buckyos.io/index.html`;
            }
          });
        });
    }

}
customElements.define("active-result-dlg", ActiveResultDlg);
