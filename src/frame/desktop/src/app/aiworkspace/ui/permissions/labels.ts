/* Names of what the permission views show (§8.2, §8.3): subjects and member specs by name — never the raw id when a
 * name is known — and the type of a resource in words. */

import { useCallback } from 'react'
import type { AclGroup, AclResource, EntityEnvelope } from '../../api/types'
import { useOutlineVersion, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { TYPE_LABEL, entityLabel } from '../sources/dataOps'
import { useAcl, useAclRead, useDirectory, type Directory } from './aclData'
import { ALL, CREATOR, GUEST, OWNER, PUB_PREFIX, memberKind, type MemberKind } from './model'

export interface Labels {
  /** A subject of an entry: a single caller, `@<group>`, a fixed group, a publication. */
  subject(subject: string): string
  /** A member spec of a logical group. */
  member(member: string): string
  memberKind(member: string): MemberKind
  /** A resource: the entity's name (`/` the whole Workspace). */
  resource(resource: AclResource): string
  entity(id: string | null): EntityEnvelope | undefined
  groups: AclGroup[]
  directory: Directory
}

/** The names the permission views use; re-rendered with the outline and the groups. */
export function useLabels(): Labels {
  const { t } = useAiwsText()
  const store = useStore()
  useOutlineVersion()
  const { api } = useAcl()
  const directory = useDirectory()
  const groups = useAclRead('groups', api.groups).data?.groups ?? []
  const userName = useCallback((id: string) => directory.userName(id), [directory])
  const member = (m: string): string => {
    switch (memberKind(m, directory.agents)) {
      case 'zone': return t('aiworkspace.perm.member.zone')
      case 'sysgroup': return directory.sysGroupName(m.slice(6)) ?? m.slice(6)
      case 'contact': {
        const [owner, tag] = m.slice(8).split('#')
        return t('aiworkspace.perm.member.contactName', { tag: tag ?? '', owner: owner ?? '' })
      }
      case 'guest': return t('aiworkspace.perm.member.guestName', { id: m.slice(6, 12) })
      case 'link': return t('aiworkspace.perm.member.linkName')
      default: return userName(m) ?? m
    }
  }
  const subject = (s: string): string => {
    if (s === OWNER) return t('aiworkspace.perm.fixed.owner')
    if (s === ALL) return t('aiworkspace.perm.fixed.all')
    if (s === GUEST) return t('aiworkspace.perm.fixed.guest')
    if (s === CREATOR) return t('aiworkspace.perm.fixed.creator')
    if (s.startsWith(PUB_PREFIX)) {
      const target = store.outline.get(s.slice(PUB_PREFIX.length))
      return t('aiworkspace.perm.publication', { name: target ? entityLabel(target) : s.slice(PUB_PREFIX.length) })
    }
    if (s.startsWith('@')) return groups.find((g) => g.subject === s)?.title || s.slice(1)
    return member(s)
  }
  return {
    subject,
    member,
    memberKind: (m) => memberKind(m, directory.agents),
    resource: (r) => {
      if (!r.entity_id) return t('aiworkspace.perm.wholeWorkspace')
      const e = store.outline.get(r.entity_id)
      return e ? entityLabel(e) : r.path
    },
    entity: (id) => (id ? store.outline.get(id) : undefined),
    groups,
    directory,
  }
}

/** The type of an entity in words (or "workspace"). */
export function useTypeName(): (entity: EntityEnvelope | null | undefined) => string {
  const { t } = useAiwsText()
  return (entity) => (!entity ? t('aiworkspace.perm.wholeWorkspace') : entity.kind === 'surface' ? t('aiworkspace.perm.type.surface') : TYPE_LABEL[entity.type_id] ?? entity.type_id)
}
