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

export type AppType = 'service' | 'dapp' | 'web' | 'agent'

/**
 * Flattened summary of an app, returned by `apps.list` and embedded in
 * `apps.details.summary`. Built from the backend's `AppServiceSpec`.
 */
export interface AppSummary {
  app_id: string
  show_name: string | null
  version: string
  app_type: AppType | string
  /** Icon URL as declared in AppDoc; may be null/empty. */
  app_icon_url: string | null
  /** Convention-based fallback: `res/<app_id>/appicon.png`. */
  icon_res_url: string
  author: string
  tags: string[]
  categories: string[]
  app_index: number
  enable: boolean
  state: AppState | string
  expected_instance_count: number
  is_agent: boolean
  /** True for BuckyOS built-in apps (MessageHub, HomeStation, Content Store). */
  is_system: boolean
  spec_path: string
  user_id: string
}

export interface AppsListResponse {
  user_id: string
  total: number
  apps: AppSummary[]
}

export interface AppDetailsResponse {
  app_id: string
  user_id: string
  is_agent: boolean
  is_system: boolean
  spec_path: string
  summary: AppSummary
  /** Full `AppServiceSpec` as serialized by the backend. */
  spec: Record<string, unknown>
}


// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Fetch the list of apps available to the caller (or an explicit user).
 *
 * The backend returns user-installed apps from
 * `users/{uid}/apps/*` and `users/{uid}/agents/*`, followed by BuckyOS
 * built-in system apps (marked `is_system: true`).
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
 * Resolves user-installed apps first (apps, then agents), and falls back to
 * built-in system apps so that callers can always inspect e.g. `messagehub`.
 */
export const fetchAppDetails = async (
  appId: string,
  options: { userId?: string } = {},
): Promise<{ data: AppDetailsResponse | null; error: unknown }> => {
  const params: Record<string, unknown> = { app_id: appId }
  if (options.userId) {
    params.user_id = options.userId
  }
  return callRpc<AppDetailsResponse>('apps.details', params)
}
