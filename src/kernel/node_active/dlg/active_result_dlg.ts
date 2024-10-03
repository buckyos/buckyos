import templateContent from './active_result_dlg.template?raw';  

class ActiveResultDlg extends HTMLElement {
    constructor() {
      super();
    }

    connectedCallback() {
        const template = document.createElement('template');
        template.innerHTML = templateContent;
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

    }

}
customElements.define("active-result-dlg", ActiveResultDlg);
