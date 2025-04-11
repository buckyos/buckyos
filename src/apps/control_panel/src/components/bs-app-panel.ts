import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import '@shoelace-style/shoelace';

@customElement('bs-app-panel')
export class BsAppPanel extends LitElement {
  declare appId:string;
  declare appName:string;
  declare version:string;
  declare status:string;
  declare description:string;
  declare iconUrl:string;

  static styles = css`
    :host {
      display: block;
      background: var(--sl-color-neutral-0);
      border-radius: 8px;
      padding: 12px;
      box-shadow: var(--sl-shadow-x-small);
    }

    .app-container {
      display: flex;
      gap: 12px;
      align-items: center;
    }

    .app-icon {
      width: 48px;
      height: 48px;
      border-radius: 8px;
    }

    .app-info {
      flex: 1;
    }

    .app-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-bottom: 4px;
    }

    .app-name {
      font-size: 16px;
      font-weight: 500;
      color: var(--sl-color-neutral-900);
    }

    .app-version {
      font-size: 12px;
      color: var(--sl-color-neutral-600);
    }

    .app-status {
      font-size: 12px;
      padding: 2px 8px;
      border-radius: 12px;
      background: var(--sl-color-success-100);
      color: var(--sl-color-success-700);
    }

    .app-description {
      font-size: 14px;
      color: var(--sl-color-neutral-700);
      display: -webkit-box;
      -webkit-line-clamp: 2;
      -webkit-box-orient: vertical;
      overflow: hidden;
    }
  `;

  render() {
    return html`
      <div class="app-container">
        <img 
          class="app-icon"
          src=${this.iconUrl || 'default-app-icon.png'}
          alt=${this.appName}
        />
        <div class="app-info">
          <div class="app-header">
            <div class="app-name">${this.appName}</div>
            <div class="app-version">${this.version}</div>
          </div>
          <div class="app-status">${this.status}</div>
          <div class="app-description">${this.description}</div>
        </div>
      </div>
    `;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'bs-app-panel': BsAppPanel;
  }
}
