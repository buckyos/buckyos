import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import { i18next } from '../i18n';
import '@shoelace-style/shoelace';
import { LOGIN_EVENT, LoginEventDetail,doLogin} from '../utils/account';


// 创建事件分发函数
export function dispatchLoginEvent(login_result: any) {
  const event = new CustomEvent<LoginEventDetail>(LOGIN_EVENT, {
    detail: {
      login_result,
      timestamp: Date.now()
    },
    bubbles: true,
    composed: true
  });
  console.log("dispatchLoginEvent: ", event);
  window.dispatchEvent(event);
}

@customElement('login-form')
export class LoginForm extends LitElement {
  declare zoneId:string;
  declare deviceName:string;
  declare ipAddress:string;
  i18nReady:boolean = false;
  static styles = css`
    :host {
      display: flex;
      justify-content: center;
      align-items: center;
      min-height: 100vh;
      padding: 1rem;
    }

    .login-container {
      width: 100%;
      max-width: 400px;
      padding: 2rem;
      border-radius: 10px;
      background: rgba(255, 255, 255, 0.7);
      backdrop-filter: blur(35px);
      -webkit-backdrop-filter: blur(35px);
      box-shadow: var(--sl-shadow-medium);
      border: 1px solid rgba(255, 255, 255, 0.12);
    }

    .header {
      text-align: center;
      margin-bottom: 2rem;
    }

    .logo {
      width: 64px;
      height: 64px;
      margin-bottom: 1rem;
    }

    .device-info {
      margin-bottom: 1.5rem;
      text-align: center;
      color: var(--sl-color-neutral-600);
    }

    sl-input, sl-button {
      margin-bottom: 1rem;
      width: 100%;
    }
  `;

  constructor() {
    super();
    this.zoneId = 'buckyos.io';
    this.deviceName = 'My BuckyOS';
    this.ipAddress = '192.168.1.1';
 
    i18next.on('initialized', () => {
      this.i18nReady = true;
      this.requestUpdate();
    });

  }

  render() {
    return html`
      <div class="login-container">
        <div class="header">
          <img src="/assets/zone_logo.svg" alt="Logo" class="logo">
          <div class="device-info">
            <div>${this.zoneId}</div>
            <div>${this.deviceName}</div>
            <div>${this.ipAddress}</div>
          </div>
        </div>

        <form @submit=${this._handleSubmit}>
          <sl-input 
            name="username"
            type="text"
            data-i18n="email_username"
            placeholder=${i18next.t('enter_email_username')}
            required
          >
            <sl-icon name="person" slot="prefix"></sl-icon>
          </sl-input>

          <sl-input 
            name="password"
            type="password"
            data-i18n="password"
            placeholder=${i18next.t('enter_password')}
            required
            toggle-password
          >
            <sl-icon name="lock" slot="prefix"></sl-icon>
          </sl-input>

          <sl-button type="submit" variant="primary" data-i18n="login">
            ${i18next.t('login')}
          </sl-button>
        </form>
      </div>
    `;
  }

  private async _handleSubmit(e: Event) {
    e.preventDefault();
    // 处理登录逻辑
    let login_button = this.shadowRoot?.querySelector('sl-button[data-i18n="login"]') as HTMLButtonElement;
    login_button.disabled = true;
    let username = (this.shadowRoot?.querySelector('sl-input[name="username"]') as HTMLInputElement)?.value;
    let password = (this.shadowRoot?.querySelector('sl-input[name="password"]') as HTMLInputElement)?.value;
    if (username && password) {
      try {
        let login_result = await doLogin(username, password, "control_panel", window.location.href);
        dispatchLoginEvent(login_result);
      } catch (error) {
        console.error("login failed: ", error);
        alert(error);
      } finally {
        login_button.disabled = false;
      }
    }
  }
} 