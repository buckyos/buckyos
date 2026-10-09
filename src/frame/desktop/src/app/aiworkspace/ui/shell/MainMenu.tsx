/* The main menu (UI improvement §6.1): the complete index of functions, grouped by task. Every item
 * calls the same action as its button, context menu entry or shortcut; an item that cannot run now is
 * disabled and says why. Deleting objects, Surfaces and the workspace keep distinct wording; the last
 * stays in the workspace list. */

import { unwrap } from '../../api/client'
import { describeError } from '../../api/session'
import type { Json } from '../../api/types'
import { pathsOf } from '../../presentation/model'
import { workspaceUrl } from '../../links'
import type { WorkspaceStore } from '../../state/store'
import { CANVAS_MODE_LABEL, CANVAS_MODES, CATALOG_GROUP_LABEL, type CanvasMode, type CatalogGroup } from '../blocks/registry'
import { extensionEntries, registryEntries, type CatalogEntry } from '../canvas/catalog'
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
  insert: (entry: CatalogEntry) => void
  openCatalog: (tab: CatalogTab, key?: string) => void
  copy: Command
  cut: Command
  paste: Command
  remove: Command
  /** Free Surfaces only. */
  view: { camera: Camera; fitAll: () => void; fitSelection: () => void; hasSelection: boolean } | null
  canvasTabs: SideTab[]
}

const NOT_ON_CANVAS = '在画布中可用'

export function buildMainMenu({ store, shell, canvas, mode }: { store: WorkspaceStore; shell: ShellApi; canvas: CanvasCommands | null; mode: CanvasMode }): MenuItem[] {
  const info = store.session.info()
  const caps = info.capabilities
  const live = store.session.status().kind === 'live'
  const undo = store.undo.snapshot()
  const sessionMode = store.session.mode()
  const insertReason = canvas ? canvas.insertReason : NOT_ON_CANVAS
  const groups: CatalogGroup[] = ['text', 'data', 'layout', 'ai']
  const entries = registryEntries()
  const extensions = extensionEntries(store)
  const insertItems: MenuItem[] = groups.map((group) => {
    const inGroup = entries.filter((entry) => entry.group === group || (group === 'data' && entry.group === 'sample'))
    return {
      id: `insert-group-${group}`, label: CATALOG_GROUP_LABEL[group], disabled: inGroup.length === 0,
      items: inGroup.map((entry) => ({ id: `insert-${entry.definition.type}`, testId: `aiws-main-insert-${entry.definition.type}`, label: `${entry.title}${entry.catalog.needs === 'none' ? '' : '…'}`, run: () => canvas?.insert(entry) })),
    }
  })
  insertItems.push({
    id: 'insert-group-extension', label: CATALOG_GROUP_LABEL.extension, disabled: extensions.length === 0, reason: '这个工作区还没有 Block 定义',
    items: extensions.map((entry) => ({ id: `insert-${entry.key}`, label: `${entry.title}…`, run: () => canvas?.openCatalog('extension', entry.key) })),
  })
  const side = (tab: SideTab): MenuItem => {
    const reason = canvas || tab === 'collab' || tab === 'edits' ? null : NOT_ON_CANVAS
    return { id: `side-${tab}`, label: SIDE_TAB_LABEL[tab], checked: shell.side === tab, disabled: reason !== null, reason, run: () => shell.setSide(shell.side === tab ? null : tab) }
  }
  const view = canvas?.view ?? null
  const viewReason = canvas ? (view ? null : '流式页没有画布缩放') : NOT_ON_CANVAS
  const notCanvas = shell.topMode !== 'canvas' ? NOT_ON_CANVAS : null
  const presentations = pathsOf(store.outline, 'presentation')
  const guides = pathsOf(store.outline, 'guide')
  return [
    { id: 'new-canvas', label: '新建画布…', disabled: !caps.includes('structure'), reason: '没有新建画布的权限', run: () => shell.openDialog({ kind: 'new', tab: 'canvas' }) },
    { id: 'new', label: '新建…', hint: '画布 / 工作区 / 模板', run: () => shell.openDialog({ kind: 'new', tab: caps.includes('structure') ? 'canvas' : 'workspace' }) },
    { id: 'open-workspace', label: '打开工作区…', hint: '返回工作区列表', run: shell.close },
    shell.home
      ? { id: 'home', label: '返回 BuckyOS 桌面', run: shell.home }
      : { id: 'new-tab', label: '在新标签页中打开', run: () => { window.open(workspaceUrl(store.session.workspaceId, { surfaceId: shell.topMode === 'canvas' ? shell.activeSurface?.entity_id ?? null : null }), '_blank', 'noopener') } },
    { id: 'sep-1', separator: true },
    { id: 'insert', label: '插入', disabled: insertReason !== null, reason: insertReason, explain: true, items: insertItems },
    { id: 'insert-catalog', label: '插入对象…', disabled: insertReason !== null, reason: insertReason, run: () => canvas?.openCatalog('all') },
    { id: 'add-data', label: '添加已有数据…', hint: '只创建视图', disabled: insertReason !== null, reason: insertReason, run: () => canvas?.openCatalog('existing') },
    {
      id: 'edit', label: '编辑', items: [
        { id: 'undo', label: '撤销', hint: 'Ctrl+Z', disabled: undo.undo.length === 0 || undo.busy, reason: '没有可撤销的步骤', run: () => { void store.undo.undo() } },
        { id: 'redo', label: '重做', hint: 'Ctrl+Shift+Z', disabled: undo.redo.length === 0 || undo.busy, reason: '没有可重做的步骤', run: () => { void store.undo.redo() } },
        { id: 'sep-e', separator: true },
        { id: 'copy', label: '复制', hint: 'Ctrl+C', disabled: !canvas || canvas.copy.reason !== null, reason: canvas?.copy.reason ?? NOT_ON_CANVAS, run: () => canvas?.copy.run() },
        { id: 'cut', label: '剪切', hint: 'Ctrl+X', disabled: !canvas || canvas.cut.reason !== null, reason: canvas?.cut.reason ?? NOT_ON_CANVAS, run: () => canvas?.cut.run() },
        { id: 'paste', label: '粘贴', hint: 'Ctrl+V', disabled: !canvas || canvas.paste.reason !== null, reason: canvas?.paste.reason ?? NOT_ON_CANVAS, run: () => canvas?.paste.run() },
        { id: 'delete-selection', label: '删除选中的对象', hint: 'Delete', disabled: !canvas || canvas.remove.reason !== null, reason: canvas?.remove.reason ?? NOT_ON_CANVAS, run: () => canvas?.remove.run() },
      ],
    },
    { id: 'sep-3', separator: true },
    {
      id: 'view', label: '视图', items: [
        { id: 'pref-object-toolbar', label: '对象工具栏', checked: shell.prefs.objectToolbar, run: () => shell.setPref('objectToolbar', !shell.prefs.objectToolbar) },
        { id: 'pref-view-toolbar', label: '视图工具栏', checked: shell.prefs.viewToolbar, run: () => shell.setPref('viewToolbar', !shell.prefs.viewToolbar) },
        { id: 'pref-grid', label: '网格', checked: shell.prefs.grid, run: () => shell.setPref('grid', !shell.prefs.grid) },
        { id: 'sep-v', separator: true },
        side('inspector'), side('relations'), side('annotations'), side('collab'), side('edits'),
        { id: 'sep-v2', separator: true },
        { id: 'reset-layout', label: '恢复默认布局', hint: '不改画布内容', run: shell.resetLayout },
      ],
    },
    {
      id: 'zoom', label: '缩放', disabled: viewReason !== null, reason: viewReason, items: view ? [
        ...ZOOM_PRESETS.map((value) => ({ id: `zoom-${value}`, label: `${value}%`, checked: Math.round(view.camera.zoom * 100) === value, run: () => view.camera.zoomTo(value / 100) })),
        { id: 'sep-z', separator: true },
        { id: 'fit-all', testId: 'aiws-menu-fit-all', label: '适应全部', run: view.fitAll },
        { id: 'fit-selection', testId: 'aiws-menu-fit-selection', label: '适应选区', disabled: !view.hasSelection, reason: '没有选中的对象', run: view.fitSelection },
      ] : [],
    },
    {
      id: 'canvas-mode', label: '画布模式', disabled: notCanvas !== null, reason: notCanvas, items: CANVAS_MODES.map((m) => ({
        id: `mode-${m}`, testId: `aiws-mode-${m}`, label: CANVAS_MODE_LABEL[m], checked: mode === m, run: () => store.userState.set('canvas:mode', m),
      })),
    },
    { id: 'start-presentation', testId: 'aiws-top-play', label: '开始放映…', disabled: presentations.length === 0, reason: '还没有演讲路径：在“路径编辑”中创建', explain: true, run: () => shell.startShow() },
    {
      id: 'guides', label: '使用引导', disabled: guides.length === 0, reason: '这个工作区没有使用引导',
      items: guides.map((g) => ({ id: `guide-${g.entity_id}`, testId: `aiws-menu-guide-${g.entity_id}`, label: g.title ?? g.name ?? g.entity_id, run: () => {
        const saved = store.userState.get<Json>(`guide:${g.entity_id}`) as { index?: number; done?: boolean } | undefined
        shell.setGuide({ pathId: g.entity_id, index: saved && !saved.done ? saved.index ?? 0 : 0 })
      } })),
    },
    { id: 'sep-4', separator: true },
    { id: 'import', label: '导入工作区包…', run: () => shell.openDialog({ kind: 'import' }) },
    { id: 'export', label: '导出…', hint: '分享包 / 个人备份', disabled: !caps.includes('export'), reason: '没有导出权限', run: () => shell.openDialog({ kind: 'export' }) },
    {
      id: 'fork', label: '创建工作区副本（Fork）', disabled: !caps.includes('manage') || !live, reason: !caps.includes('manage') ? '需要管理权限' : '需要联网',
      run: () => {
        shell.client.wsFork({ workspace_id: store.session.workspaceId }).then((result) => unwrap(result)).then(
          (forked) => store.notify('info', `已创建副本：新的工作区 ${forked.workspace_id}。`, { label: '打开副本', run: () => shell.openWorkspace(forked.workspace_id) }),
          (error: unknown) => store.notify('error', `创建副本失败：${describeError(error)}`),
        )
      },
    },
    {
      id: 'offline', label: '离线与同步', items: [
        { id: 'prepare-offline', label: '准备离线', disabled: sessionMode.kind !== 'direct' || sessionMode.reason !== 'not_prepared' || !live || shell.offlineBusy !== null,
          reason: sessionMode.kind === 'replica' ? '此窗口已持有离线副本' : !live ? '需要联网' : '此窗口不能启用离线（见修改状态）',
          run: () => shell.runOffline('正在准备离线…', () => shell.offline.prepare(store.session.workspaceId, false)) },
        { id: 'takeover', label: '接管离线副本', disabled: sessionMode.kind !== 'direct' || sessionMode.reason !== 'not_holder' || shell.offlineBusy !== null, reason: '只有另一个窗口持有副本时可用',
          run: () => shell.runOffline('正在接管…', () => shell.offline.reopen(store.session.workspaceId)) },
        { id: 'status', label: '修改状态', checked: shell.side === 'edits', run: () => shell.setSide('edits') },
        { id: 'export-local', label: '导出本机未提交内容', run: () => { void store.exportLocal() } },
      ],
    },
    ...(shell.devTools ? [{ id: 'dev', label: '开发工具', items: [{ id: 'mock', testId: 'aiws-side-mock', label: '受控加工（Mock）', run: () => shell.openDialog({ kind: 'mock' }) }] } satisfies MenuItem] : []),
    { id: 'help', label: '帮助', items: [{ id: 'shortcuts', label: '快捷键与操作说明', run: () => shell.openDialog({ kind: 'help' }) }] },
    { id: 'sep-5', separator: true },
    { id: 'close', testId: 'aiws-back', label: '关闭工作区', run: shell.close },
  ]
}
