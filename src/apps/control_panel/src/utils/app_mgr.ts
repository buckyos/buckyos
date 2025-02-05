import {buckyos} from 'buckyos';
import { SystemConfigClient } from './sys_config_client';

export interface SubPkgDesc {
    pkg_id: string;
    docker_image_name?: string;
    [key: string]: string | undefined;  // 其他配置项
}

export interface AppDoc {
    app_id: string;
    name: string;
    description: string;
    vendor_did: string;
    pkg_id: string;
    // service名称 -> 完整镜像URL的映射
    pkg_list: { [key: string]: SubPkgDesc };
    //TODO:要增加安装配置还是分开？
}

export interface AppConfig {
    app_id: string;
    app_doc: AppDoc;
    app_index: number;  // app在用户app列表中的索引
    enable: boolean;
    instance: number;   // 期望的实例数量
    state: string;
    data_mount_point: string;
    cache_mount_point?: string;
    local_cache_mount_point?: string;
    extra_mounts?: { [key: string]: string };  // 额外挂载点映射，real_path:docker_inner_path
    max_cpu_num?: number;
    max_cpu_percent?: number;  // 0-100
    memory_quota?: number;     // 内存配额（字节）
    tcp_ports: { [key: string]: number };  // 网络资源映射，name:docker_inner_port
    udp_ports?: { [key: string]: number };
}


export async function install_app(app_id:string) {

}

export async function install_app_by_url(app_url:string) {

}

export async function read_app_doc_from_url(app_url:string) : Promise<AppDoc|null> {
    return null;
}

export async function install_app_by_config(app_config: AppConfig) {
    let session_info = await buckyos.getAccountInfo();
    if (session_info == null) {
        console.error('session_info is null');
        return;
    }
  
    let rpc_client = buckyos.getServiceRpcClient("system_config");
    let user_id = session_info.user_id;
    try {
        let result = await rpc_client.call("sys_config_create", {
            key: `users/${user_id}/apps/${app_config.app_id}/config`,
            value: JSON.stringify(app_config)
        });
        console.log('install_app_by_app_doc', result);
        result = await rpc_client.call("sys_config_append", {
            key: `system/rbac/policy`,
            append_value: `\ng, ${app_config.app_id}, app`
        });
        console.log('set app rbac rules:', result);
    } catch (error) {
        console.error('install_app_by_app_doc error:', error);
    }
}

export async function uninstall_app(app_id: string,keep_user_data: boolean) {
    console.log('uninstall_app', app_id);
}

export async function enabel_app(app_id:string, is_enable:boolean) {

}

export async function get_app_config(app_id: string) : Promise<AppConfig|null> {
    let session_info = await buckyos.getAccountInfo();
    if (session_info == null) {
        console.error('session_info is null');
        return null;
    }
    let rpc_client = buckyos.getServiceRpcClient("system_config");
    let sys_client = new SystemConfigClient(rpc_client);
    let app_config_result = await sys_client.get(`users/${session_info.user_id}/apps/${app_id}/config`);
    if (app_config_result == null) {
        console.error('app_config is null');
        return null;
    }
    let app_config_str = app_config_result[0];
    if (app_config_str == null) {
        console.error('app_config is null');
        return null;
    }

    let app_config = JSON.parse(app_config_str) as AppConfig;
    return app_config;
}

export async function get_app_list() : Promise<AppConfig[] | null> {
    let session_info = await buckyos.getAccountInfo();
    if (session_info == null) {
        console.error('session_info is null');
        return null;
    }
    let rpc_client = buckyos.getServiceRpcClient("system_config");
    let sys_client = new SystemConfigClient(rpc_client);
    let app_list = await sys_client.list(`users/${session_info.user_id}/apps`);
    if (app_list == null) {
        console.error('app_list is null');
        return null;
    }
    let app_configs = await Promise.all(app_list.map((app_id: string) => get_app_config(app_id)));
    return app_configs.filter((app_config): app_config is AppConfig => app_config !== null);
}
