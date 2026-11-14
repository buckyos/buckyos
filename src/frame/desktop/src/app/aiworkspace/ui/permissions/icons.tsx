/* Icons of what the permission views show (§8.3): a resource by the icon of its type, a member by where it comes from,
 * a subject by its kind. */

import { AppWindow, Bot, Boxes, Building2, Contact, File, FileText, Folder, FolderOpen, Globe, Image, LayoutDashboard, Link2, ListChecks, MessageSquare, Puzzle, Sparkles, Square, Table2, User, UserRound, Users } from 'lucide-react'
import type { EntityEnvelope } from '../../api/types'
import { OWNER, PUB_PREFIX, type MemberKind } from './model'
import './permissions.css'

const TYPE_ICON: Record<string, typeof File> = {
  'buckyos.table-source': Table2,
  'buckyos.richtext': FileText,
  'buckyos.record': ListChecks,
  'buckyos.asset-ref': Image,
  'buckyos.dir': FolderOpen,
  'buckyos.wish': Sparkles,
  'buckyos.block-def': Puzzle,
  'buckyos.annotation': MessageSquare,
  'buckyos.cell': Square,
}

/** The icon of an entity's type (a canvas, a folder, a table…); the Workspace itself for none. */
export function TypeIcon({ entity, size = 16 }: { entity: EntityEnvelope | null | undefined; size?: number }) {
  const Icon = !entity ? Globe
    : entity.kind === 'surface' ? LayoutDashboard
      : entity.type_id === 'buckyos.container' ? Folder
        : TYPE_ICON[entity.type_id] ?? File
  return <Icon size={size} aria-hidden="true" className="aiws-perm-type-icon" />
}

const MEMBER_ICON: Record<MemberKind, typeof File> = { user: User, agent: Bot, did: UserRound, sysgroup: Users, contact: Contact, zone: Building2, guest: UserRound, link: Link2 }

export function MemberIcon({ kind, size = 16 }: { kind: MemberKind; size?: number }) {
  const Icon = MEMBER_ICON[kind]
  return <Icon size={size} aria-hidden="true" />
}

export function SubjectIcon({ subject, size = 16 }: { subject: string; size?: number }) {
  const Icon = subject.startsWith(PUB_PREFIX) ? AppWindow : subject.startsWith('@') ? (subject === OWNER ? User : Boxes) : User
  return <Icon size={size} aria-hidden="true" />
}
