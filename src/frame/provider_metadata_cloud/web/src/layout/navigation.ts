import {
  Activity,
  Boxes,
  ClipboardList,
  Database,
  FolderTree,
  GitCompare,
  History,
  Import,
  LibraryBig,
  PenLine,
  Route,
  ServerCog,
  TriangleAlert,
  type LucideIcon,
} from 'lucide-react'
import type { ServiceRole } from '../datamodel/types'

export interface NavItem {
  id: string
  path: string
  labelKey: string
  icon: LucideIcon
  roles: ServiceRole[]
}

export const navigationItems: NavItem[] = [
  { id: 'dashboard', path: '/', labelKey: 'nav.dashboard', icon: Activity, roles: ['tech', 'ops'] },
  { id: 'tech-source', path: '/tech-source', labelKey: 'nav.techSource', icon: ServerCog, roles: ['ops'] },
  { id: 'providers', path: '/providers', labelKey: 'nav.providers', icon: Database, roles: ['tech', 'ops'] },
  { id: 'models', path: '/models', labelKey: 'nav.models', icon: Boxes, roles: ['tech', 'ops'] },
  { id: 'nick-rules', path: '/nick-rules', labelKey: 'nav.nickRules', icon: PenLine, roles: ['tech'] },
  { id: 'resolver-rules', path: '/resolver-rules', labelKey: 'nav.resolverRules', icon: Route, roles: ['tech', 'ops'] },
  { id: 'logical-directory', path: '/logical-directory', labelKey: 'nav.logicalDirectory', icon: FolderTree, roles: ['tech'] },
  { id: 'dictionaries', path: '/dictionaries', labelKey: 'nav.dictionaries', icon: LibraryBig, roles: ['tech'] },
  { id: 'import-plan', path: '/import-plan', labelKey: 'nav.importPlan', icon: Import, roles: ['tech'] },
  { id: 'bulk', path: '/bulk-operations', labelKey: 'nav.bulkOperations', icon: ClipboardList, roles: ['ops'] },
  { id: 'warnings', path: '/warnings', labelKey: 'nav.warnings', icon: TriangleAlert, roles: ['ops'] },
  { id: 'publish', path: '/publish', labelKey: 'nav.publish', icon: GitCompare, roles: ['tech', 'ops'] },
  { id: 'change-logs', path: '/change-logs', labelKey: 'nav.changeLogs', icon: History, roles: ['tech', 'ops'] },
]

export function getNavigationItems(role: ServiceRole) {
  return navigationItems.filter((item) => item.roles.includes(role))
}
