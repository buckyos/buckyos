import templateContent from './config_zone_id_dlg.template?raw';  

class ConfigZoneIdDlg extends HTMLElement {
    constructor() {
      super();
    }

    connectedCallback() {
        const template = document.createElement('template');
        template.innerHTML = templateContent;
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        this.shadowRoot.getElementById('copyButton').addEventListener('click', function(event) {
            event.preventDefault(); // 阻止默认行为
            var textToCopy = shadow.getElementById('txt_zone_id_value').textContent;
            navigator.clipboard.writeText(textToCopy).then(function() {
                alert('内容已复制到剪贴板');
            }).catch(function(err) {
                console.error('复制失败', err);
            });
        });

        this.shadowRoot.getElementById('btn_next').addEventListener('click', function(event) {
            event.preventDefault(); // 阻止默认行为
            const activeWizzard = document.getElementById('active-wizzard');
            var final_check_dlg = document.createElement('final-check-dlg');
            activeWizzard.pushDlg(final_check_dlg);
        });
    }

}
customElements.define("config-zone-id-dlg", ConfigZoneIdDlg);
