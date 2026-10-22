/**
 * NFSP permission source: nfs-server get_acl / set_acl / list_acls. Users for
 * the principal picker come from control-panel `user.list`.
 */

import type { AclEntry, AclInfo, AclRecordInfo } from '../../../../api/nfsp_client'
import { fetchUserList } from '../../../../api/user_mgr'
import { registerPermissionSource } from '../permissions'
import type { AclRecordView, AclView, PrincipalOption } from '../permissions'
import { ensureSession, nfspClient } from './client'
import { nfspToUiError } from './errors'
import { notifyDfsPath } from './invalidation'
import { cyfsToDfsPath, unixToIso } from './mapping'
import { parseUserType } from '../../../../api/user_type'

const dfs = (canonical: string) => cyfsToDfsPath(canonical) ?? canonical
const parentOf = (path: string) => path.slice(0, path.lastIndexOf('/')) || '/'

async function run<T>(op: () => Promise<T>): Promise<T> {
  try {
    await ensureSession()
    return await op()
  } catch (err) {
    throw nfspToUiError(err)
  }
}

function viewOf(info: AclInfo): AclView {
  return {
    path: dfs(info.path),
    accessLevel: info.access_level,
    canManage: info.can_manage,
    revision: info.revision,
    inherit: info.inherit ?? true,
    entries: info.entries ?? [],
    inherited: (info.inherited ?? []).map((e) => ({ ...e, from: dfs(e.from) })),
    builtin: info.builtin ?? [],
    updatedBy: info.updated_by,
    updatedAt: info.updated_at ? unixToIso(info.updated_at) : undefined,
  }
}

function recordOf(r: AclRecordInfo): AclRecordView {
  return {
    path: dfs(r.path),
    inherit: r.inherit,
    entries: r.entries,
    homeOwner: r.home_owner,
    updatedBy: r.updated_by,
    updatedAt: unixToIso(r.updated_at),
    revision: r.revision,
    missing: r.missing,
  }
}

export function registerNfspPermissionSource() {
  return registerPermissionSource({
    getAcl: (path) => run(async () => viewOf(await nfspClient().raw.getAcl(path))),
    setAcl: (path, acl, expectedRevision) =>
      run(async () => {
        await nfspClient().raw.setAcl(path, { inherit: acl.inherit, entries: acl.entries as AclEntry[] }, { expectedRevision })
        // Visibility and access levels of the listing changed.
        notifyDfsPath(parentOf(path))
        notifyDfsPath(path)
      }),
    listAcls: () =>
      run(async () => {
        const r = await nfspClient().raw.listAcls()
        return { records: r.records.map(recordOf), isAdmin: r.is_admin, zoneOwner: r.zone_owner }
      }),
    async listUsers(): Promise<PrincipalOption[]> {
      const { data } = await fetchUserList()
      return (data?.users ?? [])
        .filter((u) => parseUserType(u.user_type) !== 'root' && !String(u.state).startsWith('deleted'))
        .map((u) => ({ principal: `user:${u.user_id}`, label: u.show_name || u.user_id, detail: u.user_id }))
    },
  })
}

