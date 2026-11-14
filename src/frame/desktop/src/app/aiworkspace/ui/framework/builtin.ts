/* The built-in actions of the workspace shell (UI 框架 PRD §3.3, §4.1.2, §4.2): the workspace, canvas, insert, edit,
 * view, collaboration, data-mode and help entries of the main menu, and the default items of the collaboration zone.
 * Each calls the logic it always called (the store, the shell, the open canvas's commands); none writes around a
 * Commit. Registered once, when the shell first loads this module. `phone` follows the phone's main menu (§6.1). */

import { unwrap } from '../../api/client'
import { describeError } from '../../api/session'
import type { Json } from '../../api/types'
import { aiwsT } from '../../i18n'
import { openWorkspacePage } from '../../launch'
import { pathsOf } from '../../presentation/model'
import { CANVAS_MODE_LABEL, CANVAS_MODES, CATALOG_GROUP_LABEL, type CatalogGroup } from '../blocks/registry'
import { entryId, extensionEntries, registryEntries, type CatalogEntry } from '../canvas/catalog'
import { surfaceWriteReason } from '../canvas/surfaceManage'
import { shareLink, SIDE_TAB_LABEL, ZOOM_PRESETS } from '../shell/shellContext'
import { actionRegistry, type ActionCandidate, type ActionContext, type ActionDef } from './actions'
import { mayCustomize, requestCustomize, setZonesRevealed, zonesRevealed } from './appMode'
import { listMacros, runMacro } from './macros'
import { requestPageSettings } from './pageSettings'

const notOnCanvas = () => aiwsT('aiworkspace.shell.notOnCanvas')
const caps = ({ store }: ActionContext) => store.session.info().capabilities
const live = ({ store }: ActionContext) => store.session.status().kind === 'live'
const guest = ({ shell }: ActionContext) => shell.identity.guest
const member = (ctx: ActionContext) => !guest(ctx)
const insertReason = ({ canvas }: ActionContext) => (canvas ? canvas.insertReason : notOnCanvas())
const noteReason = ({ canvas }: ActionContext) => (canvas ? canvas.noteReason : notOnCanvas())
const command = (pick: (canvas: NonNullable<ActionContext['canvas']>) => { reason: string | null; run: () => void }) => ({
  reason: (ctx: ActionContext) => (ctx.canvas ? pick(ctx.canvas).reason : notOnCanvas()),
  run: (ctx: ActionContext) => { if (ctx.canvas) pick(ctx.canvas).run() },
})
const viewReason = ({ canvas }: ActionContext) => (canvas ? (canvas.view ? null : aiwsT('aiworkspace.shell.menu.flowNoZoom')) : notOnCanvas())

function beginGuide({ store, shell }: ActionContext, pathId: string) {
  const saved = store.userState.get<Json>(`guide:${pathId}`) as { index?: number; done?: boolean } | undefined
  shell.setGuide({ pathId, index: saved && !saved.done ? saved.index ?? 0 : 0 })
}

function guideCandidates(ctx: ActionContext, named: boolean): ActionCandidate[] {
  return pathsOf(ctx.store.outline, 'guide').map((g) => {
    const name = g.title ?? g.name ?? g.entity_id
    return { id: `guide-${g.entity_id}`, testId: named ? `aiws-play-guide-${g.entity_id}` : `aiws-menu-guide-${g.entity_id}`, label: named ? aiwsT('aiworkspace.framework.play.guideNamed', { name }) : name, run: () => beginGuide(ctx, g.entity_id) }
  })
}

/** The runnable macros once per context (the tool item asks for its reason, tooltip and candidates). */
const macroLists = new WeakMap<ActionContext, ReturnType<typeof listMacros>>()

function macroCandidates(ctx: ActionContext, args?: Json): ActionCandidate[] {
  const bound = args && typeof args === 'object' && !Array.isArray(args) && typeof args.wish === 'string' ? args.wish : null
  let macros = macroLists.get(ctx)
  if (!macros) { macros = listMacros(ctx.store, ctx.shell.activeSurface?.entity_id ?? null); macroLists.set(ctx, macros) }
  return macros.filter((m) => !bound || m.wishId === bound).map((m) => ({
    id: `macro-${m.wishId}`, testId: `aiws-macro-${m.wishId}`, label: m.title, reason: m.busy ? aiwsT('aiworkspace.framework.macro.running') : null,
    run: () => runMacro(ctx.store, m, ctx.openEntity),
  }))
}

function insertGroup(group: Exclude<CatalogGroup, 'sample' | 'extension'>, order: number): ActionDef {
  const inGroup = () => registryEntries().filter((entry) => entry.group === group || (group === 'data' && entry.group === 'sample'))
  const reasonOf = (ctx: ActionContext, entry: CatalogEntry) => (entry.definition.type === 'note' ? noteReason(ctx) : insertReason(ctx))
  return {
    id: `surface.insert.${group}`, label: () => CATALOG_GROUP_LABEL[group],
    menu: { group: 'insert', order, id: `insert-group-${group}`, submenu: true },
    reason: (ctx) => (inGroup().every((entry) => reasonOf(ctx, entry) !== null) ? insertReason(ctx) : null),
    candidates: (ctx) => inGroup().map((entry) => ({
      id: `insert-${entryId(entry)}`, testId: `aiws-main-insert-${entryId(entry)}`, label: `${entry.title}${entry.catalog.needs === 'none' ? '' : '…'}`,
      reason: reasonOf(ctx, entry), run: () => ctx.canvas?.insert(entry),
    })),
  }
}

/** 播放's candidates (§4.2): the presentation paths (not on a phone), then the guides; a public presentation last. */
function playCandidates(ctx: ActionContext): ActionCandidate[] {
  const paths = ctx.shell.phone ? [] : pathsOf(ctx.store.outline, 'presentation').map((p): ActionCandidate => {
    const name = p.title ?? p.name ?? p.entity_id
    return { id: `path-${p.entity_id}`, testId: `aiws-play-path-${p.entity_id}`, label: aiwsT('aiworkspace.framework.play.pathNamed', { name }), run: () => ctx.shell.startShow({ pathId: p.entity_id }) }
  })
  const guides = guideCandidates(ctx, true)
  return [
    ...paths, ...(paths.length > 0 && guides.length > 0 ? [{ id: 'sep-guides', separator: true }] : []), ...guides,
    ...(paths.length > 0 ? [{ id: 'sep-public', separator: true, extra: true }, { id: 'public', testId: 'aiws-play-public', label: aiwsT('aiworkspace.framework.play.public'), extra: true, run: () => ctx.shell.startShow() }] : []),
  ]
}

/** Without a path, 播放 leads whoever may structure the canvas to path editing. */
const canEditPaths = (ctx: ActionContext) => caps(ctx).includes('structure') && !ctx.shell.phone && ctx.shell.topMode === 'canvas' && ctx.mode !== 'presentation_edit'

export const BUILTIN_ACTIONS: ActionDef[] = [
  // ---- a guest (a public link) owns no workspace: signing in is the way to more
  { id: 'ws.login', label: () => aiwsT('aiworkspace.shell.loginBuckyos'), phone: true, visible: guest,
    menu: { group: 'account', order: 0, id: 'login', hint: () => aiwsT('aiworkspace.shell.menu.guestHint') }, run: ({ shell }) => shell.login() },

  // ---- 工作区
  // a restricted session (Limited account, External DID) creates no workspace
  { id: 'ws.new', label: () => aiwsT('aiworkspace.framework.ws.new'), phone: true, visible: (ctx) => member(ctx) && !ctx.shell.identity.restricted,
    menu: { group: 'workspace', order: 10, id: 'new' }, run: ({ shell }) => shell.openDialog({ kind: 'new', tab: 'workspace' }) },
  { id: 'ws.open', label: () => aiwsT('aiworkspace.shell.menu.openWorkspace'), phone: true, visible: member,
    menu: { group: 'workspace', order: 20, id: 'open-workspace', hint: () => aiwsT('aiworkspace.shell.menu.openWorkspaceHint') }, run: ({ shell }) => shell.close() },
  { id: 'ws.home', label: () => aiwsT('aiworkspace.shell.menu.home'), visible: (ctx) => member(ctx) && ctx.shell.home !== null,
    menu: { group: 'workspace', order: 30, id: 'home' }, run: ({ shell }) => shell.home?.() },
  // the workspace's own origin, signed in as this Desktop user (T4-8)
  { id: 'ws.new-tab', label: () => aiwsT('aiworkspace.shell.menu.newTab'), visible: (ctx) => member(ctx) && ctx.shell.home === null,
    menu: { group: 'workspace', order: 30, id: 'new-tab' },
    run: ({ store, shell }) => { void openWorkspacePage(shareLink(store.session.info(), shell.topMode === 'canvas' ? shell.activeSurface : null), { newTab: true }) } },
  { id: 'ws.clone', label: () => aiwsT('aiworkspace.shell.menu.clone'), phone: true, visible: member,
    menu: { group: 'workspace', order: 40, id: 'clone', separator: true },
    reason: (ctx) => (!caps(ctx).includes('manage') ? aiwsT('aiworkspace.shell.menu.needsManage') : !live(ctx) ? aiwsT('aiworkspace.shell.needsOnline') : null),
    run: ({ store, shell }) => {
      shell.client.wsClone({ workspace_id: store.session.workspaceId }).then((result) => unwrap(result)).then(
        (cloned) => store.notify('info', aiwsT('aiworkspace.shell.menu.cloned', { id: cloned.workspace_id }), { label: aiwsT('aiworkspace.shell.menu.openClone'), run: () => shell.openWorkspace(cloned.workspace_id) }),
        (error: unknown) => store.notify('error', aiwsT('aiworkspace.shell.menu.cloneFailed', { error: describeError(error) })),
      )
    } },
  { id: 'ws.import', label: () => aiwsT('aiworkspace.shell.menu.import'), phone: true, visible: member,
    menu: { group: 'workspace', order: 50, id: 'import' }, run: ({ shell }) => shell.openDialog({ kind: 'import' }) },
  { id: 'ws.export', label: () => aiwsT('aiworkspace.framework.ws.export'), phone: true, visible: member,
    menu: { group: 'workspace', order: 60, id: 'export', hint: () => aiwsT('aiworkspace.shell.menu.exportPackageHint') },
    reason: (ctx) => (caps(ctx).includes('export') ? null : aiwsT('aiworkspace.shell.menu.noExportPermission')), run: ({ shell }) => shell.openDialog({ kind: 'export' }) },
  { id: 'ws.offline', label: () => aiwsT('aiworkspace.shell.menu.offline'), visible: member,
    menu: { group: 'workspace', order: 70, id: 'offline', submenu: true },
    candidates: (ctx) => {
      const { store, shell } = ctx
      const sessionMode = store.session.mode()
      return [
        { id: 'prepare-offline', label: aiwsT('aiworkspace.shell.prepareOffline'),
          reason: sessionMode.kind === 'direct' && sessionMode.reason === 'not_prepared' && live(ctx) && shell.offlineBusy === null ? null
            : sessionMode.kind === 'replica' ? aiwsT('aiworkspace.shell.menu.holdsReplica') : !live(ctx) ? aiwsT('aiworkspace.shell.needsOnline') : aiwsT('aiworkspace.shell.menu.cannotOffline'),
          run: () => shell.runOffline(aiwsT('aiworkspace.shell.preparingOffline'), () => shell.offline.prepare(store.session.workspaceId, false)) },
        { id: 'takeover', label: aiwsT('aiworkspace.shell.takeover'),
          reason: sessionMode.kind === 'direct' && sessionMode.reason === 'not_holder' && shell.offlineBusy === null ? null : aiwsT('aiworkspace.shell.menu.takeoverReason'),
          run: () => shell.runOffline(aiwsT('aiworkspace.shell.takingOver'), () => shell.offline.reopen(store.session.workspaceId)) },
        { id: 'export-local', label: aiwsT('aiworkspace.shell.exportLocal'), run: () => { void store.exportLocal() } },
      ]
    } },
  { id: 'ws.close', label: () => aiwsT('aiworkspace.shell.menu.close'), phone: true, visible: member,
    menu: { group: 'workspace', order: 90, id: 'close', testId: 'aiws-back', separator: true }, run: ({ shell }) => shell.close() },

  // ---- 画布
  // a phone makes flow pages only (§6.5, Q9)
  { id: 'surface.new', label: ({ shell }) => aiwsT(shell.phone ? 'aiworkspace.phone.newFlowPage' : 'aiworkspace.shell.menu.newCanvas'), phone: true,
    menu: { group: 'surface', order: 10, id: 'new-canvas' },
    reason: (ctx) => (caps(ctx).includes('structure') ? null : aiwsT('aiworkspace.shell.noStructurePermission')), run: ({ shell }) => shell.openDialog({ kind: 'new', tab: 'canvas' }) },
  { id: 'surface.mode', label: () => aiwsT('aiworkspace.shell.menu.canvasMode'),
    menu: { group: 'surface', order: 20, id: 'canvas-mode', submenu: true },
    reason: ({ shell }) => (shell.topMode !== 'canvas' ? notOnCanvas() : null),
    candidates: ({ store, mode }) => CANVAS_MODES.map((m) => ({ id: `mode-${m}`, testId: `aiws-mode-${m}`, label: CANVAS_MODE_LABEL[m], checked: mode === m, run: () => store.userState.set('canvas:mode', m) })) },
  { id: 'surface.page-settings', label: () => aiwsT('aiworkspace.framework.surface.pageSettings'), phone: true,
    menu: { group: 'surface', order: 30, id: 'page-settings' },
    // a phone views a free canvas, yet its page settings are an explicit way in, like "属性" (§6.4, Q12)
    reason: ({ store, shell, mode }) => (shell.topMode === 'canvas' && shell.activeSurface ? surfaceWriteReason(store, shell.activeSurface, !shell.phone && mode !== 'edit', null) : notOnCanvas()),
    run: () => requestPageSettings() },
  { id: 'surface.export-drawio', label: () => aiwsT('aiworkspace.framework.surface.exportDrawio'),
    menu: { group: 'surface', order: 40, id: 'export-drawio', testId: 'aiws-menu-export-drawio', separator: true, hint: () => aiwsT('aiworkspace.shell.menu.exportDrawioHint') },
    ...command((canvas) => canvas.exportDrawio) },
  // a Workspace template of chosen Surfaces into the library (扩展控件 P3, Q32): online, with `export`
  { id: 'surface.save-template', label: () => aiwsT('aiworkspace.shell.menu.saveTemplate'), visible: member,
    menu: { group: 'surface', order: 50, id: 'save-template', testId: 'aiws-menu-save-template', hint: () => aiwsT('aiworkspace.shell.menu.saveTemplateHint') },
    reason: (ctx) => (!caps(ctx).includes('export') ? aiwsT('aiworkspace.shell.menu.noExportPermission') : !live(ctx) || ctx.shell.phone || ctx.shell.identity.restricted ? aiwsT('aiworkspace.shell.needsOnline') : null),
    run: ({ shell }) => shell.openDialog({ kind: 'save-template' }) },

  // 手写 (§6.3, Q16): the phone's way to the pen, a row of its own on a free canvas; the desktop's is the tool selection zone
  { id: 'surface.ink', label: () => aiwsT('aiworkspace.phone.ink'), phone: true, explain: true, phoneMenu: { group: 'ink', order: 10, id: 'ink' },
    visible: ({ shell, canvas }) => shell.phone && Boolean(canvas?.ink), reason: ({ canvas }) => canvas?.ink?.reason ?? null, run: ({ canvas }) => { if (canvas?.ink && canvas.ink.reason === null) canvas.ink.run() } },

  // ---- 插入
  insertGroup('text', 10), insertGroup('data', 20), insertGroup('layout', 30), insertGroup('ai', 40),
  { id: 'surface.insert.extension', label: () => CATALOG_GROUP_LABEL.extension,
    menu: { group: 'insert', order: 50, id: 'insert-group-extension', submenu: true },
    candidates: (ctx) => {
      const extensions = extensionEntries(ctx.store)
      if (extensions.length === 0) return [{ id: 'no-definitions', label: aiwsT('aiworkspace.shell.menu.noDefinitions'), reason: aiwsT('aiworkspace.shell.menu.noDefinitions') }]
      return extensions.map((entry) => ({ id: `insert-${entry.key}`, label: `${entry.title}…`, reason: insertReason(ctx), run: () => ctx.canvas?.openCatalog('extension', entry.key) }))
    } },
  { id: 'surface.insert.catalog', label: () => aiwsT('aiworkspace.shell.menu.insertCatalog'),
    menu: { group: 'insert', order: 60, id: 'insert-catalog', separator: true }, reason: insertReason, run: ({ canvas }) => canvas?.openCatalog('all') },
  { id: 'surface.insert.existing', label: () => aiwsT('aiworkspace.shell.menu.addData'),
    menu: { group: 'insert', order: 70, id: 'add-data', hint: () => aiwsT('aiworkspace.shell.menu.addDataHint') }, reason: insertReason, run: ({ canvas }) => canvas?.openCatalog('existing') },
  // files (draw.io 导入 TODO §3, PDF 导入方案 §5, Office 导入 TODO §6, 表格数据导入 TODO §5.1): dropping or pasting does the same
  { id: 'surface.import', label: () => aiwsT('aiworkspace.framework.insert.importFile'), phone: true,
    menu: { group: 'insert', order: 80, id: 'import-file', submenu: true }, phoneMenu: { group: 'import-doc', order: 10 },
    candidates: ({ canvas }) => {
      const item = (id: string, label: string, hint: string, pick: (c: NonNullable<ActionContext['canvas']>) => { reason: string | null; run: () => void }): ActionCandidate => ({
        id, testId: `aiws-menu-${id}`, label, hint, reason: canvas ? pick(canvas).reason : notOnCanvas(), run: () => { if (canvas) pick(canvas).run() },
      })
      return [
        item('import-drawio', aiwsT('aiworkspace.shell.menu.importDrawio'), '.drawio .png .svg', (c) => c.importDrawio),
        item('import-pdf', aiwsT('aiworkspace.shell.menu.importPdf'), '.pdf', (c) => c.importPdf),
        item('import-office', aiwsT('aiworkspace.office.menu'), '.docx .pptx', (c) => c.importOffice),
        item('import-table', aiwsT('aiworkspace.shell.menu.importTable'), '.csv .xlsx .sqlite', (c) => c.importTable),
      ]
    } },
  // extension controls (扩展控件 P0): managing them is an entry of its own in the group
  { id: 'ws.extensions', label: () => aiwsT('aiworkspace.shell.menu.extensions'),
    menu: { group: 'insert', order: 90, id: 'extensions', testId: 'aiws-menu-extensions', separator: true, hint: () => aiwsT('aiworkspace.shell.menu.extensionsHint') },
    run: ({ shell }) => shell.openDialog({ kind: 'extensions' }) },

  // ---- 编辑
  { id: 'edit.undo', label: () => aiwsT('aiworkspace.shell.menu.undo'), phone: true, menu: { group: 'edit', order: 10, id: 'undo', hint: () => 'Ctrl+Z' },
    reason: ({ store }) => { const undo = store.undo.snapshot(); return undo.undo.length === 0 || undo.busy ? aiwsT('aiworkspace.shell.menu.nothingToUndo') : null },
    run: ({ store }) => { void store.undo.undo() } },
  { id: 'edit.redo', label: () => aiwsT('aiworkspace.shell.menu.redo'), phone: true, menu: { group: 'edit', order: 20, id: 'redo', hint: () => 'Ctrl+Shift+Z' },
    reason: ({ store }) => { const undo = store.undo.snapshot(); return undo.redo.length === 0 || undo.busy ? aiwsT('aiworkspace.shell.menu.nothingToRedo') : null },
    run: ({ store }) => { void store.undo.redo() } },
  { id: 'edit.copy', label: () => aiwsT('aiworkspace.shell.menu.copy'), menu: { group: 'edit', order: 30, id: 'copy', hint: () => 'Ctrl+C', separator: true }, ...command((c) => c.copy) },
  { id: 'edit.cut', label: () => aiwsT('aiworkspace.shell.menu.cut'), menu: { group: 'edit', order: 40, id: 'cut', hint: () => 'Ctrl+X' }, ...command((c) => c.cut) },
  { id: 'edit.paste', label: () => aiwsT('aiworkspace.shell.menu.paste'), menu: { group: 'edit', order: 50, id: 'paste', hint: () => 'Ctrl+V' }, ...command((c) => c.paste) },
  { id: 'edit.delete', label: () => aiwsT('aiworkspace.shell.menu.deleteSelection'), menu: { group: 'edit', order: 60, id: 'delete-selection', hint: () => 'Delete' }, ...command((c) => c.remove) },

  // ---- 视图
  { id: 'view.zoom', label: () => aiwsT('aiworkspace.shell.menu.zoom'), phone: true, menu: { group: 'view', order: 10, id: 'zoom', submenu: true }, reason: viewReason,
    candidates: ({ canvas }) => {
      const view = canvas?.view
      return view ? ZOOM_PRESETS.map((value) => ({ id: `zoom-${value}`, label: `${value}%`, checked: Math.round(view.camera.zoom * 100) === value, run: () => view.camera.zoomTo(value / 100) })) : []
    } },
  { id: 'view.fit-all', label: () => aiwsT('aiworkspace.shell.fitAll'), phone: true, menu: { group: 'view', order: 20, id: 'fit-all', testId: 'aiws-menu-fit-all' }, phoneMenu: { order: 5 }, reason: viewReason,
    run: ({ canvas }) => canvas?.view?.fitAll() },
  { id: 'view.fit-selection', label: () => aiwsT('aiworkspace.shell.fitSelection'), menu: { group: 'view', order: 30, id: 'fit-selection', testId: 'aiws-menu-fit-selection' },
    reason: (ctx) => viewReason(ctx) ?? (ctx.canvas?.view?.hasSelection ? null : aiwsT('aiworkspace.shell.noSelection')), run: ({ canvas }) => canvas?.view?.fitSelection() },
  { id: 'view.grid', label: () => aiwsT('aiworkspace.shell.menu.grid'), menu: { group: 'view', order: 40, id: 'pref-grid', separator: true },
    checked: ({ shell }) => shell.prefs.grid, run: ({ shell }) => shell.setPref('grid', !shell.prefs.grid) },
  // the person's own switches (across Workspaces, §4.6); a zone a shared layer locked is not theirs to change
  { id: 'view.zones', label: () => aiwsT('aiworkspace.framework.view.zones'), menu: { group: 'view', order: 50, id: 'zones', submenu: true },
    candidates: ({ shell, layout }) => {
      const zone = (id: string, key: 'objectToolbar' | 'viewToolbar', name: 'tools' | 'collab', label: string): ActionCandidate => {
        const shown = layout ? !layout[name].hidden : shell.prefs[key]
        return { id, label, checked: shown, reason: layout?.[name].locked ? aiwsT('aiworkspace.customize.lockedZone') : null, explain: true, run: () => shell.setPref(key, !shown) }
      }
      return [
        zone('pref-object-toolbar', 'objectToolbar', 'tools', aiwsT('aiworkspace.framework.zone.tools')),
        zone('pref-view-toolbar', 'viewToolbar', 'collab', aiwsT('aiworkspace.framework.zone.collab')),
      ]
    } },
  // 自定义工具栏… (§4.6): the shared layers of this canvas or this Workspace, for whoever may change one; not on a phone (§6.1)
  { id: 'view.customize', label: () => aiwsT('aiworkspace.customize.menu'), menu: { group: 'view', order: 70, id: 'customize-toolbar', separator: true },
    visible: (ctx) => !ctx.shell.phone && member(ctx),
    reason: ({ store, shell }) => (mayCustomize(store, shell.activeSurface?.entity_id ?? null) ? null : aiwsT('aiworkspace.customize.needRights')),
    run: () => requestCustomize() },
  // App mode's zones shown for a while: hide them again
  { id: 'view.app-mode', label: () => aiwsT('aiworkspace.customize.backToApp'), menu: { group: 'view', order: 75, id: 'app-mode-back' },
    visible: ({ store }) => zonesRevealed(store), run: ({ store }) => setZonesRevealed(store, false) },
  { id: 'view.panels', label: () => aiwsT('aiworkspace.framework.view.panels'), menu: { group: 'view', order: 60, id: 'panels', submenu: true },
    reason: ({ canvas }) => (canvas ? null : notOnCanvas()),
    candidates: ({ canvas, shell }) => (canvas?.canvasTabs ?? []).map((tab) => ({ id: `side-${tab}`, label: SIDE_TAB_LABEL[tab], checked: shell.side === tab, run: () => shell.setSide(shell.side === tab ? null : tab) })) },
  { id: 'view.reset-layout', label: () => aiwsT('aiworkspace.shell.menu.resetLayout'), menu: { group: 'view', order: 90, id: 'reset-layout', separator: true, hint: () => aiwsT('aiworkspace.shell.menu.resetLayoutHint') },
    run: ({ shell }) => shell.resetLayout() },

  // ---- 协作 (menu entries; the collaboration zone's items follow)
  { id: 'collab.share', label: () => aiwsT('aiworkspace.shell.share.label'), icon: 'share', phone: true,
    menu: { group: 'collab', order: 10, id: 'share' }, zone: { zone: 'collab', order: 60, control: 'share', testId: 'aiws-share' },
    run: ({ shell }) => shell.openDialog({ kind: 'share' }) },
  { id: 'collab.members', label: () => aiwsT('aiworkspace.framework.collab.members'), icon: 'users', phone: true,
    menu: { group: 'collab', order: 20, id: 'members' }, run: ({ shell }) => shell.openPermissions() },
  // 评论 (Q14): the comment function (`comment` capability); a free canvas also takes notes from the collaboration zone
  { id: 'collab.comment', label: () => aiwsT('aiworkspace.shell.view.annotate'), icon: 'comment', phone: true,
    menu: { group: 'collab', order: 30, id: 'annotate', separator: true }, zone: { zone: 'collab', order: 30, testId: 'aiws-annotate-start' },
    visible: ({ canvas }) => Boolean(canvas?.comment),
    reason: ({ canvas }) => {
      const a = canvas?.comment
      return a && a.reason !== null && (!a.note || a.note.reason !== null) ? a.reason : null
    },
    tip: ({ canvas }) => {
      const a = canvas?.comment
      return a?.active ? aiwsT('aiworkspace.shell.view.annotatePick') : a?.note ? aiwsT('aiworkspace.shell.view.annotateOrNote') : a?.reason ?? aiwsT('aiworkspace.shell.view.annotateHint')
    },
    pressed: ({ canvas }) => Boolean(canvas?.comment?.active),
    candidates: ({ canvas }) => {
      const a = canvas?.comment
      if (!a?.note || a.active) return []
      return [
        { id: 'annotate-object', testId: 'aiws-annotate-object', label: aiwsT('aiworkspace.shell.view.annotateObject'), reason: a.reason, run: a.run },
        { id: 'annotate-note', testId: 'aiws-annotate-note', label: aiwsT('aiworkspace.shell.view.addNote'), reason: a.note.reason, explain: true, run: a.note.run },
      ]
    },
    run: ({ canvas }) => { if (canvas?.comment && canvas.comment.reason === null) canvas.comment.run() } },
  { id: 'collab.show', label: () => aiwsT('aiworkspace.shell.startShowMore'), icon: 'presentation', phone: true, explain: true,
    menu: { group: 'collab', order: 40, id: 'start-presentation', testId: 'aiws-top-play', separator: true },
    reason: ({ store }) => (pathsOf(store.outline, 'presentation').length === 0 ? aiwsT('aiworkspace.shell.menu.noPresentation') : null), run: ({ shell }) => shell.startShow() },
  { id: 'collab.guide', label: () => aiwsT('aiworkspace.shell.guide'), icon: 'guide', phone: true,
    menu: { group: 'collab', order: 50, id: 'guides', submenu: true },
    reason: ({ store }) => (pathsOf(store.outline, 'guide').length === 0 ? aiwsT('aiworkspace.shell.menu.noGuide') : null), candidates: (ctx) => guideCandidates(ctx, false) },
  // 宏 (§4.2, Q10): the last one used first, then the current canvas's, then the rest; none, no entry
  { id: 'collab.macro', label: () => aiwsT('aiworkspace.framework.macro.label'), icon: 'macro', phone: true,
    menu: { group: 'collab', order: 60, id: 'macros', submenu: true }, zone: { zone: 'collab', order: 20, testId: 'aiws-collab-macro' },
    reason: (ctx) => (macroCandidates(ctx).length === 0 ? aiwsT('aiworkspace.framework.macro.none') : null),
    tip: (ctx) => { const only = macroCandidates(ctx); return only.length === 1 ? aiwsT('aiworkspace.framework.macro.one', { name: only[0].label ?? '' }) : aiwsT('aiworkspace.framework.macro.label') },
    candidates: macroCandidates },
  // 播放 (§4.2): the presentation paths and the guides; one starts at once, several drop down; none: path editing for
  // whoever may structure the canvas, nothing for anyone else. A phone plays guides only.
  { id: 'collab.play', label: () => aiwsT('aiworkspace.framework.play.label'), icon: 'play', zone: { zone: 'collab', order: 10, testId: 'aiws-collab-play' },
    visible: (ctx) => playCandidates(ctx).some((c) => !c.separator && !c.extra) || canEditPaths(ctx),
    tip: (ctx) => (playCandidates(ctx).length === 0 ? aiwsT('aiworkspace.framework.play.editPaths') : aiwsT('aiworkspace.framework.play.label')),
    pressed: ({ shell, mode }) => shell.guide !== null || mode === 'presentation_edit',
    candidates: playCandidates,
    run: (ctx) => { if (canEditPaths(ctx)) ctx.store.userState.set('canvas:mode', 'presentation_edit') } },
  { id: 'collab.zoom', label: () => aiwsT('aiworkspace.shell.view.zoomMenu'), zone: { zone: 'collab', order: 40, control: 'zoom', overflow: 'narrow' },
    visible: ({ canvas }) => Boolean(canvas?.view),
    candidates: ({ canvas }) => {
      const view = canvas?.view
      if (!view) return []
      return [
        ...ZOOM_PRESETS.map((value) => ({ id: `zoom-${value}`, label: `${value}%`, checked: Math.round(view.camera.zoom * 100) === value, run: () => view.camera.zoomTo(value / 100) })),
        { id: 'sep-fit', separator: true },
        { id: 'fit-all', testId: 'aiws-more-fit-all', label: aiwsT('aiworkspace.shell.fitAll'), run: view.fitAll },
        { id: 'fit-selection', label: aiwsT('aiworkspace.shell.fitSelection'), reason: view.hasSelection ? null : aiwsT('aiworkspace.shell.noSelection'), run: view.fitSelection },
      ]
    } },
  { id: 'collab.avatars', label: () => aiwsT('aiworkspace.framework.members.panel'), icon: 'users', zone: { zone: 'collab', order: 50, control: 'avatars' } },

  // ---- 数据模式 / 返回画布: a row of its own
  // a phone has the data mode too (§5.6): its main menu is the way in
  { id: 'view.data-mode', label: ({ shell }) => (shell.topMode === 'sources' ? aiwsT('aiworkspace.shell.toolbar.backToCanvas') : aiwsT('aiworkspace.framework.dataMode')), icon: 'data', phone: true,
    menu: { group: 'mode', order: 10, id: 'data-mode', separator: true },
    checked: ({ shell }) => shell.topMode === 'sources', run: ({ shell }) => shell.setTopMode(shell.topMode === 'sources' ? 'canvas' : 'sources') },

  // ---- 帮助
  { id: 'help.shortcuts', label: () => aiwsT('aiworkspace.shell.help.title'), menu: { group: 'help', order: 10, id: 'shortcuts' }, run: ({ shell }) => shell.openDialog({ kind: 'help' }) },
  { id: 'help.dev', label: () => aiwsT('aiworkspace.shell.devTools'), visible: ({ shell }) => shell.devTools, menu: { group: 'help', order: 20, id: 'dev', submenu: true },
    candidates: ({ shell }) => [{ id: 'mock', testId: 'aiws-side-mock', label: aiwsT('aiworkspace.shell.menu.mock'), run: () => shell.openDialog({ kind: 'mock' }) }] },
]

for (const def of BUILTIN_ACTIONS) actionRegistry.register(def)
