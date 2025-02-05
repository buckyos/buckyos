import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { i18next } from '../i18n';
import '@shoelace-style/shoelace';

@customElement('bs-title-bar')
export class BsTitleBar extends LitElement {
  declare username:string;
  declare avatarUrl:string;
  declare message:string;
  declare hasMessage:boolean;
  declare showBackButton:boolean;
  declare pathSegments:string[];

  
  private isMobile = false;

  static styles = css`
    :host {
      display: block;
      width: 100%;
      background: var(--sl-color-neutral-0);
      border-bottom: 1px solid var(--sl-color-neutral-200);
    }

    .title-bar {
      display: flex;
      align-items: center;
      padding: 0.5rem 1rem;
      gap: 1rem;
    }

    .left-section {
      display: flex;
      align-items: center;
      flex: 1;
      gap: 0.5rem;
    }

    .right-section {
      display: flex;
      align-items: center;
      gap: 1rem;
    }

    .path-segments {
      display: flex;
      align-items: center;
      gap: 0.5rem;
    }

    .message {
      color: var(--sl-color-danger-600);
      display: flex;
      align-items: center;
      gap: 0.25rem;
    }

    @media (max-width: 768px) {
      .username {
        display: none;
      }
      
      .path-segments {
        display: none;
      }
    }
  `;

  constructor() {
    super();
    this.pathSegments = [];
    this.handleResize = this.handleResize.bind(this);
  }

  connectedCallback() {
    super.connectedCallback();
    window.addEventListener('resize', this.handleResize);
    this.handleResize();
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    window.removeEventListener('resize', this.handleResize);
  }

  private handleResize() {
    this.isMobile = window.innerWidth <= 768;
    this.showBackButton = this.isMobile;
  }

  render() {
    return html`
      <div class="title-bar">
        <div class="left-section">
          ${this.showBackButton ? html`
            <sl-icon-button 
              name="arrow-left" 
              label=${i18next.t('back')}
              @click=${this.handleBack}
            ></sl-icon-button>
          ` : ''}

          ${!this.isMobile ? html`
            <div class="path-segments">
              ${this.pathSegments.map((segment, index) => html`
                ${index > 0 ? html`<sl-icon name="chevron-right"></sl-icon>` : ''}
                <span>${segment}</span>
              `)}
            </div>
          ` : html`
            <span>${this.pathSegments[this.pathSegments.length - 1]}</span>
          `}
        </div>

        <div class="right-section">
          ${this.hasMessage ? html`
            <div class="message">
              <sl-icon name="exclamation-circle"></sl-icon>
              <span>${this.message}</span>
            </div>
          ` : ''}

          <div class="user-info">
            ${!this.isMobile ? html`
              <span class="username">${this.username}</span>
            ` : ''}
            <sl-avatar
              image=${this.avatarUrl}
              label=${this.username}
            ></sl-avatar>
          </div>
        </div>
      </div>
    `;
  }

  private handleBack() {
    this.dispatchEvent(new CustomEvent('back'));
  }
} 