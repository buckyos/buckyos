/* What the Block menu asks of other parts of the window (UI 框架 PRD §10.5) without depending on them.
 *
 * "Permissions…" (`block.permissions`): the per-entity permissions view (PRD §8.4) belongs to the permissions editor. It
 * registers itself with `onOpenPermissions`; until one is registered, the request falls back to what the canvas offers now
 * (the right panel's collaboration tab, the workspace-level grants). */

export interface PermissionsRequest {
  /** The entity whose permissions are shown: the Block, or the data it shows when `dataId` is given. */
  entityId: string
  /** The data entity the Block shows, if any (the view may offer both). */
  dataId: string | null
  /** The person has no `manage` there: only "my permissions" is shown, read-only. */
  readOnly: boolean
}

type Listener = (request: PermissionsRequest) => void
const listeners = new Set<Listener>()

/** Register the per-entity permissions view; returns the unregister function. The newest registration answers. */
export function onOpenPermissions(listener: Listener): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

/** Ask for the permissions view of an entity; false when no view is registered (the caller falls back). */
export function openPermissions(request: PermissionsRequest): boolean {
  const last = [...listeners].at(-1)
  if (!last) return false
  last(request)
  return true
}
