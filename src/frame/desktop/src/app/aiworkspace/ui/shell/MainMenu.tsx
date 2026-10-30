/* The main menu (UI improvement §6.1): the complete index of functions, grouped by task. Every item
 * calls the same action as its button, context menu entry or shortcut; an item that cannot run now is
 * disabled and says why. Deleting objects, Surfaces and the workspace keep distinct wording; the last
 * stays in the workspace list. */

import { unwrap } from '../../api/client'
import { describeError } from '../../api/session'
import type { Json } from '../../api/types'
import { aiwsT, aiwsText } from '../../i18n'
import { pathsOf } from '../../presentation/model'
import { openWorkspacePage } from '../../launch'
import { shareLink } from './shellContext'
import type { WorkspaceStore } from '../../state/store'
import { CANVAS_MODE_LABEL, CANVAS_MODES, CATALOG_GROUP_LABEL, type CanvasMode, type CatalogGroup } from '../blocks/registry'
import { entryId, extensionEntries, registryEntries, type CatalogEntry } from '../canvas/catalog'
import type { CatalogTab } from '../canvas/InsertCatalog'
import type { Camera } from '../canvas/render/camera'
import type { MenuItem } from './popover'
import type { ShellApi, SideTab } from './shellContext'
import { SIDE_TAB_LABEL, ZOOM_PRESETS } from './shellContext'

export interface Command { reason: string | null; run: () => void }

/** What the open canvas offers the menus (absent in the data-source view and without a Surface). */
export interface CanvasCommands {
  isFree: boolean
  /** Why inserting is not possible now, or null. */
  insertReason: string | null
  /** The same for notes, which need only `comment` (公开链接便签). */
  noteReason: string | null
  insert: (entry: CatalogEntry) => void
  openCatalog: (tab: CatalogTab, key?: string) => void
  copy: Command
  cut: Command
  paste: Command
  remove: Command
  /** draw.io files (draw.io 导入 TODO §3): import into this canvas or new ones; export this canvas. */
  importDrawio: Command
  /** PDF files (PDF 导入方案 §5): pages on a new canvas with a presentation path. */
  importPdf: Command
  /** Table data (表格数据导入 TODO §5.1): CSV / xlsx / SQLite as a table on this canvas. */
  importTable: Command
  exportDrawio: Command
  /** Free Surfaces only. */
  view: { camera: Camera; fitAll: () => void; fitSelection: () => void; hasSelection: boolean } | null
  canvasTabs: SideTab[]
}

/** Built when the menu opens, in the language of that moment. */
export function buildMainMenu({ store, shell, canvas, mode }: { store: WorkspaceStore; shell: ShellApi; canvas: CanvasCommands | null; mode: CanvasMode }): MenuItem[] {
  const { t } = aiwsText()
  const notOnCanvas = t('aiworkspace.shell.notOnCanvas')
  const info = store.session.info()
  const caps = info.capabilities
  const live = store.session.status().kind === 'live'
  const undo = store.undo.snapshot()
  const sessionMode = store.session.mode()
  const insertReason = canvas ? canvas.insertReason : notOnCanvas
  const noteReason = canvas ? canvas.noteReason : notOnCanvas
  const reasonOf = (entry: CatalogEntry) => (entry.definition.type === 'note' ? noteReason : insertReason)
  const groups: CatalogGroup[] = ['text', 'data', 'layout', 'ai']
  const entries = registryEntries()
  const extensions = extensionEntries(store)
  const insertItems: MenuItem[] = groups.map((group) => {
    const inGroup = entries.filter((entry) => entry.group === group || (group === 'data' && entry.group === 'sample'))
    return {
      id: `insert-group-${group}`, label: CATALOG_GROUP_LABEL[group], disabled: inGroup.length === 0 || inGroup.every((entry) => reasonOf(entry) !== null), reason: insertReason,
      items: inGroup.map((entry) => ({ id: `insert-${entryId(entry)}`, testId: `aiws-main-insert-${entryId(entry)}`, label: `${entry.title}${entry.catalog.needs === 'none' ? '' : '…'}`,
        disabled: reasonOf(entry) !== null, reason: reasonOf(entry), run: () => canvas?.insert(entry) })),
    }
  })
  // extension controls (扩展控件 P0): managing them sits under their own group, not at the top level (the menu must fit a small window)
  insertItems.push({
    id: 'insert-group-extension', label: CATALOG_GROUP_LABEL.extension, items: [
      ...(extensions.length === 0
        ? [{ id: 'no-definitions', label: t('aiworkspace.shell.menu.noDefinitions'), disabled: true }]
        : extensions.map((entry) => ({ id: `insert-${entry.key}`, label: `${entry.title}…`, disabled: insertReason !== null, reason: insertReason, run: () => canvas?.openCatalog('extension', entry.key) }))),
      { id: 'sep-extensions', separator: true },
      { id: 'extensions', testId: 'aiws-menu-extensions', label: t('aiworkspace.shell.menu.extensions'), hint: t('aiworkspace.shell.menu.extensionsHint'), run: () => shell.openDialog({ kind: 'extensions' }) },
    ],
  })
  // draw.io files (draw.io 导入 TODO §3): the file picker; dropping or pasting a file does the same
  insertItems.push({ id: 'sep-import', separator: true }, {
    id: 'import-drawio', testId: 'aiws-menu-import-drawio', label: t('aiworkspace.shell.menu.importDrawio'), hint: '.drawio .png .svg', disabled: !canvas || canvas.importDrawio.reason !== null, reason: canvas?.importDrawio.reason ?? notOnCanvas, run: () => canvas?.importDrawio.run(),
  }, {
    id: 'import-pdf', testId: 'aiws-menu-import-pdf', label: t('aiworkspace.shell.menu.importPdf'), hint: '.pdf', disabled: !canvas || canvas.importPdf.reason !== null, reason: canvas?.importPdf.reason ?? notOnCanvas, run: () => canvas?.importPdf.run(),
  }, {
    id: 'import-table', testId: 'aiws-menu-import-table', label: t('aiworkspace.shell.menu.importTable'), hint: '.csv .xlsx .sqlite', disabled: !canvas || canvas.importTable.reason !== null, reason: canvas?.importTable.reason ?? notOnCanvas, run: () => canvas?.importTable.run(),
  })
  const side = (tab: SideTab): MenuItem => {
    const reason = canvas || tab === 'collab' || tab === 'edits' ? null : notOnCanvas
    return { id: `side-${tab}`, label: SIDE_TAB_LABEL[tab], checked: shell.side === tab, disabled: reason !== null, reason, run: () => shell.setSide(shell.side === tab ? null : tab) }
  }
  const view = canvas?.view ?? null
  const viewReason = canvas ? (view ? null : t('aiworkspace.shell.menu.flowNoZoom')) : notOnCanvas
  const notCanvas = shell.topMode !== 'canvas' ? notOnCanvas : null
  const presentations = pathsOf(store.outline, 'presentation')
  const guides = pathsOf(store.outline, 'guide')
  const guest = shell.identity.guest
  return [
    // a guest (a public link) owns no workspace: nothing to create or list, signing in is the way to more
    ...(guest
      ? [{ id: 'login', testId: 'aiws-menu-login', label: t('aiworkspace.shell.loginBuckyos'), hint: t('aiworkspace.shell.menu.guestHint'), run: shell.login } satisfies MenuItem]
      : [
        { id: 'new-canvas', label: t('aiworkspace.shell.menu.newCanvas'), disabled: !caps.includes('structure'), reason: t('aiworkspace.shell.noStructurePermission'), run: () => shell.openDialog({ kind: 'new', tab: 'canvas' }) },
        // a restricted session (Limited account, External DID) creates no workspace
        ...(shell.identity.restricted ? [] : [{ id: 'new', label: t('aiworkspace.shell.menu.new'), hint: t('aiworkspace.shell.menu.newHint'), run: () => shell.openDialog({ kind: 'new', tab: caps.includes('structure') ? 'canvas' : 'workspace' }) }]),
        { id: 'open-workspace', label: t('aiworkspace.shell.menu.openWorkspace'), hint: t('aiworkspace.shell.menu.openWorkspaceHint'), run: shell.close },
      ] satisfies MenuItem[]),
    shell.home
      ? { id: 'home', label: t('aiworkspace.shell.menu.home'), run: shell.home }
      // the workspace's own origin, signed in as this Desktop user (T4-8)
      : { id: 'new-tab', label: t('aiworkspace.shell.menu.newTab'), run: () => { void openWorkspacePage(shareLink(store.session.info(), shell.topMode === 'canvas' ? shell.activeSurface : null), { newTab: true }) } },
    { id: 'sep-1', separator: true },
    { id: 'insert', label: t('aiworkspace.shell.menu.insert'), disabled: insertReason !== null && noteReason !== null, reason: insertReason, explain: true, items: insertItems },
    { id: 'insert-catalog', label: t('aiworkspace.shell.menu.insertCatalog'), disabled: insertReason !== null, reason: insertReason, run: () => canvas?.openCatalog('all') },
    { id: 'add-data', label: t('aiworkspace.shell.menu.addData'), hint: t('aiworkspace.shell.menu.addDataHint'), disabled: insertReason !== null, reason: insertReason, run: () => canvas?.openCatalog('existing') },
    {
      id: 'edit', label: t('aiworkspace.common.edit'), items: [
        { id: 'undo', label: t('aiworkspace.shell.menu.undo'), hint: 'Ctrl+Z', disabled: undo.undo.length === 0 || undo.busy, reason: t('aiworkspace.shell.menu.nothingToUndo'), run: () => { void store.undo.undo() } },
        { id: 'redo', label: t('aiworkspace.shell.menu.redo'), hint: 'Ctrl+Shift+Z', disabled: undo.redo.length === 0 || undo.busy, reason: t('aiworkspace.shell.menu.nothingToRedo'), run: () => { void store.undo.redo() } },
        { id: 'sep-e', separator: true },
        { id: 'copy', label: t('aiworkspace.shell.menu.copy'), hint: 'Ctrl+C', disabled: !canvas || canvas.copy.reason !== null, reason: canvas?.copy.reason ?? notOnCanvas, run: () => canvas?.copy.run() },
        { id: 'cut', label: t('aiworkspace.shell.menu.cut'), hint: 'Ctrl+X', disabled: !canvas || canvas.cut.reason !== null, reason: canvas?.cut.reason ?? notOnCanvas, run: () => canvas?.cut.run() },
        { id: 'paste', label: t('aiworkspace.shell.menu.paste'), hint: 'Ctrl+V', disabled: !canvas || canvas.paste.reason !== null, reason: canvas?.paste.reason ?? notOnCanvas, run: () => canvas?.paste.run() },
        { id: 'delete-selection', label: t('aiworkspace.shell.menu.deleteSelection'), hint: 'Delete', disabled: !canvas || canvas.remove.reason !== null, reason: canvas?.remove.reason ?? notOnCanvas, run: () => canvas?.remove.run() },
      ],
    },
    { id: 'sep-3', separator: true },
    {
      id: 'view', label: t('aiworkspace.shell.menu.view'), items: [
        { id: 'pref-object-toolbar', label: t('aiworkspace.shell.menu.objectToolbar'), checked: shell.prefs.objectToolbar, run: () => shell.setPref('objectToolbar', !shell.prefs.objectToolbar) },
        { id: 'pref-view-toolbar', label: t('aiworkspace.shell.menu.viewToolbar'), checked: shell.prefs.viewToolbar, run: () => shell.setPref('viewToolbar', !shell.prefs.viewToolbar) },
        { id: 'pref-grid', label: t('aiworkspace.shell.menu.grid'), checked: shell.prefs.grid, run: () => shell.setPref('grid', !shell.prefs.grid) },
        { id: 'sep-v', separator: true },
        side('inspector'), side('relations'), side('annotations'), side('collab'), side('edits'),
        { id: 'sep-v2', separator: true },
        { id: 'reset-layout', label: t('aiworkspace.shell.menu.resetLayout'), hint: t('aiworkspace.shell.menu.resetLayoutHint'), run: shell.resetLayout },
      ],
    },
    {
      id: 'zoom', label: t('aiworkspace.shell.menu.zoom'), disabled: viewReason !== null, reason: viewReason, items: view ? [
        ...ZOOM_PRESETS.map((value) => ({ id: `zoom-${value}`, label: `${value}%`, checked: Math.round(view.camera.zoom * 100) === value, run: () => view.camera.zoomTo(value / 100) })),
        { id: 'sep-z', separator: true },
        { id: 'fit-all', testId: 'aiws-menu-fit-all', label: t('aiworkspace.shell.fitAll'), run: view.fitAll },
        { id: 'fit-selection', testId: 'aiws-menu-fit-selection', label: t('aiworkspace.shell.fitSelection'), disabled: !view.hasSelection, reason: t('aiworkspace.shell.noSelection'), run: view.fitSelection },
      ] : [],
    },
    {
      id: 'canvas-mode', label: t('aiworkspace.shell.menu.canvasMode'), disabled: notCanvas !== null, reason: notCanvas, items: CANVAS_MODES.map((m) => ({
        id: `mode-${m}`, testId: `aiws-mode-${m}`, label: CANVAS_MODE_LABEL[m], checked: mode === m, run: () => store.userState.set('canvas:mode', m),
      })),
    },
    { id: 'start-presentation', testId: 'aiws-top-play', label: t('aiworkspace.shell.startShowMore'), disabled: presentations.length === 0, reason: t('aiworkspace.shell.menu.noPresentation'), explain: true, run: () => shell.startShow() },
    {
      id: 'guides', label: t('aiworkspace.shell.guide'), disabled: guides.length === 0, reason: t('aiworkspace.shell.menu.noGuide'),
      items: guides.map((g) => ({ id: `guide-${g.entity_id}`, testId: `aiws-menu-guide-${g.entity_id}`, label: g.title ?? g.name ?? g.entity_id, run: () => {
        const saved = store.userState.get<Json>(`guide:${g.entity_id}`) as { index?: number; done?: boolean } | undefined
        shell.setGuide({ pathId: g.entity_id, index: saved && !saved.done ? saved.index ?? 0 : 0 })
      } })),
    },
    { id: 'sep-4', separator: true },
    { id: 'import', label: t('aiworkspace.shell.menu.import'), run: () => shell.openDialog({ kind: 'import' }) },
    // one entry for both exports: the menu must not grow past a small window (the last items would fall outside it)
    {
      id: 'export-menu', label: t('aiworkspace.common.export'), items: [
        { id: 'export', label: t('aiworkspace.shell.menu.exportPackage'), hint: t('aiworkspace.shell.menu.exportPackageHint'), disabled: !caps.includes('export'), reason: t('aiworkspace.shell.menu.noExportPermission'), run: () => shell.openDialog({ kind: 'export' }) },
        { id: 'export-drawio', testId: 'aiws-menu-export-drawio', label: t('aiworkspace.shell.menu.exportDrawio'), hint: t('aiworkspace.shell.menu.exportDrawioHint'), disabled: !canvas || canvas.exportDrawio.reason !== null, reason: canvas?.exportDrawio.reason ?? notOnCanvas, run: () => canvas?.exportDrawio.run() },
      ],
    },
    {
      id: 'fork', label: t('aiworkspace.shell.menu.fork'), disabled: !caps.includes('manage') || !live, reason: !caps.includes('manage') ? t('aiworkspace.shell.menu.needsManage') : t('aiworkspace.shell.needsOnline'),
      run: () => {
        shell.client.wsFork({ workspace_id: store.session.workspaceId }).then((result) => unwrap(result)).then(
          (forked) => store.notify('info', aiwsT('aiworkspace.shell.menu.forked', { id: forked.workspace_id }), { label: aiwsT('aiworkspace.shell.menu.openFork'), run: () => shell.openWorkspace(forked.workspace_id) }),
          (error: unknown) => store.notify('error', aiwsT('aiworkspace.shell.menu.forkFailed', { error: describeError(error) })),
        )
      },
    },
    {
      id: 'offline', label: t('aiworkspace.shell.menu.offline'), items: [
        { id: 'prepare-offline', label: t('aiworkspace.shell.prepareOffline'), disabled: sessionMode.kind !== 'direct' || sessionMode.reason !== 'not_prepared' || !live || shell.offlineBusy !== null,
          reason: sessionMode.kind === 'replica' ? t('aiworkspace.shell.menu.holdsReplica') : !live ? t('aiworkspace.shell.needsOnline') : t('aiworkspace.shell.menu.cannotOffline'),
          run: () => shell.runOffline(t('aiworkspace.shell.preparingOffline'), () => shell.offline.prepare(store.session.workspaceId, false)) },
        { id: 'takeover', label: t('aiworkspace.shell.takeover'), disabled: sessionMode.kind !== 'direct' || sessionMode.reason !== 'not_holder' || shell.offlineBusy !== null, reason: t('aiworkspace.shell.menu.takeoverReason'),
          run: () => shell.runOffline(t('aiworkspace.shell.takingOver'), () => shell.offline.reopen(store.session.workspaceId)) },
        { id: 'status', label: t('aiworkspace.shell.side.edits'), checked: shell.side === 'edits', run: () => shell.setSide('edits') },
        { id: 'export-local', label: t('aiworkspace.shell.exportLocal'), run: () => { void store.exportLocal() } },
      ],
    },
    ...(shell.devTools ? [{ id: 'dev', label: t('aiworkspace.shell.devTools'), items: [{ id: 'mock', testId: 'aiws-side-mock', label: t('aiworkspace.shell.menu.mock'), run: () => shell.openDialog({ kind: 'mock' }) }] } satisfies MenuItem] : []),
    { id: 'help', label: t('aiworkspace.shell.menu.help'), items: [{ id: 'shortcuts', label: t('aiworkspace.shell.help.title'), run: () => shell.openDialog({ kind: 'help' }) }] },
    ...(guest ? [] : [{ id: 'sep-5', separator: true } satisfies MenuItem, { id: 'close', testId: 'aiws-back', label: t('aiworkspace.shell.menu.close'), run: shell.close } satisfies MenuItem]),
  ]
}
