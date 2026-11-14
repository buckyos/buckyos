/* What the Block menu asks of other parts of the window (UI 框架 PRD §10.5), and what asks for the Block menu, without
 * depending on them.
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

// ---- the Block's whole menu asked for by its content (a data control's title row ⋯, PRD §6.2, §10.3)

type MenuListener = (blockId: string, anchor: HTMLElement) => void
const menuListeners = new Set<MenuListener>()

/** Register the host that opens a Block's menu (the canvas); returns the unregister function. The newest one answers. */
export function onOpenBlockMenu(listener: MenuListener): () => void {
  menuListeners.add(listener)
  return () => { menuListeners.delete(listener) }
}

/** Open the menu of Block `blockId` under `anchor`; false when no host shows Block menus here (a control's own page). */
export function openBlockMenu(blockId: string, anchor: HTMLElement): boolean {
  const last = [...menuListeners].at(-1)
  if (!last) return false
  last(blockId, anchor)
  return true
}
