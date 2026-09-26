import { buckyos, getActiveSessionToken } from 'buckyos'
import { isMockRuntime } from '../runtime'
import { toAiccRpcCallOptions } from './aicc_rpc_options'
import { PLAYGROUND_APIS, record, type JsonObject } from '../app/ai-center/datamodel/playground'
import type { ApiType, ProviderView } from '../app/ai-center/mock/types'

export interface PlaygroundModel { exact_model: string; provider: string; api_types: ApiType[]; health?: string }

async function call(method: string, params: JsonObject): Promise<JsonObject> {
  const account = await buckyos.getAccountInfo()
  const token = account?.session_token || await getActiveSessionToken()
  if (!token) throw new Error('Current login session expired. Please sign in again.')
  return record(await buckyos.getServiceRpcClient('aicc').call(method, params, toAiccRpcCallOptions(token)))
}

export async function listPlaygroundModels(providers: ProviderView[]): Promise<PlaygroundModel[]> {
  if (isMockRuntime()) return providers.filter((provider) => provider.config.enabled).flatMap((provider) => provider.inventory.models.map((model) => ({ exact_model: model.exact_model, provider: provider.config.provider_instance_name, api_types: model.api_types, health: model.health.status })))
  const response = await call('models.list', {})
  if (!Array.isArray(response.models)) throw new Error('Invalid models.list response')
  return response.models.map((value) => {
    const model = record(value)
    return { exact_model: String(model.exact_model ?? ''), provider: String(model.provider_instance_name ?? ''), api_types: (Array.isArray(model.api_types) ? model.api_types : []) as ApiType[], health: String(record(model.health).status ?? '') }
  }).filter((model) => model.exact_model.includes('@'))
}

export async function invokePlayground(api: ApiType, params: JsonObject): Promise<JsonObject> {
  if (isMockRuntime()) {
    const { mockPlaygroundResponse } = await import('../app/ai-center/mock/playground')
    return mockPlaygroundResponse(api, params)
  }
  return call(PLAYGROUND_APIS[api].method, params)
}

export async function getPlaygroundTask(taskId: string): Promise<JsonObject> {
  return record(await buckyos.getTaskManagerClient().getTask(taskId))
}

export async function cancelPlaygroundTask(taskId: string): Promise<JsonObject> {
  return call('cancel', { task_id: taskId })
}
