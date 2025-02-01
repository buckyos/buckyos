import { css, html, LitElement } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import '@/components/bs-app-panel';
import './config_app';
import '@shoelace-style/shoelace/dist/components/button/button.js';
import '@shoelace-style/shoelace/dist/components/icon/icon.js';
import '@shoelace-style/shoelace/dist/components/dialog/dialog.js';
import '@shoelace-style/shoelace/dist/components/textarea/textarea.js';
import { read_app_doc_from_url, install_app_by_config } from '@/utils/app_mgr';

import { AppDoc, AppConfig } from '@/utils/app_mgr';


@customElement('app-setting-dialog')
export class AppSettingDialog extends LitElement {
  declare apps: AppConfig[];

  static styles = css`
    :host {
      display: block;
      padding: 16px;
    }

    .header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 24px;
    }

    .title {
      font-size: 20px;
      font-weight: bold;
    }

    .app-group {
      margin-bottom: 24px;
    }

    .group-title {
      font-size: 18px;
      font-weight: bold;
      margin-bottom: 16px;
    }

    .app-grid {
      display: grid;
      gap: 16px;
      grid-template-columns: 1fr;
    }

    @media (min-width: 768px) {
      .app-grid {
        grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
      }
    }

    .dialog-content {
      padding: 20px;
    }
  `;

  private groupApps() {
    const groups = new Map<string, AppConfig[]>();
    
    groups.set("已安装", this.apps);

    return groups;
  }

  setApps(apps: AppConfig[]) {
    this.apps = apps;
    this.requestUpdate();
  }

  private showAddAppDialog() {
    const dialog = this.shadowRoot?.querySelector('#add-app-dialog') as any;
    dialog?.show();
  }

  private async closeAddAppDialog() {
    const dialog = this.shadowRoot?.querySelector('#add-app-dialog') as any;
    dialog?.hide();
  }

  private async handleAddApp() {
    const textarea = this.shadowRoot?.querySelector('#app-config') as any;
    const input_app_doc:string = textarea?.value;
    console.log('New app config:', input_app_doc);
    let app_doc: AppDoc | null = null;
    if(input_app_doc.startsWith("http")) {
      app_doc = read_app_doc_from_url(input_app_doc);
      if(app_doc != null) {
        app_doc = JSON.parse(app_doc) as AppDoc;
      }
    
    } else {
      //app_doc = JSON.parse(input_app_doc);
      app_doc =  {};
    }

    if (app_doc != null) {
      console.log('create app_config for app_doc', app_doc);
      //sleep 1ms
      await new Promise(resolve => setTimeout(resolve, 1));
      //创建config_app dlg
      const config_dlg = this.shadowRoot?.querySelector('#config-app-dialog') as any;
      config_dlg?.show();

      /*
      // 创建app配置
      const app_config: AppConfig = {
        id: app_doc.pkg_id,
        app_doc: app_doc,
        app_index: 0,
        enable: true,
        instance: 1,
        state: 'stopped',
        data_mount_point: `/data/${app_doc.pkg_id}`,
        tcp_ports: {}
      };

      // 为每个服务配置端口
      for (const [service_name, pkg_desc] of Object.entries(app_doc.pkg_list)) {
        if (pkg_desc.docker_image_name) {
          app_config.tcp_ports[service_name] = 0; // 0表示自动分配端口
        }
      }

      install_app_by_config(app_config);
      alert('发送安装请求成功,系统将很快自动完成所有配置工作');
      */
    }
  }

  render() {
    const groupedApps = this.groupApps();
    const totalApps = this.apps?.length || 0;

    return html`
      <div class="header">
        <div class="title">已安装应用 (${totalApps})</div>
        <sl-button variant="primary" @click=${this.showAddAppDialog}>
          <sl-icon slot="prefix" name="plus"></sl-icon>
          添加应用
        </sl-button>
      </div>

      ${Array.from(groupedApps.entries()).map(([group, apps]) => html`
        <div class="app-group">
          <div class="group-title">${group}</div>
          <div class="app-grid">
            ${apps.map(app => html`
              <bs-app-panel
                .name=${app.app_doc.name}
                .version=${app.app_doc.pkg_id}
                .status=${app.state}
                .description=${app.app_doc.description}
              ></bs-app-panel>
            `)}
          </div>
        </div>
      `)}

      <sl-dialog label="添加新应用" id="add-app-dialog">
        <div class="dialog-content">
          <sl-textarea
            id="app-config"
            label="应用配置"
            rows="8"
            placeholder="请输入应用配置信息..."
          ></sl-textarea>
        </div>
        <div slot="footer">
          <sl-button variant="neutral" @click=${this.closeAddAppDialog}>取消</sl-button>
          <sl-button variant="primary" @click=${this.handleAddApp}>确定</sl-button>
        </div>
      </sl-dialog>

      <sl-dialog label="确认安装" id="config-app-dialog">
        <div class="dialog-content">
          <config-app-content></config-app-content>
        </div>
        <div slot="footer">
          <sl-button variant="neutral" @click=${this.closeAddAppDialog}>取消</sl-button>
          <sl-button variant="primary" @click=${this.handleAddApp}>确定</sl-button>
        </div>
      </sl-dialog>
    `;
  }
}
