import templateContent from './active_result_dlg.template?raw';  
import { end_active } from '../active_lib';

class ActiveResultDlg extends HTMLElement {
    constructor() {
      super();
    }

    connectedCallback() {
        const template = document.createElement('template');
        template.innerHTML = templateContent;
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        let btn_end = shadow.getElementById('btn_end');
        btn_end.addEventListener('click',() => {
          btn_end.disabled = true;
          end_active().then((success) => {
            btn_end.disabled = false;
            if (success) {
              alert("Active end");
            }
          });
        });
    }

}
customElements.define("active-result-dlg", ActiveResultDlg);
