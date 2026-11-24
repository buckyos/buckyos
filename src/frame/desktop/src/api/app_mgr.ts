import {
  listApps,
  type AppId,
  type AppInstanceId,
  type AppState,
  type AppsListResponse,
  type AppSummary,
  type AvailabilityMatch,
} from '../shared/controlPanel/apps.ts'
import { shellChannel } from './channel.ts'
import { callRpc, settle } from './rpc.ts'

export type {
  AppId,
  AppInstanceId,
  AppPresentationText,
  AppRuntimeType,
  AppsListResponse,
  AppState,
  AppSummary,
  AvailabilityMatch,
  AvailabilityMatchType,
} from '../shared/controlPanel/apps.ts'

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
): Promise<{ data: AppsListResponse | null; error: unknown }> => settle(listApps(shellChannel, options))

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
