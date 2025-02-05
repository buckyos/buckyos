import { html, css, LitElement } from 'lit';
import { customElement } from 'lit/decorators.js';
import { AppDoc, AppConfig } from '../utils/app_mgr';

@customElement('config-app-content')
export class ConfigAppContent extends LitElement {
  declare app_doc: AppDoc;

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

  setAppDoc(app_doc: AppDoc) {
    this.app_doc = app_doc;
  }

  getAppConfig() : AppConfig | null {
    let app_config: AppConfig = {
      app_id: this.app_doc.app_id,
      app_doc: this.app_doc,
      app_index: 3,
      enable: true,
      instance: 1,
      state: "New",
      data_mount_point: "/data/",
      cache_mount_point: "/cache/",
      local_cache_mount_point: "/local_cache/",
      extra_mounts: {},
      max_cpu_num: 2,
      max_cpu_percent: 100,
      memory_quota: 1024 * 1024 * 1024,
      tcp_ports: {
        "www": 80,
      },
      udp_ports: {},
    }
    return app_config;
  }

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
