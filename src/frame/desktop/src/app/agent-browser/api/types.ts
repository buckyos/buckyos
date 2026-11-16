/* The wire types of doc/opendan/protocol/Agent Browser Protocol.md: kRPC `browser.*` (§2) and the screen (§3). */

export type Controller = 'user' | 'agent'
export type AgentState = 'idle' | 'working' | 'waiting_answer' | 'waiting_takeover' | 'done' | 'stuck'
export type HandoverReason = 'password' | 'captcha' | 'other'

export const AGENT_STATES: readonly AgentState[] = ['idle', 'working', 'waiting_answer', 'waiting_takeover', 'done', 'stuck']

export interface BrowserTab {
  tab_id: string
  url: string
  title: string
  loading: boolean
  active: boolean
}

export interface BrowserGroup {
  group_id: string
  controller: Controller
  user_hold: boolean
  agent_state: AgentState
  note: string | null
  handover: { reason: HandoverReason; note?: string | null; at_ms: number } | null
  lease: { session_id: string; expires_at_ms: number } | null
  tabs: BrowserTab[]
}

/** Why there is no browser (§2.1): turned off for the Agent, no ext-core on the node, or the browser not downloaded there yet. */
export type UnavailableCode = 'disabled' | 'no_package' | 'not_downloaded' | 'no_display'

export interface BrowserStatus {
  enabled: boolean
  available: boolean
  running: boolean
  unavailable: string | null
  unavailable_code: UnavailableCode | null
  /** How the browser is shown (§2.1): headed on a virtual display, or headless. */
  display: 'virtual' | 'headless'
  /** Device pixels per CSS pixel the headed browser draws at; null when headless. */
  device_scale_factor: number | null
  version: { browser: string; playwright: string } | null
  /** The MessageHub session the default group is bound to (`browser.bind_default`). */
  default_session_id: string | null
  groups: BrowserGroup[]
}

export interface Viewport {
  width: number
  height: number
  mobile: boolean
  /** Pixels of a frame per CSS pixel, 1–2 (§3.2); the service's state always says it, a message may leave it out. */
  pixel_ratio?: number
}

/** `state` (and `ready.state`): always the whole state. */
export interface ScreenState {
  group: BrowserGroup
  active_tab_id: string | null
  url: string
  title: string
  loading: boolean
  can_go_back: boolean
  can_go_forward: boolean
  viewport: Viewport
  running: boolean
}

export type MouseButton = 'left' | 'middle' | 'right' | 'none'
export interface TouchPoint { id: number; x: number; y: number }

export type ScreenInput =
  | { kind: 'touch'; phase: 'start' | 'move' | 'end' | 'cancel'; points: TouchPoint[] }
  | { kind: 'mouse'; action: 'down' | 'up' | 'move'; x: number; y: number; button: MouseButton; click_count: number; modifiers: number }
  | { kind: 'wheel'; x: number; y: number; delta_x: number; delta_y: number; modifiers: number }
  | { kind: 'key'; action: 'down' | 'up'; key: string; code: string; modifiers: number; text: string }
  | { kind: 'text'; text: string }
  | { kind: 'composition'; text: string; selection_start: number; selection_end: number }

export type RequestResult =
  | { selected: number[] }
  | { accept: boolean; prompt_text?: string }
  | { cancel: true }

export type ScreenUpstream =
  | { type: 'hello'; token: string; group_id: string; viewport: Viewport }
  | { type: 'ack'; seq: number }
  | ({ type: 'input' } & ScreenInput)
  | { type: 'nav'; action: 'url'; url: string }
  | { type: 'nav'; action: 'back' | 'forward' | 'reload' | 'stop' }
  | { type: 'tab'; action: 'open'; url?: string }
  | { type: 'tab'; action: 'close' | 'activate'; tab_id: string }
  | ({ type: 'viewport' } & Viewport)
  | { type: 'respond'; request_id: string; result: RequestResult }
  | { type: 'quality'; max_fps?: number; jpeg_quality?: number; pixel_ratio?: number }
  | { type: 'ping' }

export interface SelectOption { index: number; label: string; selected: boolean; disabled: boolean; group?: string }

interface RequestBase { request_id: string; tab_id: string }
export type ScreenRequest =
  | (RequestBase & { kind: 'select'; multiple: boolean; options: SelectOption[]; rect: { x: number; y: number; width: number; height: number } })
  | (RequestBase & { kind: 'dialog'; dialog_type: 'alert' | 'confirm' | 'prompt' | 'beforeunload'; message: string; default_prompt?: string })
  | (RequestBase & { kind: 'file_chooser'; multiple: boolean; accept: string[] })
  | (RequestBase & { kind: 'download'; download_id: string; name: string; size: number; state: 'completed' | 'failed' })
  | (RequestBase & { kind: 'permission'; permission: string; origin: string })

export interface FrameMeta { seq: number; tab_id: string; width: number; height: number }
export interface ScreenFrame extends FrameMeta { data: ArrayBuffer }

/** `focus`: the element of the current tab that has the focus changed; `editable` ones take text. */
export interface ScreenFocus {
  tab_id: string
  editable: boolean
  /** The `type` of an `input`, or `textarea` / `contenteditable`. */
  input_type?: string
  rect?: { x: number; y: number; width: number; height: number }
}

export type ScreenDownstream =
  | { type: 'ready'; state: ScreenState }
  | ({ type: 'frame_meta' } & FrameMeta)
  | ({ type: 'state' } & ScreenState)
  | ({ type: 'request' } & ScreenRequest)
  | { type: 'request_closed'; request_id: string }
  | ({ type: 'focus' } & ScreenFocus)
  | { type: 'error'; code: string; message?: string; unavailable_code?: UnavailableCode }
  | { type: 'pong' }

export const MODIFIER = { alt: 1, ctrl: 2, meta: 4, shift: 8 } as const
export const VIEWPORT_MIN = 320
export const VIEWPORT_MAX = 2560
export const DEFAULT_GROUP = 'default'

/** `code` of a `browser.*` error: the wire error is `error_to_wire`'s JSON inside the RPC error's message. */
export function browserErrorCode(error: unknown): string | null {
  const direct = (error as { code?: unknown } | null)?.code
  if (typeof direct === 'string' && direct !== 'transport' && direct !== 'http_error') return direct
  const message = error instanceof Error ? error.message : String(error ?? '')
  const start = message.indexOf('{')
  if (start < 0) return null
  try {
    const wire = JSON.parse(message.slice(start)) as { code?: unknown }
    return typeof wire.code === 'string' ? wire.code : null
  } catch {
    return null
  }
}
