import { callRpc } from './rpc.ts'

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

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

export interface AppDetailsResponse {
  app_id: AppId
  app_instance_id: AppInstanceId
  owner_user_id: string
  spec_path: string
  summary: AppSummary
  /** Full `AppServiceSpec` as serialized by the backend. */
  spec: Record<string, unknown>
}

export interface AppDeploymentIdentity {
  app_instance_id: string
  task_id: string
  app_doc_object_id: string
  spec_generation: number
  pikg_digest?: string
}

export interface AppDeploymentObservation {
  deployment?: AppDeploymentIdentity | null
  node_id: string
  observed_at: number
  expires_at: number
  deployment_error?: { code: string; message: string } | null
}

export interface AppInstallationStatus {
  app_instance_id: string
  desired_deployment: AppDeploymentIdentity
  desired_spec: {
    state: AppState | string
    enable: boolean
    packages?: Array<{ sub_pkg_name: string }>
    app_doc: { app_type: string; pkg_list?: Record<string, unknown> }
  }
  desired_instance_count: number
  runtime_instances?: Array<AppDeploymentObservation & { state: string; health: string }>
  static_web_evidence?: Array<AppDeploymentObservation & {
    materialized_at: number
    gateway_ready_at: number
    gateway_config_generation: string
  }>
}

export type AvailabilityEffect = 'allow' | 'deny'

/**
 * `admins` / `users` / `limited` are the account-type groups, `guest` the anonymous pseudo-group
 * (requests without an account), `group:<id>` a system user group. System group rules come before
 * the account-type rule; among a user's system groups an allow wins.
 */
export interface AvailabilityGroupRule {
  group_id: 'admins' | 'users' | 'limited' | 'guest' | `group:${string}`
  effect: AvailabilityEffect
}

export interface AvailabilityUserRule {
  user_id: string
  effect: AvailabilityEffect
}

export interface AppAvailabilityPolicy {
  schema_version: number
  app_instance_id: AppInstanceId
  default_effect: 'deny'
  group_rules: AvailabilityGroupRule[]
  user_rules: AvailabilityUserRule[]
  revision: number
  updated_by: string
  updated_at: number
}

export interface AppAvailabilityDecision {
  allowed: boolean
  app_id: AppId
  app_instance_id: AppInstanceId
  owner_user_id: string
  availability_match?: AvailabilityMatch
  reason: string
}


// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Fetch the list of apps available to the caller (or an explicit user).
 *
 * The backend returns the effective authorized set for the target user.
 */
export const fetchAppList = async (
  options: { userId?: string } = {},
): Promise<{ data: AppsListResponse | null; error: unknown }> => {
  const params: Record<string, unknown> = {}
  if (options.userId) {
    params.user_id = options.userId
  }
  return callRpc<AppsListResponse>('apps.list', params)
}

/**
 * Fetch the full details (including full `AppServiceSpec`) for a single app.
 *
 * Resolves user-installed apps first, and falls back to built-in system apps
 * so that callers can always inspect e.g. `messagehub`.
 */
export const fetchAppDetails = async (
  appInstanceId: string,
): Promise<{ data: AppDetailsResponse | null; error: unknown }> => {
  return callRpc<AppDetailsResponse>('apps.details', {
    app_instance_id: appInstanceId,
  })
}

export const fetchAppAvailability = async (
  appInstanceId: string,
): Promise<{ data: AppAvailabilityPolicy | null; error: unknown }> =>
  callRpc<AppAvailabilityPolicy>('apps.availability.get', {
    app_instance_id: appInstanceId,
  })

export const fetchAppStatus = (appInstanceId: string) =>
  callRpc<AppInstallationStatus>('apps.status', { selector: appInstanceId })

export const setAppAvailability = async (
  appInstanceId: string,
  expectedRevision: number,
  groupRules: AvailabilityGroupRule[],
  userRules: AvailabilityUserRule[],
): Promise<{ data: AppAvailabilityPolicy | null; error: unknown }> =>
  callRpc<AppAvailabilityPolicy>('apps.availability.set', {
    app_instance_id: appInstanceId,
    expected_revision: expectedRevision,
    group_rules: groupRules,
    user_rules: userRules,
  })

export const checkAppAvailability = async (
  appInstanceId: string,
  userId?: string,
): Promise<{ data: AppAvailabilityDecision | null; error: unknown }> =>
  callRpc<AppAvailabilityDecision>('apps.availability.check', {
    app_instance_id: appInstanceId,
    ...(userId ? { user_id: userId } : {}),
  })
