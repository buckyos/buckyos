import type { ZoneChannel } from '../channel.ts'
import { callControlPanel } from './call.ts'

export type AppState =
  | 'new'
  | 'running'
  | 'stopped'
  | 'stopping'
  | 'restarting'
  | 'updating'
  | 'deleted'
  | 'unknown'

export type AppRuntimeType = 'service' | 'dapp' | 'web' | 'agent'
export type AppId = string & { readonly __appId: unique symbol }
export type AppInstanceId = string & { readonly __appInstanceId: unique symbol }
export type AvailabilityMatchType =
  | 'system_builtin'
  | 'owner'
  | 'zone_all_users'
  | 'group'
  | 'exact_user'

export interface AvailabilityMatch {
  type: AvailabilityMatchType
  subject?: string
}

/**
 * Flattened summary of an app, returned by `apps.list` and embedded in
 * `apps.details.summary`. Built from the backend's `AppServiceSpec`.
 */
export interface AppSummary {
  app_id: AppId
  /** Stable Zone-wide identity used by routing, authorization, and UI actions. */
  app_instance_id: AppInstanceId
  app_did: string
  runtime_type: AppRuntimeType
  owner_user_id: string
  availability_match: AvailabilityMatch | null
  /** The AppDoc package name. */
  name?: string | null
  show_name: string | null
  /** AppDoc `presentation` text, each a language → text map; null when the document has none. */
  presentation?: AppPresentationText | null
  version: string
  /**
   * AppDoc `presentation.links.app_icon_url`: absolute, a data URL, or relative to the app's own
   * web host. `presentation.icons` (ObjIds) are not served to browsers yet.
   */
  app_icon_url: string | null
  author: string
  app_index: number
  enable: boolean
  state: AppState | string
  expected_instance_count: number
  spec_path: string
  /** Gateway host keys compiled from Web expose_config; first entry is the default launch host. */
  web_hosts: string[]
  /** AppDoc `window`, to be read with the SDK's `readAppWindowDeclaration`; null when the document has none. */
  window?: Record<string, unknown> | null
}

export interface AppPresentationText {
  title?: Record<string, string>
  summary?: Record<string, string>
  description?: Record<string, string>
}

export interface AppsListResponse {
  user_id: string
  total: number
  apps: AppSummary[]
}

/** The apps available to the caller (or to an explicit user): the effective authorized set the backend computed. */
export function listApps(channel: ZoneChannel, options: { userId?: string } = {}): Promise<AppsListResponse> {
  return callControlPanel<AppsListResponse>(channel, 'apps.list', options.userId ? { user_id: options.userId } : {})
}
