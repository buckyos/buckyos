import { operationError } from '../data/folderOps'
import { mockDelay } from '../data/mockReader'
import { registerPermissionSource } from '../data/permissions'
import type { AclGrant, AclRecordView } from '../data/permissions'
import { mockEntryByPath } from './data'

/** The mock signs in as the zone owner (control_panel_mock account alice/admin). */
const ZONE_OWNER = 'alice'
const users = [
  { principal: 'user:alice', label: 'Alice', detail: 'alice' },
  { principal: 'user:bob', label: 'Bob', detail: 'bob' },
  { principal: 'user:carol', label: 'Carol', detail: 'carol' },
]

interface MockRecord { inherit: boolean; entries: AclGrant[]; updatedBy: string; updatedAt: string; rev: number }
const records = new Map<string, MockRecord>([
  ['/public', { inherit: true, entries: [{ principal: 'public', level: 'read' }], updatedBy: 'user:alice', updatedAt: '2026-10-01T08:00:00.000Z', rev: 1 }],
  ['/shared', { inherit: true, entries: [{ principal: 'zone_users', level: 'write' }], updatedBy: 'user:alice', updatedAt: '2026-10-02T08:00:00.000Z', rev: 1 }],
  ['/home/Projects', { inherit: true, entries: [{ principal: 'user:bob', level: 'write' }, { principal: 'user:carol', level: 'read' }], updatedBy: 'user:alice', updatedAt: '2026-10-03T08:00:00.000Z', rev: 1 }],
])

const parentOf = (path: string) => (path === '/' ? null : path.slice(0, path.lastIndexOf('/')) || '/')
const revisionOf = (path: string) => `mock-${records.get(path)?.rev ?? 0}`

function inheritedFor(path: string) {
  const out: (AclGrant & { from: string })[] = []
  if (records.get(path)?.inherit === false) return out
  for (let cur = parentOf(path); cur; cur = parentOf(cur)) {
    const rec = records.get(cur)
    if (!rec) continue
    out.push(...rec.entries.map((entry) => ({ ...entry, from: cur! })))
    if (!rec.inherit) break
  }
  return out
}

export function registerMockPermissionSource() {
  return registerPermissionSource({
    async getAcl(path) {
      await mockDelay(20, 60)
      if (path !== '/' && !mockEntryByPath(path) && !records.has(path)) throw operationError('NOT_FOUND', 'This item no longer exists')
      const rec = records.get(path)
      return {
        path,
        accessLevel: 'manage',
        canManage: true,
        revision: revisionOf(path),
        inherit: rec?.inherit ?? true,
        entries: rec?.entries.map((entry) => ({ ...entry })) ?? [],
        inherited: inheritedFor(path),
        builtin: [{ principal: `user:${ZONE_OWNER}`, level: 'manage', reason: 'zone_owner' }],
        updatedBy: rec?.updatedBy,
        updatedAt: rec?.updatedAt,
      }
    },
    async setAcl(path, acl, expectedRevision) {
      await mockDelay(20, 60)
      if (expectedRevision && expectedRevision !== revisionOf(path)) throw operationError('REVISION_MISMATCH', 'Permissions changed elsewhere. Reload and try again.')
      if (acl.entries.some((entry) => entry.principal === 'public' && entry.level !== 'read')) throw operationError('INVALID_ARGUMENT', 'Public access can only be read')
      const rev = (records.get(path)?.rev ?? 0) + 1
      if (acl.inherit && acl.entries.length === 0) records.delete(path)
      else records.set(path, { inherit: acl.inherit, entries: acl.entries.map((entry) => ({ ...entry })), updatedBy: `user:${ZONE_OWNER}`, updatedAt: new Date().toISOString(), rev })
    },
    async listAcls() {
      await mockDelay(20, 60)
      const list: AclRecordView[] = [...records.entries()]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([path, rec]) => ({ path, inherit: rec.inherit, entries: rec.entries.map((entry) => ({ ...entry })), homeOwner: null, updatedBy: rec.updatedBy, updatedAt: rec.updatedAt, revision: revisionOf(path), missing: false }))
      return { records: list, isAdmin: true, zoneOwner: ZONE_OWNER }
    },
    async listUsers() {
      return users.map((user) => ({ ...user }))
    },
  })
}
