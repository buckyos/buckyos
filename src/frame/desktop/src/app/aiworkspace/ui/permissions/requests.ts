/* What the shell asks of the permission views (UI 框架 PRD §8.1): open the Workspace's permission editor, or one
 * object's permissions in the side drawer without leaving the canvas. `PermissionsHost` (mounted once at the workspace
 * shell) answers; menus, the object tree's "权限…", a Block's "权限…" and the share dialog only call `openPermissions`. */

import { Emitter } from '../../state/emitter'

export type PermissionRequest =
  /** The Workspace's permission editor: the data mode with the editor open (a full-screen sheet on a phone). */
  | { kind: 'workspace' }
  /** One object's permissions (§8.4) in the side drawer; `alsoId`: a second object the drawer offers (the Block showing
   * the data, or the data a Block shows). */
  | { kind: 'entity'; entityId: string; alsoId?: string | null }

class PermissionRequests {
  private request: PermissionRequest | null = null
  private readonly emitter = new Emitter()
  readonly subscribe = this.emitter.subscribe
  snapshot = () => this.request
  open(request: PermissionRequest | null) {
    this.request = request
    this.emitter.emit()
  }
}

export const permissionRequests = new PermissionRequests()

/** Open the permission editor or an object's permissions; `null` closes the drawer. */
export function openPermissions(request: PermissionRequest | null) {
  permissionRequests.open(request)
}
