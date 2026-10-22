/* ── What the signed-in account may do with its address book, and the names of system user groups ──
 *
 * `user.get` says whether the account may edit its own address book (Limited accounts may not by
 * default; an admin can allow it). Without that right the address book is the system projection only
 * and every editing entry is hidden; msg-center refuses the writes anyway. `user_group.list` is
 * admin-only: anyone else shows projection groups by their id or type name.
 */

import { useEffect, useState } from 'react'
import { fetchUserDetail, fetchUserGroups } from '../../api/user_mgr'
import type { Contact } from './types'

let editPermission: Promise<boolean> | null = null
let groupNames: Promise<ReadonlyMap<string, string>> | null = null

function loadEditPermission(): Promise<boolean> {
  editPermission ??= fetchUserDetail()
    .then(({ data }) => data?.allow_contact_edit !== false)
    .catch(() => true)
  return editPermission
}

function loadGroupNames(): Promise<ReadonlyMap<string, string>> {
  groupNames ??= fetchUserGroups()
    .then(({ data }) => new Map((data?.groups ?? []).map((group) => [group.group_id, group.name || group.group_id])))
    .catch(() => new Map<string, string>())
  return groupNames
}

/** Forget the cached answers (another account signed in, or an admin changed them). */
export function resetContactPermissionCache() {
  editPermission = null
  groupNames = null
}

/** True until `user.get` says otherwise, so a slow answer never hides the address book. */
export function useContactEditPermission(): boolean {
  const [canEdit, setCanEdit] = useState(true)
  useEffect(() => {
    let cancelled = false
    void loadEditPermission().then((value) => { if (!cancelled) setCanEdit(value) })
    return () => { cancelled = true }
  }, [])
  return canEdit
}

export function useUserGroupNames(): ReadonlyMap<string, string> {
  const [names, setNames] = useState<ReadonlyMap<string, string>>(() => new Map())
  useEffect(() => {
    let cancelled = false
    void loadGroupNames().then((value) => { if (!cancelled) setNames(value) })
    return () => { cancelled = true }
  }, [])
  return names
}

/** A system projection entry: read-only, put there by the admin's projection rules. */
export function isProjectedContact(contact: Pick<Contact, 'projected_from'> | undefined): boolean {
  return (contact?.projected_from?.length ?? 0) > 0
}
