/* The data mode on a phone (UI 框架 PRD §5.6): one column in three layers. (1) The object tree, the whole screen: search
 * and filters on top, rows at least 48 px with their ⋯ always shown. (2) The editor of the current object, the whole
 * screen: a top bar with "back", the object's name, how many objects are open (tap to switch, like the tabs) and ⋯.
 * (3) The right sidebar's three tabs as a bottom panel, from ⋯ or from the object's line in the status bar. "Back" from
 * an editor that a canvas's "展开" opened goes straight back to the canvas. */

import { createElement, useState } from 'react'
import { ArrowLeft, ChevronLeft, Ellipsis, LayoutDashboard, X } from 'lucide-react'
import { DATA_ID, type EntityEnvelope } from '../../api/types'
import { useOverlayMounted, type MenuItem } from '../shell/popover'
import { AnnotationsPanel } from '../shell/AnnotationsPanel'
import { SidePanel } from '../shell/SidePanel'
import { useStore, useWorkspaceUi } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { useActionContext } from '../framework/hooks'
import { useShell } from '../shell/shellContext'
import { StatusDock } from '../shell/StatusSummary'
import { dataMenu, type SideTab } from './actions'
import type { DataController } from './DataModeView'
import { TabEditor } from './editors'
import { SheetMenu } from './menus'
import { acceptsData, iconOf, nameOf, sideTabsOf } from './model'
import { ObjectTree } from './ObjectTree'
import { PhoneMainMenu } from './PhoneMenu'
import { SideContent } from './SideBar'
import { ObjectInfo } from './StatusInfo'
import { useDirty } from './hooks'
import { dataTabsOf, PERMISSIONS_TAB } from './tabs'

export function PhoneDataMode({ data }: { data: DataController }) {
  const { t } = useAiwsText()
  const store = useStore()
  const shell = useShell()
  const ui = useWorkspaceUi()
  const actions = useActionContext(null)
  const tabs = dataTabsOf(store)
  const { state, activeEntity, host } = data
  const [chosen, setPanel] = useState<SideTab | null>(null)
  // a menu asked for the sidebar ("权限…", "发布…"): its tab as the bottom panel, until it is closed
  const [seenAsk, setSeenAsk] = useState(0)
  const panel = chosen ?? (data.sideAsked > seenAsk ? data.sideTab : null)
  const closePanel = () => { setPanel(null); setSeenAsk(data.sideAsked) }
  const [switcher, setSwitcher] = useState(false)
  const [more, setMore] = useState(false)
  const activeTab = state.tabs.find((tab) => tab.id === state.active)
  const editing = state.phone.layer === 'editor' && activeTab
  const back = () => {
    if (state.phone.returnTo === 'canvas') { tabs.showTree(); shell.setTopMode('canvas'); return }
    tabs.showTree()
  }
  const name = activeTab ? nameOf(store, activeEntity, activeTab.id) : ''
  const canComment = Boolean(ui.annotate) && Boolean(activeEntity)
  const commentFolder = activeEntity?.parent_id && acceptsData(store.outline.get(activeEntity.parent_id)) ? activeEntity.parent_id : DATA_ID
  const moreItems: MenuItem[] = activeTab ? [
    ...sideTabsOf(activeTab.id).map((tab): MenuItem => ({ id: `side-${tab}`, testId: `aiws-dm-phone-side-${tab}`, label: t(`aiworkspace.datamode.side.${tab}`), run: () => { data.setSideTab(tab); setPanel(tab) } })),
    ...(canComment ? [{ id: 'sep-comment', separator: true }, { id: 'comment', testId: 'aiws-dm-phone-comment', label: t('aiworkspace.shell.view.annotate'), run: () => {
      if (!activeEntity || !ui.annotate) return
      ui.annotate({ target: { entity_id: activeEntity.entity_id }, label: name })
      shell.setSide('annotations')
    } } as MenuItem] : []),
    ...(activeTab.id !== PERMISSIONS_TAB ? [{ id: 'sep-object', separator: true } as MenuItem, ...dataMenu({ ...actions, target: activeTab.id, host }).filter((item) => item.id !== 'open' && item.id !== 'open-tab')] : []),
    { id: 'sep-close', separator: true },
    { id: 'close', testId: 'aiws-dm-phone-close', label: t('aiworkspace.datamode.tab.close'), run: () => tabs.close(activeTab.id) },
  ] : []
  return (
    <div className="aiws-sources-stage aiws-dm is-phone" data-testid="aiws-dm" data-layout="phone" data-layer={editing ? 'editor' : 'tree'}>
      {editing ? (
        <div className="aiws-dm-phone-bar" data-testid="aiws-dm-phone-bar">
          <button type="button" className="aiws-tool" aria-label={state.phone.returnTo === 'canvas' ? t('aiworkspace.shell.toolbar.backToCanvas') : t('aiworkspace.datamode.back')} data-testid="aiws-dm-phone-back" onClick={back}>
            {state.phone.returnTo === 'canvas' ? <ArrowLeft size={20} /> : <ChevronLeft size={22} />}
          </button>
          <span className="aiws-dm-phone-title" data-testid="aiws-dm-phone-title">{createElement(iconOf(activeEntity, activeTab.id), { size: 16, 'aria-hidden': true })}<b>{name}</b></span>
          <button type="button" className="aiws-tool aiws-dm-phone-count" aria-label={t('aiworkspace.datamode.phone.open', { count: state.tabs.length })} data-testid="aiws-dm-phone-tabs" onClick={() => setSwitcher(true)}>
            <span>{state.tabs.length}</span>
          </button>
          <button type="button" className="aiws-tool" aria-label={t('aiworkspace.datamode.phone.more')} data-testid="aiws-dm-phone-more" onClick={() => setMore(true)}><Ellipsis size={20} /></button>
        </div>
      ) : (
        <div className="aiws-dm-phone-bar" data-testid="aiws-dm-phone-bar">
          <button type="button" className="aiws-tool" aria-label={t('aiworkspace.shell.toolbar.backToCanvas')} data-testid="aiws-top-canvas" data-action="view.data-mode" onClick={() => shell.setTopMode('canvas')}><LayoutDashboard size={20} /></button>
          <span className="aiws-dm-phone-title" data-testid="aiws-dm-workspace-title"><b>{store.session.info().title}</b></span>
          {state.tabs.length > 0 && (
            <button type="button" className="aiws-tool aiws-dm-phone-count" aria-label={t('aiworkspace.datamode.phone.open', { count: state.tabs.length })} data-testid="aiws-dm-phone-tabs" onClick={() => setSwitcher(true)}><span>{state.tabs.length}</span></button>
          )}
          <PhoneMainMenu canvas={null} />
        </div>
      )}
      <div className="aiws-sources-work aiws-dm-work">
        <div className="aiws-sources aiws-dm-body" data-testid="aiws-sources" data-active={state.active ?? ''}>
          {editing ? (
            <main className="aiws-dm-main">
              <div className="aiws-dm-editor" data-testid="aiws-dm-editor" data-tab-id={activeTab.id}>
                <TabEditor key={activeTab.id} id={activeTab.id} entity={activeTab.id === PERMISSIONS_TAB ? undefined : store.outline.get(activeTab.id)} page={activeTab.page}
                  onPage={(page) => tabs.setPage(activeTab.id, page)} onOpen={(id) => tabs.open(id)} />
              </div>
            </main>
          ) : (
            <aside className="aiws-dm-left is-phone" data-testid="aiws-dm-left">
              <ObjectTree activeId={state.active} host={{ ...host, open: (id, options) => host.open(id, { ...options, preview: false }) }} phone coarse />
            </aside>
          )}
        </div>
        <StatusDock info={<ObjectInfo id={editing ? state.active : null} entity={editing ? activeEntity : undefined} onOpen={editing ? () => setPanel(data.sideTab) : undefined} />} />
      </div>
      {panel && activeTab && <SidePanelSheet id={activeTab.id} entity={activeEntity} tab={sideTabsOf(activeTab.id).includes(panel) ? panel : 'history'} onTab={(tab) => { data.setSideTab(tab); setPanel(tab) }} focus={data.focus} onClose={closePanel} />}
      <SidePanel tabs={['annotations']} render={() => <AnnotationsPanel parentId={commentFolder} />} />
      {switcher && <Switcher onClose={() => setSwitcher(false)} />}
      {more && <SheetMenu items={moreItems} label={name} testId="aiws-dm-phone-menu" onClose={() => setMore(false)} />}
      {data.dialogs}
    </div>
  )
}

/** The right sidebar's tabs as a bottom panel (§5.6 layer 3). */
function SidePanelSheet({ id, entity, tab, onTab, focus, onClose }: { id: string; entity: EntityEnvelope | undefined; tab: SideTab; onTab: (tab: SideTab) => void; focus: DataController['focus']; onClose: () => void }) {
  const { t } = useAiwsText()
  useOverlayMounted(true)
  return (
    <div className="aiws-dm-sheet-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
      <div className="aiws-dm-sheet is-tall" role="dialog" aria-label={t(`aiworkspace.datamode.side.${tab}`)} data-testid="aiws-dm-phone-panel" data-tab={tab}>
        <div className="aiws-dm-sheet-grip" aria-hidden="true" />
        <div className="aiws-dm-sheet-head">
          <div className="aiws-dm-side-tabs" role="tablist" aria-label={t('aiworkspace.datamode.side.label')}>
            {sideTabsOf(id).map((x) => <button key={x} type="button" role="tab" aria-selected={tab === x} data-testid={`aiws-dm-side-tab-${x}`} onClick={() => onTab(x)}>{t(`aiworkspace.datamode.side.${x}`)}</button>)}
          </div>
          <span className="aiws-grow" />
          <button type="button" className="aiws-tool" aria-label={t('aiworkspace.common.close')} data-testid="aiws-dm-phone-panel-close" onClick={onClose}><X size={20} /></button>
        </div>
        <div className="aiws-dm-sheet-body aiws-dm-side-body"><SideContent id={id} entity={entity} tab={tab} focus={focus} /></div>
      </div>
    </div>
  )
}

/** The open objects, like the tabs: tap one to switch, × to close it. */
function Switcher({ onClose }: { onClose: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const tabs = dataTabsOf(store)
  const state = tabs.snapshot()
  useOverlayMounted(true)
  return (
    <div className="aiws-dm-sheet-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
      <div className="aiws-dm-sheet" role="dialog" aria-label={t('aiworkspace.datamode.tabs')} data-testid="aiws-dm-phone-switcher">
        <div className="aiws-dm-sheet-grip" aria-hidden="true" />
        <div className="aiws-dm-sheet-head"><b>{t('aiworkspace.datamode.phone.opened', { count: state.tabs.length })}</b><span className="aiws-grow" /><button type="button" className="aiws-tool" aria-label={t('aiworkspace.common.close')} onClick={onClose}><X size={20} /></button></div>
        <div className="aiws-dm-sheet-body">
          {state.tabs.map((tab) => <SwitchRow key={tab.id} id={tab.id} active={tab.id === state.active} onPick={() => { tabs.activate(tab.id); onClose() }} onClose={() => tabs.close(tab.id)} />)}
        </div>
      </div>
    </div>
  )
}

function SwitchRow({ id, active, onPick, onClose }: { id: string; active: boolean; onPick: () => void; onClose: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const entity = id === PERMISSIONS_TAB ? undefined : store.outline.get(id)
  const dirty = useDirty(id)
  const name = nameOf(store, entity, id)
  return (
    <div className={`aiws-dm-switch${active ? ' is-active' : ''}`} data-testid="aiws-dm-phone-switch" data-tab-id={id}>
      <button type="button" className="aiws-dm-sheet-item" onClick={onPick}>{createElement(iconOf(entity, id), { size: 18, 'aria-hidden': true })}<span className="aiws-menu-label">{name}</span>{dirty && <span className="aiws-dm-tab-dot" />}</button>
      <button type="button" className="aiws-tool" aria-label={t('aiworkspace.datamode.tab.closeNamed', { name })} data-testid="aiws-dm-phone-switch-close" onClick={onClose}><X size={18} /></button>
    </div>
  )
}
