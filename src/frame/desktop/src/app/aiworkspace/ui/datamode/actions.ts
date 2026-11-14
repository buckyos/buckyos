/* The actions of the object tree (UI 框架 PRD §5.2, §10.3 last rule): one registry with the rest of the window
 * (ui/framework), `data.*`, asked with the object the menu opened on. A folder or the blank area: 新建 ▸, 导入 ▸, 粘贴表格数据;
 * a data object: 打开, 在新标签页打开, 放到画布…, 在画布上定位 ▸, 重命名, 移动到…, 权限…, 发布…, 复制路径, 查看引用与依赖, 删除 —
 * and 新建 ▸ / 导入 ▸ as its next sibling. The same list is the row's context menu (right click, a long press, the ⋯ of a
 * touch row), the tree's ＋ and a tab's menu. Shown only where it means something; disabled with its reason. */

import { CANVAS_CONTENT_ID, DATA_ID, SHOWS_ID, SURFACES_ID, type EntityEnvelope } from '../../api/types'
import { aiwsT } from '../../i18n'
import { defaultRendererFor } from '../blocks/ops'
import { actionRegistry, type ActionCandidate, type ActionContext, type ActionDef } from '../framework/actions'
import type { MenuItem } from '../shell/popover'
import type { NewDataKind } from '../sources/dataOps'
import { acceptsData, blocksShowing, isFolder, nameOf, pathText, SESSION_FOLDER } from './model'
import { placeOf, precheckDelete, type DeleteCheck, type Place } from './ops'
import { PERMISSIONS_TAB, type OpenOptions } from './tabs'

export type SideTab = 'properties' | 'permissions' | 'history'
/** The sections of the properties tab a menu can lead to. */
export type PropSection = 'basic' | 'source' | 'publish' | 'derived' | 'relations' | 'policy' | 'diagnostics'

export type DataDialog =
  | { kind: 'new'; type: NewDataKind; place: Place }
  | { kind: 'rename'; entityId: string }
  | { kind: 'move'; entityId: string }
  | { kind: 'place'; entityId: string }
  | { kind: 'delete'; entityId: string; check: DeleteCheck }

export type PickKind = 'zone' | 'device' | 'table' | 'folder'

/** What the data mode does for its actions. */
export interface DataHost {
  open(id: string, options?: OpenOptions): void
  /** Show a tab of the right sidebar for the current object (a section of the properties unfolded). */
  side(tab: SideTab, section?: PropSection): void
  dialog(request: DataDialog): void
  pick(kind: PickKind, place: Place): void
  pasteTable(place: Place): void
}

/** The object a menu opened on: an entity, the Workspace's permissions node, or the blank area (null: the data root). */
export interface DataActionContext extends ActionContext { target: string | null; host: DataHost }

const data = (ctx: ActionContext) => ctx as DataActionContext
const target = (ctx: ActionContext): EntityEnvelope | undefined => { const id = data(ctx).target; return id && id !== PERMISSIONS_TAB ? ctx.store.outline.get(id) : undefined }
const SYSTEM = new Set([DATA_ID, CANVAS_CONTENT_ID, SHOWS_ID, SURFACES_ID, SESSION_FOLDER])
/** The blank area, a folder, or the data root: what takes new data. */
const onPlace = (ctx: ActionContext) => { const t = data(ctx).target; return t === null || acceptsData(target(ctx)) }
/** An object of the tree (a folder included), not a system node, not the permissions node. */
const onObject = (ctx: ActionContext) => { const e = target(ctx); return Boolean(e && !SYSTEM.has(e.entity_id) && e.system !== 'surface_content') }
/** Data (not a folder, a canvas or a Block). */
const onData = (ctx: ActionContext) => { const e = target(ctx); return Boolean(e && !isFolder(e) && e.type_id !== 'buckyos.cell' && e.kind !== 'surface' && e.type_id !== 'buckyos.container') }
const openable = (ctx: ActionContext) => data(ctx).target === PERMISSIONS_TAB || Boolean(target(ctx))
const lacks = (e: EntityEnvelope | undefined, capability: string) => !e?.capabilities.includes(capability as never)

/** Where new things go for this menu, and whether the caller may put them there. */
function placeReason(ctx: ActionContext): string | null {
  const place = placeOf(ctx.store, data(ctx).target)
  return lacks(ctx.store.outline.get(place.parentId), 'structure') ? aiwsT('aiworkspace.datamode.noStructure') : null
}

const NEW_KINDS: { kind: NewDataKind; label: string }[] = [
  { kind: 'table', label: 'aiworkspace.data.tree.kindTable' }, { kind: 'richtext', label: 'aiworkspace.data.tree.kindRichtext' },
  { kind: 'record', label: 'aiworkspace.data.tree.kindRecord' }, { kind: 'dir', label: 'aiworkspace.data.type.dir' },
  { kind: 'link', label: 'aiworkspace.datamode.new.link' }, { kind: 'wish', label: 'aiworkspace.data.tree.kindWish' },
  { kind: 'folder', label: 'aiworkspace.data.tree.kindFolder' },
]

async function copyText(ctx: ActionContext, text: string) {
  try { await navigator.clipboard.writeText(text); ctx.store.notify('info', aiwsT('aiworkspace.datamode.pathCopied', { path: text })) } catch { ctx.store.notify('info', aiwsT('aiworkspace.datamode.pathNotCopied', { path: text })) }
}

export const DATA_ACTIONS: ActionDef[] = [
  { id: 'data.open', label: () => aiwsT('aiworkspace.datamode.open'), visible: openable,
    run: (ctx) => data(ctx).host.open(data(ctx).target ?? DATA_ID, { preview: true }) },
  { id: 'data.open-tab', label: () => aiwsT('aiworkspace.datamode.openTab'), visible: openable,
    run: (ctx) => data(ctx).host.open(data(ctx).target ?? DATA_ID) },
  { id: 'data.place', label: () => aiwsT('aiworkspace.datamode.place'), visible: (ctx) => onData(ctx) && Boolean(defaultRendererFor(target(ctx)?.type_id ?? '')),
    reason: (ctx) => (ctx.store.session.info().capabilities.includes('structure') ? null : aiwsT('aiworkspace.datamode.noStructure')),
    run: (ctx) => data(ctx).host.dialog({ kind: 'place', entityId: target(ctx)!.entity_id }) },
  { id: 'data.locate', label: () => aiwsT('aiworkspace.datamode.locate'), visible: (ctx) => Boolean(target(ctx)) && target(ctx)?.kind !== 'surface',
    reason: (ctx) => (locateCandidates(ctx).length === 0 ? aiwsT('aiworkspace.datamode.notOnCanvas') : null),
    candidates: locateCandidates },
  { id: 'data.rename', label: () => aiwsT('aiworkspace.data.rename'), visible: onObject,
    reason: (ctx) => (lacks(target(ctx), 'structure') ? aiwsT('aiworkspace.datamode.noStructure') : null),
    run: (ctx) => data(ctx).host.dialog({ kind: 'rename', entityId: target(ctx)!.entity_id }) },
  { id: 'data.move', label: () => aiwsT('aiworkspace.datamode.move'), visible: (ctx) => onObject(ctx) && target(ctx)?.type_id !== 'buckyos.cell' && target(ctx)?.kind !== 'surface',
    reason: (ctx) => (lacks(target(ctx), 'structure') ? aiwsT('aiworkspace.datamode.noStructure') : null),
    run: (ctx) => data(ctx).host.dialog({ kind: 'move', entityId: target(ctx)!.entity_id }) },
  { id: 'data.permissions', label: () => aiwsT('aiworkspace.datamode.permissionsOf'), visible: (ctx) => Boolean(target(ctx)) && !SYSTEM.has(target(ctx)!.entity_id),
    run: (ctx) => { data(ctx).host.open(target(ctx)!.entity_id, { preview: true }); data(ctx).host.side('permissions') } },
  { id: 'data.publish', label: () => aiwsT('aiworkspace.datamode.publish'), visible: (ctx) => onData(ctx) && PUBLISHABLE.has(target(ctx)?.type_id ?? '') && !target(ctx)?.source,
    reason: (ctx) => (ctx.store.session.info().capabilities.includes('manage') ? null : aiwsT('aiworkspace.datamode.needsManage')),
    run: (ctx) => { data(ctx).host.open(target(ctx)!.entity_id, { preview: true }); data(ctx).host.side('properties', 'publish') } },
  { id: 'data.copy-path', label: () => aiwsT('aiworkspace.datamode.copyPath'), visible: (ctx) => Boolean(target(ctx)),
    run: (ctx) => { void copyText(ctx, pathText(ctx.store, target(ctx)!.entity_id)) } },
  { id: 'data.relations', label: () => aiwsT('aiworkspace.datamode.relations'), visible: (ctx) => Boolean(target(ctx)) && !SYSTEM.has(target(ctx)!.entity_id),
    run: (ctx) => { data(ctx).host.open(target(ctx)!.entity_id, { preview: true }); data(ctx).host.side('properties', 'relations') } },
  { id: 'data.new', label: () => aiwsT('aiworkspace.datamode.new'), visible: (ctx) => onPlace(ctx) || onData(ctx),
    reason: placeReason,
    candidates: (ctx) => NEW_KINDS.map(({ kind, label }) => ({ id: `new-${kind}`, testId: `aiws-dm-new-${kind}`, label: `${aiwsT(label)}…`, run: () => data(ctx).host.dialog({ kind: 'new', type: kind, place: placeOf(ctx.store, data(ctx).target) }) })) },
  { id: 'data.import', label: () => aiwsT('aiworkspace.datamode.import'), visible: (ctx) => onPlace(ctx) || onData(ctx),
    reason: placeReason,
    candidates: (ctx) => {
      const place = placeOf(ctx.store, data(ctx).target)
      const pick = (kind: PickKind) => () => data(ctx).host.pick(kind, place)
      return [
        { id: 'import-zone', testId: 'aiws-pick-files', label: aiwsT('aiworkspace.datamode.importZone'), run: pick('zone') },
        { id: 'import-device', testId: 'aiws-dm-import-device', label: aiwsT('aiworkspace.datamode.importDevice'), run: pick('device') },
        { id: 'import-table', testId: 'aiws-datatree-import-table', label: aiwsT('aiworkspace.datamode.importTable'), run: pick('table') },
        { id: 'import-folder', testId: 'aiws-pick-folder', label: aiwsT('aiworkspace.datamode.importFolder'), hint: aiwsT('aiworkspace.datamode.folderLimit'), run: pick('folder') },
      ]
    } },
  { id: 'data.paste-table', label: () => aiwsT('aiworkspace.datamode.pasteTable'), visible: onPlace,
    reason: placeReason,
    run: (ctx) => data(ctx).host.pasteTable(placeOf(ctx.store, data(ctx).target)) },
  { id: 'data.delete', label: () => aiwsT('aiworkspace.datamode.delete'), visible: onObject,
    reason: (ctx) => (lacks(target(ctx), 'delete') ? aiwsT('aiworkspace.datamode.noDelete') : null),
    run: (ctx) => {
      const entity = target(ctx)
      if (!entity) return
      void precheckDelete(ctx.store, entity).then((check) => { if (check) data(ctx).host.dialog({ kind: 'delete', entityId: entity.entity_id, check }) })
    } },
]

const PUBLISHABLE = new Set(['buckyos.table-source', 'buckyos.richtext', 'buckyos.record', 'buckyos.asset-ref', 'buckyos.dir'])

function locateCandidates(ctx: ActionContext): ActionCandidate[] {
  const e = target(ctx)
  if (!e) return []
  const blocks = e.type_id === 'buckyos.cell' ? [{ block: e, surface: ctx.store.outline.ancestors(e.entity_id).map((id) => ctx.store.outline.get(id)).find((s) => s?.kind === 'surface') }] : blocksShowing(ctx.store, e.entity_id)
  return blocks.map(({ block, surface }) => ({
    id: `locate-${block.entity_id}`, testId: `aiws-dm-locate-${block.entity_id}`,
    label: surface ? aiwsT('aiworkspace.datamode.onCanvas', { canvas: nameOf(ctx.store, surface, surface.entity_id), block: block.title ?? block.view_type ?? block.entity_id }) : block.entity_id,
    run: () => ctx.openEntity(block.entity_id),
  }))
}

for (const def of DATA_ACTIONS) actionRegistry.register(def)

/** The menu's layout: groups of action ids, a separator between groups. */
const OBJECT_MENU: string[][] = [['data.open', 'data.open-tab'], ['data.place', 'data.locate'], ['data.rename', 'data.move', 'data.permissions', 'data.publish', 'data.copy-path', 'data.relations'], ['data.new', 'data.import'], ['data.delete']]
const PLACE_MENU: string[][] = [['data.new', 'data.import', 'data.paste-table'], ['data.open', 'data.open-tab'], ['data.rename', 'data.move', 'data.permissions', 'data.copy-path'], ['data.delete']]
/** The test id of an entry (the old data tree's ids where the function is the same). */
const TEST_ID: Record<string, string> = { 'data.delete': 'aiws-delete-entity' }

/** The context menu of the object `ctx.target` (or of the blank area), from the registry. */
export function dataMenu(ctx: DataActionContext): MenuItem[] {
  const layout = ctx.target === null || acceptsData(target(ctx)) ? PLACE_MENU : OBJECT_MENU
  const out: MenuItem[] = []
  for (const group of layout) {
    const items: MenuItem[] = []
    for (const id of group) {
      const def = actionRegistry.get(id)
      if (!def || (def.visible && !def.visible(ctx))) continue
      const reason = def.reason?.(ctx) ?? null
      const base = { id: id.replace(/^data\./, ''), testId: TEST_ID[id] ?? `aiws-dm-menu-${id.slice(5)}`, label: def.label(ctx), danger: id === 'data.delete' }
      if (def.candidates) {
        const candidates = def.candidates(ctx)
        items.push({ ...base, disabled: reason !== null || candidates.length === 0, reason, items: candidates.map((c) => ({ id: c.id, testId: c.testId, label: c.label, hint: c.hint, disabled: Boolean(c.reason), reason: c.reason ?? null, run: c.run })) })
      } else items.push({ ...base, disabled: reason !== null, reason, run: () => def.run?.(ctx) })
    }
    if (items.length === 0) continue
    if (out.length > 0) out.push({ id: `sep-${group[0]}`, separator: true })
    out.push(...items)
  }
  return out
}
