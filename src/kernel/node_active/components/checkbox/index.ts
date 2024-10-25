
import templateContent from './checkbox.template?raw';


class BuckyCheckBox extends HTMLElement {
    _checked: boolean;
    _disabled: boolean;
    _lable: string;

    constructor() {
      super();
      this._checked = false;
      this._disabled = false;
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

    set disabled(value) {
        this._disabled = value;
        const _element = this.shadowRoot.getElementById('check_box');
        if (_element) {
            _element.disabled = this._disabled;
        }
    }

    get disabled() {
        return this._disabled;
    }

    connectedCallback() {
        const template = document.createElement('template');
        template.innerHTML = templateContent;
        const shadow = this.attachShadow({ mode: 'open' });
        shadow.appendChild(template.content.cloneNode(true));

        this.lable = this.getAttribute('lable') || 'checkbox';
        this.checked = this.getAttribute('check') === 'true';
        this.disabled = this.getAttribute('disabled') === 'true';

        const _element = this.shadowRoot.getElementById('check_box');
        if (_element) {
            _element.addEventListener('click', () => {
                this.checked = !this.checked;
                this.dispatchEvent(new Event('click', { bubbles: true }));
            });
        }
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

