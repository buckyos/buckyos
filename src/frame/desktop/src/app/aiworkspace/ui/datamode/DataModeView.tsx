/* The data mode (UI 框架 PRD §5): like a text editor — the object tree on the left, the editing area with its tabs in the
 * middle, the right sidebar (属性 / 权限 / 修改记录), the status bar across the bottom. The title zone shows the
 * workspace's name, "返回画布" and the main menu; the collaboration zone stays, its comment is about the current tab's
 * object. The two side columns are dragged wider or narrower and folded away (the person's state, `ui:data-layout`); a
 * medium window turns the sidebar into a drawer, a narrow one the tree too. A phone has its own single column
 * (PhoneDataMode). The tree's actions run here (`DataHost`): open, the sidebar, dialogs, picking files, pasting tables. */

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type PointerEvent as ReactPointerEvent } from 'react'
import { Database, Ellipsis, LayoutDashboard, PanelLeft, PanelRight } from 'lucide-react'
import { DATA_ID, type EntityEnvelope, type Json } from '../../api/types'
import { aiwsT } from '../../i18n'
import { useOutlineVersion, useStore, useUserState, useWorkspaceUi } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { TABULAR_ACCEPT, tabularKind } from '@table/files'
import { looksLikeHtmlTable, looksLikeTsv } from '@table/mod'
import { useActionContext } from '../framework/hooks'
import { useMacroWatch } from '../framework/macros'
import { clipboardTable } from '../import/clipboardTable'
import { tableImports } from '../import/tableImportRequest'
import { AnnotationsPanel } from '../shell/AnnotationsPanel'
import { useFilePicker } from '../shell/filePicker'
import { buildMainMenu, type CanvasCommands, type CommentCommand } from '../shell/MainMenu'
import { MenuButton } from '../shell/popover'
import { useShell } from '../shell/shellContext'
import { SidePanel } from '../shell/SidePanel'
import { StatusDock } from '../shell/StatusSummary'
import { ViewToolbar } from '../shell/ViewToolbar'
import type { DataDialog, DataHost, PickKind, PropSection, SideTab } from './actions'
import { DataDialogs } from './dialogs'
import { TabEditor } from './editors'
import { EditorHost } from './EditorHost'
import { useCoarse } from './hooks'
import { acceptsData, nameOf, SIDE_TAB_KEY, sideTabsOf } from './model'
import { ObjectTree } from './ObjectTree'
import { importFiles, type Place } from './ops'
import { PhoneDataMode } from './PhoneDataMode'
import { SideContent } from './SideBar'
import { ObjectInfo } from './StatusInfo'
import { TabStrip } from './TabStrip'
import { dataTabsOf, PERMISSIONS_TAB, type DataTabsState } from './tabs'
import './datamode.css'

const LAYOUT_KEY = 'ui:data-layout'
interface Layout { left: number; right: number; leftHidden: boolean; rightHidden: boolean }
const DEFAULT_LAYOUT: Layout = { left: 280, right: 320, leftHidden: false, rightHidden: false }
const clampWidth = (w: number) => Math.max(180, Math.min(560, Math.round(w)))

function parseLayout(value: Json | undefined): Layout {
  const raw = value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, Json> : {}
  return {
    left: typeof raw.left === 'number' ? clampWidth(raw.left) : DEFAULT_LAYOUT.left,
    right: typeof raw.right === 'number' ? clampWidth(raw.right) : DEFAULT_LAYOUT.right,
    leftHidden: raw.leftHidden === true, rightHidden: raw.rightHidden === true,
  }
}

/** The tree's actions, the sidebar's tab and the dialogs, for both layouts. */
export interface DataController {
  state: DataTabsState
  activeEntity: EntityEnvelope | undefined
  host: DataHost
  sideTab: SideTab
  setSideTab: (tab: SideTab) => void
  focus: { section: PropSection; nonce: number } | null
  /** The sidebar was asked for (a menu's "权限…" or "发布…"): a drawer or a phone's panel opens. */
  sideAsked: number
  dialogs: React.ReactNode
}

function useDataController(): DataController {
  const store = useStore()
  const { t } = useAiwsText()
  useOutlineVersion()
  const tabs = dataTabsOf(store)
  const state = useSyncExternalStore(tabs.subscribe, tabs.snapshot)
  const activeEntity = state.active && state.active !== PERMISSIONS_TAB ? store.outline.get(state.active) : undefined
  const sideState = useUserState<string>(SIDE_TAB_KEY)
  const sideTab: SideTab = state.active === PERMISSIONS_TAB ? 'history' : sideState === 'permissions' || sideState === 'history' ? sideState : 'properties'
  const setSideTab = useCallback((tab: SideTab) => store.userState.set(SIDE_TAB_KEY, tab === 'properties' ? null : tab), [store])
  const [focus, setFocus] = useState<{ section: PropSection; nonce: number } | null>(null)
  const [sideAsked, setSideAsked] = useState(0)
  const [dialog, setDialog] = useState<DataDialog | null>(null)
  const pickFile = useFilePicker((message) => store.notify('error', message))
  const local = useRef<HTMLInputElement>(null)
  const localPlace = useRef<Place | null>(null)
  const importing = useRef<AbortController | null>(null)
  useEffect(() => () => importing.current?.abort(), [])
  const open = useCallback((id: string, options = {}) => tabs.open(id, options), [tabs])
  /** A single table file asks: a table, or a file (表格数据导入 TODO §5.1); the rest are assets. */
  const takeFiles = useCallback(async (files: File[], place: Place, folder: boolean) => {
    if (files.length === 0) return
    if (!folder && files.length === 1 && tabularKind(files[0].name, new Uint8Array(await files[0].slice(0, 64 * 1024).arrayBuffer()))) {
      if (tableImports.busy) { store.notify('info', aiwsT('aiworkspace.import.busy')); return }
      const bytes = new Uint8Array(await files[0].arrayBuffer())
      tableImports.open({ source: { kind: 'file', name: files[0].name, bytes }, step: 'ask', destination: { folderId: place.parentId },
        insertAsFile: () => { void importFiles(store, place, files, false).then((id) => { if (id) tabs.open(id) }) } })
      return
    }
    importing.current?.abort()
    const controller = new AbortController()
    importing.current = controller
    const id = await importFiles(store, place, files, folder, controller.signal)
    if (id) tabs.open(id, { preview: true })
  }, [store, tabs])
  const pasteTable = useCallback(async (place: Place) => {
    if (tableImports.busy) { store.notify('info', aiwsT('aiworkspace.import.busy')); return }
    try {
      let html: string | null = null
      let text: string | null = null
      if (navigator.clipboard?.read) {
        for (const item of await navigator.clipboard.read()) {
          if (item.types.includes('text/html')) html = await (await item.getType('text/html')).text()
          if (item.types.includes('text/plain')) text = await (await item.getType('text/plain')).text()
        }
      } else text = await navigator.clipboard.readText()
      const source = html && looksLikeHtmlTable(html) ? { kind: 'clipboard' as const, html, text } : text && looksLikeTsv(text) ? { kind: 'clipboard' as const, html: null, text } : null
      if (!source) { store.notify('info', t('aiworkspace.datamode.pasteNone')); return }
      tableImports.open({ source, step: 'wizard', destination: { folderId: place.parentId } })
    } catch {
      store.notify('info', t('aiworkspace.datamode.pasteDenied'))
    }
  }, [store, t])
  const host = useMemo<DataHost>(() => ({
    open,
    side: (tab, section) => {
      setSideTab(tab)
      if (section) setFocus({ section, nonce: Date.now() })
      setSideAsked(Date.now())
    },
    dialog: setDialog,
    pick: (kind: PickKind, place: Place) => {
      if (kind === 'device') { localPlace.current = place; local.current?.click(); return }
      if (kind === 'table') {
        void pickFile.file(TABULAR_ACCEPT).then(async (file) => {
          if (!file) return
          if (tableImports.busy) { store.notify('info', aiwsT('aiworkspace.import.busy')); return }
          tableImports.open({ source: { kind: 'file', name: file.name, bytes: new Uint8Array(await file.arrayBuffer()) }, step: 'wizard', destination: { folderId: place.parentId } })
        })
        return
      }
      void pickFile.files(kind === 'folder' ? { folder: true } : { multiple: true }).then((files) => { if (files?.length) void takeFiles(files, place, kind === 'folder') })
    },
    pasteTable: (place) => { void pasteTable(place) },
  }), [open, setSideTab, pickFile, store, takeFiles, pasteTable])
  // a table pasted while the data mode has the focus goes into the current folder (表格数据导入 TODO §4)
  useEffect(() => {
    const onPaste = (event: ClipboardEvent) => {
      const el = event.target instanceof Element ? event.target : null
      if (el?.closest('input, textarea, select, [contenteditable="true"], [contenteditable="plaintext-only"], .aiws-dialog') || !(el?.closest('.aiws-dm') || el === document.body)) return
      const table = event.clipboardData ? clipboardTable(event.clipboardData) : null
      if (!table || tableImports.busy) return
      const current = activeEntity && acceptsData(activeEntity) ? activeEntity.entity_id : activeEntity?.parent_id && acceptsData(store.outline.get(activeEntity.parent_id)) ? activeEntity.parent_id : DATA_ID
      if (!store.outline.get(current)?.capabilities.includes('structure')) return
      event.preventDefault()
      tableImports.open({ source: table, step: 'wizard', destination: { folderId: current } })
    }
    window.addEventListener('paste', onPaste)
    return () => window.removeEventListener('paste', onPaste)
  }, [store, activeEntity])
  const dialogs = (
    <>
      <input ref={local} type="file" multiple hidden data-testid="aiws-dm-local-input" onChange={(event) => {
        const files = [...(event.target.files ?? [])]
        event.target.value = ''
        const place = localPlace.current
        if (place && files.length) void takeFiles(files, place, false)
      }} />
      {dialog && <DataDialogs request={dialog} onClose={() => setDialog(null)} onCreated={(id) => tabs.open(id)} />}
    </>
  )
  return { state, activeEntity, host, sideTab, setSideTab, focus, sideAsked, dialogs }
}

/** The data mode as the workspace shell mounts it; `onActive` tells the shell the current object (comments in scope). */
export function DataModeStage({ onActive }: { onActive: (id: string | null) => void }) {
  const shell = useShell()
  const data = useDataController()
  useMacroWatch()
  const active = data.state.active
  useEffect(() => onActive(active && active !== PERMISSIONS_TAB ? active : null), [active, onActive])
  return shell.phone ? <PhoneDataMode data={data} /> : <DesktopDataMode data={data} />
}

/** The title zone in the data mode (§5.1): the workspace, "返回画布", the main menu. */
function DataTitle() {
  const { t } = useAiwsText()
  const store = useStore()
  const shell = useShell()
  const actions = useActionContext(null)
  return (
    <div className="aiws-main-area">
      <div className="aiws-panel aiws-main-toolbar" role="toolbar" aria-label={t('aiworkspace.shell.toolbar.main')} data-testid="aiws-main-toolbar">
        <span className="aiws-main-title" data-testid="aiws-dm-workspace-title"><LayoutDashboard size={18} aria-hidden="true" /> <b>{store.session.info().title}</b></span>
        <span className="aiws-chip aiws-dm-mode-chip"><Database size={13} aria-hidden="true" /> {t('aiworkspace.framework.dataMode')}</span>
        <span className="aiws-toolbar-sep" aria-hidden="true" />
        <button type="button" className="aiws-tool" aria-pressed aria-label={t('aiworkspace.shell.toolbar.backToCanvas')} title={t('aiworkspace.shell.toolbar.backToCanvasHint')}
          data-testid="aiws-top-canvas" data-action="view.data-mode" onClick={() => shell.setTopMode('canvas')}><LayoutDashboard size={18} /></button>
        <MenuButton label={t('aiworkspace.shell.toolbar.mainMenu')} title={t('aiworkspace.shell.toolbar.mainMenu')} testId="aiws-main-menu" menuTestId="aiws-main-menu-list" items={() => buildMainMenu(actions)}>
          <Ellipsis size={18} />
        </MenuButton>
      </div>
    </div>
  )
}

/** What the collaboration zone and its comment see in the data mode: no canvas, a comment about the current object. */
function useDataCommands(entity: EntityEnvelope | undefined): CanvasCommands {
  const store = useStore()
  const shell = useShell()
  const ui = useWorkspaceUi()
  const { t } = useAiwsText()
  return useMemo(() => {
    const away = { reason: t('aiworkspace.shell.notOnCanvas'), run: () => undefined }
    const comment: CommentCommand = {
      reason: !ui.annotate ? t('aiworkspace.datamode.comment.noPermission') : !entity ? t('aiworkspace.datamode.comment.pick') : null, active: false,
      run: () => {
        if (!entity || !ui.annotate) return
        ui.annotate({ target: { entity_id: entity.entity_id }, label: nameOf(store, entity, entity.entity_id) })
        shell.setSide('annotations')
      },
    }
    return {
      isFree: false, insertReason: away.reason, noteReason: away.reason, insert: () => undefined, openCatalog: () => undefined,
      copy: away, cut: away, paste: away, remove: away, importDrawio: away, importPdf: away, importOffice: away, importTable: away, exportDrawio: away,
      view: null, canvasTabs: [], comment,
    }
  }, [store, shell, ui, entity, t])
}

function DesktopDataMode({ data }: { data: DataController }) {
  const { t } = useAiwsText()
  const store = useStore()
  const shell = useShell()
  const ui = useWorkspaceUi()
  const coarse = useCoarse()
  const tabs = dataTabsOf(store)
  const { state, activeEntity, host } = data
  const layoutState = useUserState<Json>(LAYOUT_KEY)
  const layout = useMemo(() => parseLayout(layoutState), [layoutState])
  const setLayout = (next: Partial<Layout>) => store.userState.set(LAYOUT_KEY, { ...layout, ...next } as unknown as Json)
  const [drag, setDrag] = useState<{ side: 'left' | 'right'; width: number } | null>(null)
  // a medium window: the sidebar is a drawer; a narrow one: the tree too (§5.1)
  const rightDrawer = shell.size !== 'wide'
  const leftDrawer = shell.size === 'narrow'
  const [drawer, setDrawer] = useState<'left' | 'right' | null>(null)
  // a menu asked for the sidebar ("权限…", "发布…"): it shows until it is put away again
  const [seenAsk, setSeenAsk] = useState(0)
  const asked = data.sideAsked > seenAsk
  const leftShown = leftDrawer ? drawer === 'left' : !layout.leftHidden
  const rightShown = rightDrawer ? drawer === 'right' || asked : !layout.rightHidden || asked
  const hideRight = () => { setSeenAsk(data.sideAsked); if (rightDrawer) setDrawer(null); else setLayout({ rightHidden: true }) }
  const leftWidth = drag?.side === 'left' ? drag.width : layout.left
  const rightWidth = drag?.side === 'right' ? drag.width : layout.right
  const startDrag = (side: 'left' | 'right') => (event: ReactPointerEvent<HTMLDivElement>) => {
    event.preventDefault()
    const x0 = event.clientX
    const w0 = side === 'left' ? layout.left : layout.right
    const el = event.currentTarget
    el.setPointerCapture(event.pointerId)
    let width = w0
    const onMove = (e: PointerEvent) => { width = clampWidth(side === 'left' ? w0 + e.clientX - x0 : w0 - (e.clientX - x0)); setDrag({ side, width }) }
    const onUp = () => {
      el.removeEventListener('pointermove', onMove)
      el.removeEventListener('pointerup', onUp)
      el.removeEventListener('pointercancel', onUp)
      setDrag(null)
      if (width !== w0) setLayout(side === 'left' ? { left: width } : { right: width })
    }
    el.addEventListener('pointermove', onMove)
    el.addEventListener('pointerup', onUp)
    el.addEventListener('pointercancel', onUp)
  }
  const activeTab = state.tabs.find((tab) => tab.id === state.active)
  const commands = useDataCommands(activeEntity)
  const commentFolder = activeEntity ? (acceptsData(activeEntity) ? activeEntity.entity_id : activeEntity.parent_id && acceptsData(store.outline.get(activeEntity.parent_id)) ? activeEntity.parent_id : DATA_ID) : DATA_ID
  const openFromTree: DataHost = useMemo(() => ({ ...host, open: (id, options) => { host.open(id, options); if (leftDrawer) setDrawer(null) } }), [host, leftDrawer])
  const sideTabs = sideTabsOf(state.active)
  const tree = <ObjectTree activeId={state.active} host={openFromTree} phone={false} coarse={coarse} />
  const side = (
    <>
      <div className="aiws-dm-side-tabs" role="tablist" aria-label={t('aiworkspace.datamode.side.label')}>
        {sideTabs.map((tab) => <button key={tab} type="button" role="tab" aria-selected={data.sideTab === tab} data-testid={`aiws-dm-side-tab-${tab}`} onClick={() => data.setSideTab(tab)}>{t(`aiworkspace.datamode.side.${tab}`)}</button>)}
        <span className="aiws-grow" />
        <button type="button" className="aiws-tool aiws-tool-small" aria-label={t('aiworkspace.datamode.side.hide')} title={t('aiworkspace.datamode.side.hide')} data-testid="aiws-dm-side-hide"
          onClick={hideRight}><PanelRight size={16} /></button>
      </div>
      <div className="aiws-dm-side-body"><SideContent id={state.active} entity={activeEntity} tab={data.sideTab} focus={data.focus} /></div>
    </>
  )
  return (
    <div className="aiws-sources-stage aiws-dm" data-testid="aiws-dm" data-layout="desktop">
      <div className="aiws-sources-bar aiws-dm-bar">
        <DataTitle />
        <span className="aiws-grow" />
        {!shell.prefs.viewToolbar ? null : <ViewToolbar canvas={commands} />}
      </div>
      <div className="aiws-sources-row">
        <div className="aiws-sources-work aiws-dm-work">
          <div className="aiws-sources aiws-dm-body" data-testid="aiws-sources" data-active={state.active ?? ''}>
            {leftShown && (
              <aside className={`aiws-dm-left${leftDrawer ? ' is-drawer' : ''}`} style={leftDrawer ? undefined : { width: leftWidth }} data-testid="aiws-dm-left">
                {tree}
                <button type="button" className="aiws-tool aiws-tool-small aiws-dm-left-hide" aria-label={t('aiworkspace.datamode.tree.hide')} title={t('aiworkspace.datamode.tree.hide')} data-testid="aiws-dm-tree-hide"
                  onClick={() => (leftDrawer ? setDrawer(null) : setLayout({ leftHidden: true }))}><PanelLeft size={16} /></button>
              </aside>
            )}
            {leftShown && !leftDrawer && <div className="aiws-dm-splitter" role="separator" aria-orientation="vertical" aria-label={t('aiworkspace.datamode.resizeTree')} data-testid="aiws-dm-splitter-left" onPointerDown={startDrag('left')} />}
            <main className="aiws-dm-main">
              <div className="aiws-dm-mainbar">
                {!leftShown && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.datamode.tree.show')} title={t('aiworkspace.datamode.tree.show')} data-testid="aiws-dm-tree-show"
                  onClick={() => (leftDrawer ? setDrawer('left') : setLayout({ leftHidden: false }))}><PanelLeft size={18} /></button>}
                <TabStrip tabs={tabs} controller={state} onLocate={(id) => ui.openEntity(id)} />
                {!rightShown && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.datamode.side.show')} title={t('aiworkspace.datamode.side.show')} data-testid="aiws-dm-side-show"
                  onClick={() => (rightDrawer ? setDrawer('right') : setLayout({ rightHidden: false }))}><PanelRight size={18} /></button>}
              </div>
              {activeTab ? (
                <EditorHost key={activeTab.id} tab={activeTab} tabs={tabs}>
                  <TabEditor id={activeTab.id} entity={activeTab.id === PERMISSIONS_TAB ? undefined : store.outline.get(activeTab.id)} page={activeTab.page}
                    onPage={(page) => tabs.setPage(activeTab.id, page)} onOpen={(id) => tabs.open(id, { preview: true })} />
                </EditorHost>
              ) : (
                <div className="aiws-empty" data-testid="aiws-detail-empty">
                  <p>{t('aiworkspace.datamode.empty')}</p>
                  <p className="aiws-muted">{t('aiworkspace.datamode.emptyHint')}</p>
                </div>
              )}
            </main>
            {rightShown && !rightDrawer && <div className="aiws-dm-splitter" role="separator" aria-orientation="vertical" aria-label={t('aiworkspace.datamode.resizeSide')} data-testid="aiws-dm-splitter-right" onPointerDown={startDrag('right')} />}
            {rightShown && <aside className={`aiws-dm-side${rightDrawer ? ' is-drawer' : ''}`} style={rightDrawer ? undefined : { width: rightWidth }} data-testid="aiws-dm-side" data-tab={data.sideTab}>{side}</aside>}
            <SidePanel tabs={['annotations']} render={() => <AnnotationsPanel parentId={commentFolder} />} />
          </div>
          <StatusDock info={<ObjectInfo id={state.active} entity={activeEntity} />} />
        </div>
      </div>
      {data.dialogs}
    </div>
  )
}
