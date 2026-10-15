/**
 * File permissions (path ACLs) — the data-layer seam shared by the File
 * Browser permissions dialog and the Settings "File permissions" page.
 * The NFSP adapter talks to nfs-server get_acl/set_acl/list_acls; the mock
 * keeps an in-memory table so the UI runs without a backend.
 */

/** Effective level; `traverse` = only the way to something shared deeper down. */
export type AccessLevel = 'none' | 'traverse' | 'read' | 'write' | 'manage'
export type GrantLevel = 'read' | 'write' | 'manage'

/** `user:<user_id>`, `zone_users` (any signed-in user) or `public` (anyone, read only). */
export type Principal = string
export const PUBLIC_PRINCIPAL = 'public'
export const ZONE_USERS_PRINCIPAL = 'zone_users'
export const userPrincipal = (userId: string) => `user:${userId}`

export interface AclGrant {
  principal: Principal
  level: GrantLevel
}

export interface AclView {
  /** DFS path, e.g. `/home/alice/docs`. */
  path: string
  accessLevel: AccessLevel
  canManage: boolean
  revision: string
  /** The fields below are only known to callers who can manage. */
  inherit: boolean
  entries: AclGrant[]
  inherited: (AclGrant & { from: string })[]
  builtin: (AclGrant & { reason: 'zone_owner' | 'home_owner' })[]
  updatedBy?: string
  updatedAt?: string
}

export interface AclRecordView {
  path: string
  inherit: boolean
  entries: AclGrant[]
  homeOwner: string | null
  updatedBy: string
  updatedAt: string
  revision: string
  missing: boolean
}

export interface AclRecordList {
  records: AclRecordView[]
  isAdmin: boolean
  zoneOwner: string | null
}

export interface PrincipalOption {
  principal: Principal
  label: string
  detail?: string
}

export interface PermissionSource {
  getAcl(path: string): Promise<AclView>
  /** Replaces the item's own entries; inherit + no entries removes the record. */
  setAcl(path: string, acl: { inherit: boolean; entries: AclGrant[] }, expectedRevision?: string): Promise<void>
  listAcls(): Promise<AclRecordList>
  /** Zone users that can be granted access (the caller included). */
  listUsers(): Promise<PrincipalOption[]>
}

const LEVEL_RANK: Record<AccessLevel, number> = { none: 0, traverse: 1, read: 2, write: 3, manage: 4 }
export const levelAtLeast = (level: AccessLevel | undefined, min: AccessLevel) => LEVEL_RANK[level ?? 'none'] >= LEVEL_RANK[min]

let active: PermissionSource | null = null
export function registerPermissionSource(source: PermissionSource): () => void {
  active = source
  return () => {
    if (active === source) active = null
  }
}
export function permissionSource(): PermissionSource | null {
  return active
}
