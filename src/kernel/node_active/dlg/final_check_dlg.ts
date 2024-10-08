import templateContent from './final_check_dlg.template?raw';  

class FinalCheckDlg extends HTMLElement {
    constructor() {
      super();
    }

    connectedCallback() {
        const template = document.createElement('template');
        template.innerHTML = templateContent;
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        const copyButton = shadow.getElementById('copyButton');
        copyButton.addEventListener('click', () => {
            const privateKey = shadow.getElementById('txt_private_key').textContent;
            navigator.clipboard.writeText(privateKey).then(() => {
                alert('私钥已复制到剪贴板');
            }).catch(err => {
                console.error('复制失败', err);
            });
        });

        const next_btn = shadow.getElementById('btn_next');
        next_btn.addEventListener('click', () => {
            const activeWizzard = document.getElementById('active-wizzard');
            const active_result_dlg = document.createElement('active-result-dlg');
            activeWizzard.pushDlg(active_result_dlg);
        });
    }

}
customElements.define("final-check-dlg", FinalCheckDlg);
