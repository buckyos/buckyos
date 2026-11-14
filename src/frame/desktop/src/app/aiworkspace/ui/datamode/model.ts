/* What the data mode shows of the outline (UI 框架 PRD §5.2): the object tree's default (the five data source types and
 * the folders holding them; the canvas content folded; macros and the Workspace's permissions pinned below), the
 * filters (types, origin, everything), the search that keeps the levels, each row's type icon and state marks, and the
 * read-only reason of an object. Non-visual helpers only. */

import {
  Box, CalendarClock, Clapperboard, File, FileText, Folder, FolderOpen, Image, LayoutDashboard, ListChecks, MessageSquare, MessagesSquare,
  Puzzle, ScanEye, ShieldCheck, Sparkles, Square, Table2, Vote, type LucideIcon,
} from 'lucide-react'
import { pathOf } from '../../api/paths'
import { CANVAS_CONTENT_ID, DATA_ID, SHOWS_ID, SURFACES_ID, type EntityEnvelope, type Json } from '../../api/types'
import { aiwsT } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'
import { TYPE_LABEL, entityLabel } from '../sources/dataOps'
import type { SideTab } from './actions'
import { PERMISSIONS_TAB } from './tabs'

/** The right sidebar's current tab (the person's state). */
export const SIDE_TAB_KEY = 'ui:data-side-tab'

/** The tabs the sidebar has for the current tab (only the record beside the permission editor, §5.4). */
export function sideTabsOf(id: string | null): SideTab[] {
  return id === PERMISSIONS_TAB ? ['history'] : ['properties', 'permissions', 'history']
}

/** The data source objects (术语表, §5.2): what the tree shows by default. */
export const DATA_SOURCE_TYPES: ReadonlySet<string> = new Set(['buckyos.table-source', 'buckyos.richtext', 'buckyos.record', 'buckyos.asset-ref', 'buckyos.dir'])
/** The types the filter offers (§5.2: the defaults, then what "显示全部对象" adds). */
export const FILTER_TYPES = ['buckyos.table-source', 'buckyos.richtext', 'buckyos.record', 'buckyos.asset-ref', 'buckyos.dir', 'buckyos.wish', 'buckyos.block-def', 'buckyos.annotation', 'buckyos.show-path', 'buckyos.viewport']
/** The session types of a presentation's Workspace (第四期规划 §5.2): a name, no editor of their own. */
export const SESSION_TYPES = ['buckyos.task', 'buckyos.response', 'buckyos.result']
/** The system folder of a session's data (core `SESSION_ID`). */
export const SESSION_FOLDER = 'session'

export type Origin = 'all' | 'original' | 'generated'
export interface TreeFilter { types: string[]; origin: Origin; all: boolean }
export const FILTER_KEY = 'ui:data-filter'
export const DEFAULT_FILTER: TreeFilter = { types: [], origin: 'all', all: false }

export function parseFilter(value: Json | undefined): TreeFilter {
  const raw = value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, Json> : {}
  return {
    types: Array.isArray(raw.types) ? raw.types.filter((t): t is string => typeof t === 'string') : [],
    origin: raw.origin === 'original' || raw.origin === 'generated' ? raw.origin : 'all',
    all: raw.all === true,
  }
}

export function filtering(filter: TreeFilter): boolean {
  return filter.types.length > 0 || filter.origin !== 'all' || filter.all
}

const ICONS: Record<string, LucideIcon> = {
  'buckyos.table-source': Table2, 'buckyos.richtext': FileText, 'buckyos.record': ListChecks, 'buckyos.asset-ref': Image, 'buckyos.dir': FolderOpen,
  'buckyos.wish': Sparkles, 'buckyos.block-def': Puzzle, 'buckyos.annotation': MessageSquare, 'buckyos.cell': Square, 'buckyos.show-path': Clapperboard,
  'buckyos.viewport': ScanEye, 'buckyos.task': Vote, 'buckyos.response': MessagesSquare, 'buckyos.result': CalendarClock,
}

/** The icon of an object's type (a folder, a canvas, a table…); the Workspace's permissions for its node. */
export function iconOf(entity: EntityEnvelope | undefined, id?: string): LucideIcon {
  if (id === PERMISSIONS_TAB) return ShieldCheck
  if (!entity) return File
  if (entity.kind === 'surface') return LayoutDashboard
  if (entity.type_id === 'buckyos.container') return entity.kind === 'folder' || entity.system ? Folder : Box
  return ICONS[entity.type_id] ?? File
}

/** A type in words (the session types and the folders included). */
export function typeName(entity: EntityEnvelope | undefined): string {
  if (!entity) return ''
  if (entity.kind === 'surface') return aiwsT('aiworkspace.data.canvas')
  const session = SESSION_TYPES.indexOf(entity.type_id)
  if (session >= 0) return aiwsT(`aiworkspace.datamode.type.${['task', 'response', 'result'][session]}`)
  return TYPE_LABEL[entity.type_id] ?? entity.type_id
}

/** A row's or a tab's name: the system folders by what they are. */
export function nameOf(store: WorkspaceStore, entity: EntityEnvelope | undefined, id: string): string {
  if (id === PERMISSIONS_TAB) return aiwsT('aiworkspace.datamode.permissions')
  if (!entity) return id
  if (entity.entity_id === CANVAS_CONTENT_ID) return aiwsT('aiworkspace.datamode.canvasData')
  if (entity.entity_id === SHOWS_ID) return aiwsT('aiworkspace.data.tree.showPaths')
  if (entity.entity_id === SESSION_FOLDER) return aiwsT('aiworkspace.datamode.sessionData')
  if (entity.entity_id === SURFACES_ID) return aiwsT('aiworkspace.datamode.surfaces')
  if (entity.system === 'surface_content' && entity.surface_id) {
    const surface = store.outline.get(entity.surface_id)
    if (surface) return entityLabel(surface)
  }
  return entityLabel(entity)
}

/** The path of an object as Agents and permission rules use it (`/data/销售`). */
export function pathText(store: WorkspaceStore, id: string): string {
  return pathOf(store.outline, id)
}

/** A user folder of the data tree (not a system folder, not a canvas group). */
export function isFolder(entity: EntityEnvelope | undefined): boolean {
  return Boolean(entity && entity.type_id === 'buckyos.container' && (entity.kind === 'folder' || entity.system === 'surface_content' || entity.system === 'canvas_content' || entity.system === 'shows' || entity.entity_id === SESSION_FOLDER))
}

/** A folder new data may go into: the data root, a user folder, a canvas's own content folder (not the presentation
 * folder, the canvas-content area itself or a session's folder). */
export function acceptsData(entity: EntityEnvelope | undefined): boolean {
  if (!entity) return false
  if (entity.entity_id === DATA_ID) return true
  return entity.type_id === 'buckyos.container' && (entity.kind === 'folder' || entity.system === 'surface_content') && entity.entity_id !== SHOWS_ID && entity.entity_id !== CANVAS_CONTENT_ID && entity.entity_id !== SESSION_FOLDER
}

/** Which entities the tree shows and which folders open by themselves (the search keeps the levels). */
export interface TreeView {
  /** Shown rows. */
  shown: ReadonlySet<string>
  /** Matches of the search or the filters themselves (not only their containers). */
  hits: ReadonlySet<string>
  searching: boolean
}

/** Not shown unless the filters ask for it: what is not a data source (a wish with a macro is listed under "宏"), the
 * presentation and session folders, canvas groups. */
function hiddenByDefault(entity: EntityEnvelope): boolean {
  if (entity.type_id === 'buckyos.container') return entity.entity_id === SHOWS_ID || entity.entity_id === SESSION_FOLDER || (entity.kind !== 'folder' && !entity.system)
  return !DATA_SOURCE_TYPES.has(entity.type_id)
}

/** One pass over the tree under `data` (and `surfaces` with "everything"): the rows to show for a filter and a query. */
export function treeView(store: WorkspaceStore, filter: TreeFilter, query: string): TreeView {
  const q = query.trim().toLowerCase()
  const shown = new Set<string>()
  const hits = new Set<string>()
  const typed = filter.types.length > 0
  const own = (e: EntityEnvelope): boolean => {
    if (typed ? !filter.types.includes(e.type_id) : !filter.all && hiddenByDefault(e)) return false
    if (filter.origin === 'generated' && !e.derived) return false
    if (filter.origin === 'original' && e.derived) return false
    if (q && !nameOf(store, e, e.entity_id).toLowerCase().includes(q) && !(e.name ?? '').toLowerCase().includes(q) && !e.entity_id.toLowerCase().includes(q)) return false
    return true
  }
  const narrowed = typed || filter.origin !== 'all' || q !== ''
  const walk = (parentId: string): boolean => {
    let any = false
    for (const child of store.outline.childrenOf(parentId)) {
      if (child.type_id === 'buckyos.cell' && !filter.all) continue
      const below = walk(child.entity_id)
      const folder = isFolder(child)
      // a user folder (and the canvas content area) stands on its own unless the filters narrow the tree; a system folder
      // that is hidden by default (presentations, a session) only with "everything"
      const stands = folder && !narrowed && (filter.all || !hiddenByDefault(child))
      const hit = own(child)
      if (hit) hits.add(child.entity_id)
      if (hit || below || stands) { shown.add(child.entity_id); any = true }
    }
    return any
  }
  walk(DATA_ID)
  if (filter.all) {
    shown.add(SURFACES_ID)
    walk(SURFACES_ID)
  }
  return { shown, hits, searching: q !== '' }
}

/** Why the object cannot be changed here, or null (§5.3: the data layer's limits hold in the data mode too). */
export function readOnlyReason(store: WorkspaceStore, entity: EntityEnvelope | undefined, windowReason: string | null): string | null {
  if (!entity) return null
  if (windowReason) return windowReason
  if (entity.source) return aiwsT('aiworkspace.datamode.readOnly.source')
  if (!entity.capabilities.includes('update') && !entity.capabilities.includes('append')) return aiwsT('aiworkspace.datamode.readOnly.noPermission')
  if (entity.write_policy === 'lock_required' && !store.locks.isHeld(entity.entity_id)) {
    return entity.lock_holder ? aiwsT('aiworkspace.datamode.readOnly.lockedBy', { principal: entity.lock_holder.principal }) : aiwsT('aiworkspace.datamode.readOnly.lock')
  }
  return null
}

/** The Blocks that show an object, with the canvas each is on. */
export function blocksShowing(store: WorkspaceStore, entityId: string): { block: EntityEnvelope; surface: EntityEnvelope | undefined }[] {
  return store.outline.all().filter((e) => e.type_id === 'buckyos.cell' && e.source_id === entityId && !e.deleted).map((block) => ({
    block, surface: store.outline.ancestors(block.entity_id).map((id) => store.outline.get(id)).find((e) => e?.kind === 'surface'),
  }))
}
