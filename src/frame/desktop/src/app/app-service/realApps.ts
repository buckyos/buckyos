/* ── The App layer from `apps.list` (real runtime): display and state only ──
 *
 * Installer tasks, drafts and runtime evidence stay with the prototype store; a listed real app
 * gets no simulated actions. Title, summary and icon come from the same AppDoc projection the
 * Desktop uses, so both show one app the same way.
 */

import type { AppRuntimeType, AppSummary } from '../../api/app_mgr'
import { createBackendAppDefinitionMapper, type BackendAppMapperOptions } from '../backend-apps'
import { pickLocalized } from '../../presentation/model'
import { emptyRuntime } from './mock/fixtures'
import type { AppServiceItem, AppServiceStatus, RuntimeType } from './types'

const runtimeTypes: Record<AppRuntimeType, RuntimeType> = { service: 'docker', dapp: 'docker', web: 'web', agent: 'agent' }

function serviceStatus(state: string): AppServiceStatus {
  switch (state) {
    case 'running':
    case 'stopped':
    case 'stopping':
      return state
    case 'new':
      return 'deploying'
    case 'restarting':
    case 'updating':
      return 'starting'
    default:
      return 'unknown'
  }
}

export function appServiceItemFromSummary(summary: AppSummary, options: BackendAppMapperOptions = {}): AppServiceItem {
  const definition = createBackendAppDefinitionMapper([], options)(summary)
  const status = serviceStatus(String(summary.state))
  const address = definition.webHosts?.[0] && options.zoneHost
    ? `${options.protocol ?? 'https:'}//${definition.webHosts[0]}.${options.zoneHost}`
    : undefined
  return {
    id: summary.app_instance_id,
    app_instance_id: summary.app_instance_id,
    system_service_id: null,
    owner_user_id: summary.owner_user_id,
    app_did: definition.did ?? null,
    name: pickLocalized(definition.text?.title, 'en') ?? summary.app_id,
    description: '',
    iconKey: summary.app_id,
    ...(definition.iconUrl ? { iconUrl: definition.iconUrl } : {}),
    ...(definition.text ? { text: definition.text } : {}),
    version: summary.version,
    layer: 'app',
    status,
    runtime: emptyRuntime(runtimeTypes[summary.runtime_type] ?? 'unknown', status),
    record: null,
    docker: null,
    diagnostics: [],
    spec: {
      enable: String(summary.enable),
      expected_instance_count: String(summary.expected_instance_count),
    },
    settings: {},
    serviceInfo: address ? { endpoint: address } : {},
    logs: [],
    available_actions: [],
    operation: null,
    operation_error: null,
  }
}
