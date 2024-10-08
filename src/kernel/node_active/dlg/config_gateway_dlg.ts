import templateContent from './config_gateway_dlg.template?raw';  

class ConfigGatewayDlg extends HTMLElement {
    constructor() {
      super();
    }

    connectedCallback() {
        const template = document.createElement('template');
        template.innerHTML = templateContent;
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        const next_btn = shadow.getElementById('btn_next');
        next_btn.addEventListener('click', () => {
            const activeWizzard = document.getElementById('active-wizzard');
            var config_zone_id_dlg = document.createElement('config-zone-id-dlg');
            activeWizzard.pushDlg(config_zone_id_dlg);
        });
    }
}
customElements.define("config-gateway-dlg", ConfigGatewayDlg);
