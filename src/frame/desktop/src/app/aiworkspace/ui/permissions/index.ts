/* The permission views (UI 框架 PRD §8) for the hosts that embed them:
 * - `WorkspacePermissionEditor` — the Workspace's editor, the content of a data-mode tab (`audit` adds the record);
 * - `EntityPermissions` — one object's permissions, for the right sidebar's "权限" tab and the canvas side drawer;
 * - `MacroIdentity` — a macro's identity (bound group, union / intersection), for the macro's "权限" tab;
 * - `AclAuditList` — the operation record, for the right sidebar ("修改记录" of the editor's tab);
 * - `MyPermissions` — the read-only view of one's own permissions (the Workspace, or one object);
 * - `openPermissions` / `PermissionsHost` — the shell's way in: `{ kind: 'workspace' }` or `{ kind: 'entity', entityId }`. */

export { WorkspacePermissionEditor, type WorkspacePermissionEditorProps } from './WorkspacePermissionEditor'
export { EntityPermissions, type EntityPermissionsProps } from './EntityPermissions'
export { MacroIdentity } from './MacroIdentity'
export { AclAuditList, type AclAuditListProps } from './AclAuditList'
export { MyPermissions, type MyPermissionsProps } from './MyPermissions'
export { EntityPermissionsDrawer, PermissionsHost } from './PermissionsHost'
export { openPermissions, permissionRequests, type PermissionRequest } from './requests'
export { useAclRev } from './aclData'
