import {buckyos} from 'buckyos';


export class SystemConfigError extends Error {
    constructor(message: string) {
        super(message);
        this.name = 'SystemConfigError';
    }
}

export class SystemConfigClient {
    private client: any;

    constructor(rpc_client: any) {
        this.client = rpc_client;
    }

    async get(key: string): Promise<[string, number]> {
        try {
            const result = await this.client.call('sys_config_get', { key });
            if (result === null) {
                throw new SystemConfigError(`key ${key} not found`);
            }
            return [result as string, 0]; // revision 暂时返回 0
        } catch (error) {
            throw new SystemConfigError(`Failed to get key: ${error}`);
        }
    }

    async set(key: string, value: string): Promise<number> {
        if (!key || !value) {
            throw new SystemConfigError('key or value is empty');
        }
        if (key.includes(':')) {
            throw new SystemConfigError('key cannot contain ":"');
        }

        try {
            await this.client.call('sys_config_set', { key, value });
            return 0;
        } catch (error) {
            throw new SystemConfigError(`Failed to set key: ${error}`);
        }
    }

    async setByJsonPath(key: string, jsonPath: string, value: string): Promise<number> {
        try {
            await this.client.call('sys_config_set_by_json_path', {
                key,
                json_path: jsonPath,
                value
            });
            return 0;
        } catch (error) {
            throw new SystemConfigError(`Failed to set by json path: ${error}`);
        }
    }

    async create(key: string, value: string): Promise<number> {
        try {
            await this.client.call('sys_config_create', { key, value });
            return 0;
        } catch (error) {
            throw new SystemConfigError(`Failed to create: ${error}`);
        }
    }

    async delete(key: string): Promise<number> {
        try {
            await this.client.call('sys_config_delete', { key });
            return 0;
        } catch (error) {
            throw new SystemConfigError(`Failed to delete: ${error}`);
        }
    }

    async append(key: string, value: string): Promise<number> {
        try {
            await this.client.call('sys_config_append', {
                key,
                append_value: value
            });
            return 0;
        } catch (error) {
            throw new SystemConfigError(`Failed to append: ${error}`);
        }
    }

    async list(key: string): Promise<string[]> {
        try {
            const result = await this.client.call('sys_config_list', { key });
            return result as string[];
        } catch (error) {
            throw new SystemConfigError(`Failed to list: ${error}`);
        }
    }

    async execTx(
        txActions: Map<string, { action: string; value?: string; all_set?: any }>,
        mainKey?: [string, number]
    ): Promise<number> {
        if (txActions.size === 0) {
            return 0;
        }

        const actions: Record<string, any> = {};
        txActions.forEach((action, key) => {
            actions[key] = action;
        });

        const params: any = { actions };
        if (mainKey) {
            params.main_key = `${mainKey[0]}:${mainKey[1]}`;
        }

        try {
            await this.client.call('sys_config_exec_tx', params);
            return 0;
        } catch (error) {
            throw new SystemConfigError(`Failed to execute transaction: ${error}`);
        }
    }
}
