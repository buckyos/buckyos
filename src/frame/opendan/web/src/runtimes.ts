import type { TextKey } from './i18n'
import type { KnownRuntime, PauseSource, RegistryEntry, RuntimeSettings } from './model'

export const inputClass = 'mt-1 block min-h-11 w-full rounded-xl border border-line bg-bg px-3 text-base text-fg'

// Inside the desktop's app window the page is framed: a link to the desktop leaves the frame.
export const outerTarget = window.top === window.self ? undefined : '_top'

export const runtimeAvailability: Record<KnownRuntime['availability'], TextKey> = {
  available: 'runtimeAvailable',
  runtime_unavailable: 'runtimeUnavailable',
  permission_denied: 'runtimePermissionDenied',
}

const WRITE_OPS = ['exec', 'write', 'edit', 'mkdir', 'rename', 'remove']
export const isReadOnly = (runtime: KnownRuntime) => runtime.grant.ops.length > 0 && !runtime.grant.ops.some((op) => WRITE_OPS.includes(op))
export const mayHoldWorkspaces = (runtime: KnownRuntime) =>
  runtime.lifecycle === 'active' && runtime.enabled && runtime.availability === 'available' && !isReadOnly(runtime) && runtime.grant.expires_at_ms == null
export const isExpired = (runtime: KnownRuntime) => runtime.grant.expires_at_ms != null && runtime.grant.expires_at_ms <= Date.now()

/** The id a runtime has in the owner's settings (the private one shared with the owner's machine: its alias). */
export const settingIdOf = (runtime: KnownRuntime) =>
  runtime.connector.type === 'native' ? (runtime.connector.aliases?.[0] ?? runtime.runtime_id) : runtime.runtime_id

export type RuntimeRole = 'provider' | 'owner_default' | 'assigned'

export const runtimeRole = (runtime: KnownRuntime, settings?: RuntimeSettings): RuntimeRole =>
  runtime.connector.type === 'native' ? 'provider' : settings?.configured_default_runtime_id === settingIdOf(runtime) ? 'owner_default' : 'assigned'

export const pauseLabel: Record<PauseSource, TextKey> = {
  user: 'pausedByUser',
  parent: 'pausedWithParent',
  interrupted: 'pausedInterrupted',
  goal: 'pausedWithGoal',
}

/** The interrupted sessions whose parent is not interrupted: resuming them resumes the rest. */
export const topInterrupted = (entries: RegistryEntry[]): RegistryEntry[] => {
  const interrupted = new Set(entries.filter((e) => e.status.paused === 'interrupted').map((e) => e.session_id))
  return entries.filter((e) => interrupted.has(e.session_id) && !(e.origin?.parent_session && interrupted.has(e.origin.parent_session)))
}
