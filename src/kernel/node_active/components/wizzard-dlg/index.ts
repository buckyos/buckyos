import templateContent from './wizzard-dlg.template?raw';
import {MdOutlinedIconButton} from '@material/web/iconbutton/outlined-icon-button';

//该组件，可以往里push dlg(另一个component).当有dlg时，左上角有back按钮。每次push时，当前的dlg会往左淡出，新的dlg从右边进场。 

export class BuckyWizzardDlg extends HTMLElement {
    private dlgStack: HTMLElement[] = [];
    public wizzard_data: any = {};

    constructor() {
      super();
    }

    connectedCallback() {
      const template = document.createElement('template');
      template.innerHTML = templateContent;
      const shadow = this.attachShadow({ mode: 'open' });
      shadow.appendChild(template.content.cloneNode(true));

      const backButton = this.shadowRoot?.getElementById('back-button') as MdOutlinedIconButton | null;
      if (backButton) {
        backButton.addEventListener('click', () => { 
          console.log('backButton clicked');
          this.popDlg()
        });
      }

      const slot = this.shadowRoot?.getElementById('dlg-content') as HTMLSlotElement | null;
      if (!slot) {
        console.log("no slot");
        return;
      }

      const dlgContent = slot.assignedElements() as HTMLElement[];
      if (dlgContent.length == 0) {
        return;
      }

      this.dlgStack.push(dlgContent[0]);
    }

    pushDlg(dlg: HTMLElement) {
      if (!this.shadowRoot)
        return;

      const container = this.shadowRoot.querySelector('#dlg-frame') as HTMLElement;

      if (this.dlgStack.length > 0) {
        const currentDlg = this.dlgStack[this.dlgStack.length - 1];
        if (currentDlg) {
          //console.log(currentDlg);
          //currentDlg.style.transform = 'translateX(100%)';
          currentDlg.style.display = 'none';
          //console.log('hide current dlg');
        }
      }

      this.dlgStack.push(dlg);
      container.appendChild(dlg);
      this.updateBackButton();
    }

    popDlg() {
      if (this.dlgStack.length <= 1) 
        return;

      const currentDlg = this.dlgStack.pop();
      if (currentDlg) {
        currentDlg.style.display = 'none';
      }

      const previousDlg = this.dlgStack[this.dlgStack.length - 1];
      if (previousDlg) {
        previousDlg.style.display = 'block';
      }

      this.updateBackButton();
    }

    disableBackButton() {
      if (!this.shadowRoot)
        return;

      const backButton = this.shadowRoot.querySelector('#back-button') as HTMLButtonElement;
      backButton.style.display = 'none';
    }

    private updateBackButton() {
      if (!this.shadowRoot) 
        return;

      const backButton = this.shadowRoot.querySelector('#back-button') as HTMLButtonElement;
      backButton.style.display = this.dlgStack.length > 1 ? 'block' : 'none';
    }
}

customElements.define('bucky-wizzard-dlg', BuckyWizzardDlg);


