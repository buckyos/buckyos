
import templateContent from './checkbox.template?raw';


class BuckyCheckBox extends HTMLElement {
    _checked: boolean;
    _lable: string;

    constructor() {
      super();
      this._checked = false;
      this._lable = '';
    }

    set lable(value) {
        this._lable = value;
        if (!this.shadowRoot) {
            return;
        }
        
        const _element = this.shadowRoot.getElementById('check_lable');
        if (_element) {
            _element.innerText = this._lable;
        }
    }
      
    get lable() {
        return this._lable;
    }

    set checked(value) {
        this._checked = value;
        if (!this.shadowRoot) {
            return;
        }

        const _element = this.shadowRoot.getElementById('check_box');
        if (_element) {
            _element.checked = this._checked;
        }
    }

    get checked() {
        return this._checked;
    }

    connectedCallback() {
        const template = document.createElement('template');
        template.innerHTML = templateContent;
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        this.lable = this.getAttribute('lable') || 'checkbox';
        this.checked = this.getAttribute('check') === 'true';
      }

    attributeChangedCallback(attributeName:string, oldValue:string, newValue:string) {
        if (attributeName === 'label') {
          this.lable = newValue;
        }

        if (attributeName === 'check') {
          this.checked = newValue === 'true';
        }
    }
}

customElements.define("bucky-checkbox", BuckyCheckBox);

