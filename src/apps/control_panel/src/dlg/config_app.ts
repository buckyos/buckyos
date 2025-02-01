import { html, css, LitElement } from 'lit';
import { customElement } from 'lit/decorators.js';

@customElement('config-app-content')
export class ConfigAppContent extends LitElement {
  static styles = css`
    .config-app-dialog {
      padding: 1rem;
    }

    .version-info {
      margin-bottom: 1rem;
    }

    .permission-text {
      margin-bottom: 0.5rem;
    }

    ul {
      margin: 0;
      padding-left: 1.5rem;
    }
  `;

  render() {
    return html`
      <div class="config-app-dialog">
        <div class="version-info">
          v1.0.1 开发者: buckyos
        </div>
        
        <div class="permission-text">
          您确认要安装该应用吗？它将申请下面权限
        </div>

        <ul>
          <li>安装App服务，可通过浏览器访问</li>
          <li>需要永久存储空间来保存App数据</li>
        </ul>
      </div>
    `;
  }
}
