/* The tab strip of the data mode (UI 框架 PRD §5.3): each tab its type's icon and name, an unsaved dot and a read-only
 * mark; the preview tab in italics. Click to make it current, double click to fix a preview, drag to reorder, the
 * middle button closes. What does not fit scrolls sideways, and the list button lists every tab. A tab's menu: close,
 * close the others, close those to the right, locate on a canvas, copy the path. */

import { createElement, useEffect, useRef, useState, useSyncExternalStore, type MouseEvent as ReactMouseEvent } from 'react'
import { ChevronDown, Lock, X } from 'lucide-react'
import { useOutlineVersion, useReadOnlyReason, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import type { MenuItem } from '../shell/popover'
import { FloatingMenu, type MenuAt } from './menus'
import { blocksShowing, iconOf, nameOf, pathText, readOnlyReason } from './model'
import { useDirty } from './hooks'
import { PERMISSIONS_TAB, type DataTab, type DataTabs } from './tabs'

const DRAG_MIME = 'application/x-aiws-data-tab'

export function TabStrip({ tabs, controller, onLocate }: { tabs: DataTabs; controller: { tabs: DataTab[]; active: string | null }; onLocate: (blockId: string) => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  useOutlineVersion()
  const [menu, setMenu] = useState<{ id: string; at: MenuAt } | null>(null)
  const [listAt, setListAt] = useState<MenuAt | null>(null)
  const strip = useRef<HTMLDivElement>(null)
  const { active } = controller
  // the current tab stays in view
  useEffect(() => {
    const el = strip.current?.querySelector<HTMLElement>(`[data-tab-id="${CSS.escape(active ?? '')}"]`)
    el?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [active, controller.tabs.length])
  const copyPath = async (id: string) => {
    const path = pathText(store, id)
    try { await navigator.clipboard.writeText(path); store.notify('info', t('aiworkspace.datamode.pathCopied', { path })) } catch { store.notify('info', t('aiworkspace.datamode.pathNotCopied', { path })) }
  }
  const tabMenu = (id: string): MenuItem[] => {
    const at = controller.tabs.findIndex((x) => x.id === id)
    const blocks = id === PERMISSIONS_TAB ? [] : blocksShowing(store, id)
    return [
      { id: 'close', testId: 'aiws-dm-tab-close-this', label: t('aiworkspace.datamode.tab.close'), run: () => tabs.close(id) },
      { id: 'close-others', testId: 'aiws-dm-tab-close-others', label: t('aiworkspace.datamode.tab.closeOthers'), disabled: controller.tabs.length < 2, run: () => tabs.closeOthers(id) },
      { id: 'close-right', testId: 'aiws-dm-tab-close-right', label: t('aiworkspace.datamode.tab.closeRight'), disabled: at === controller.tabs.length - 1, run: () => tabs.closeRight(id) },
      { id: 'sep', separator: true },
      { id: 'locate', testId: 'aiws-dm-tab-locate', label: t('aiworkspace.datamode.locate'), disabled: blocks.length === 0, reason: blocks.length === 0 ? t('aiworkspace.datamode.notOnCanvas') : null,
        items: blocks.map(({ block, surface }) => ({ id: block.entity_id, testId: `aiws-dm-tab-locate-${block.entity_id}`, label: t('aiworkspace.datamode.onCanvas', { canvas: surface ? nameOf(store, surface, surface.entity_id) : '', block: block.title ?? block.view_type ?? block.entity_id }), run: () => onLocate(block.entity_id) })) },
      { id: 'copy-path', testId: 'aiws-dm-tab-copy-path', label: t('aiworkspace.datamode.copyPath'), disabled: id === PERMISSIONS_TAB, run: () => { void copyPath(id) } },
    ]
  }
  return (
    <div className="aiws-dm-tabbar">
      <div ref={strip} className="aiws-dm-tabs" role="tablist" aria-label={t('aiworkspace.datamode.tabs')} data-testid="aiws-dm-tabs" data-count={controller.tabs.length}
        onDragOver={(event) => { if (event.dataTransfer.types.includes(DRAG_MIME)) event.preventDefault() }}
        onDrop={(event) => { const id = event.dataTransfer.getData(DRAG_MIME); if (id && event.target === event.currentTarget) tabs.move(id, null) }}>
        {controller.tabs.map((tab) => (
          <Tab key={tab.id} tab={tab} active={tab.id === active} tabs={tabs} onMenu={(at) => setMenu({ id: tab.id, at })} />
        ))}
      </div>
      {controller.tabs.length > 1 && (
        <button type="button" className="aiws-tool aiws-dm-tab-list" aria-label={t('aiworkspace.datamode.tab.list')} title={t('aiworkspace.datamode.tab.list')} data-testid="aiws-dm-tab-list"
          onClick={(event) => { const r = event.currentTarget.getBoundingClientRect(); setListAt({ x: r.right - 220, y: r.bottom + 2 }) }}><ChevronDown size={16} /></button>
      )}
      {menu && <FloatingMenu at={menu.at} items={tabMenu(menu.id)} label={t('aiworkspace.datamode.tab.menu')} testId="aiws-dm-tab-menu" onClose={() => setMenu(null)} />}
      {listAt && (
        <FloatingMenu at={listAt} label={t('aiworkspace.datamode.tab.list')} testId="aiws-dm-tab-list-menu" onClose={() => setListAt(null)}
          items={controller.tabs.map((tab) => ({ id: tab.id, testId: `aiws-dm-tab-list-${tab.id}`, label: nameOf(store, store.outline.get(tab.id), tab.id), checked: tab.id === active, run: () => tabs.activate(tab.id) }))} />
      )}
    </div>
  )
}

function Tab({ tab, active, tabs, onMenu }: { tab: DataTab; active: boolean; tabs: DataTabs; onMenu: (at: MenuAt) => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const windowReason = useReadOnlyReason()
  const entity = tab.id === PERMISSIONS_TAB ? undefined : store.outline.get(tab.id)
  useSyncExternalStore(store.locks.subscribe, store.locks.snapshot)
  const dirty = useDirty(tab.id)
  const name = nameOf(store, entity, tab.id)
  const gone = tab.id !== PERMISSIONS_TAB && (!entity || entity.deleted)
  const reason = tab.id === PERMISSIONS_TAB ? null : readOnlyReason(store, entity, windowReason)
  const [over, setOver] = useState(false)
  const menuAt = (event: ReactMouseEvent) => ({ x: event.clientX, y: event.clientY })
  return (
    <div role="tab" tabIndex={active ? 0 : -1} aria-selected={active} className={`aiws-dm-tab${active ? ' is-active' : ''}${tab.preview ? ' is-preview' : ''}${gone ? ' is-gone' : ''}${over ? ' is-over' : ''}`}
      data-testid="aiws-dm-tab" data-tab-id={tab.id} data-preview={tab.preview ? 'true' : 'false'} data-dirty={dirty ? 'true' : 'false'} data-read-only={reason ? 'true' : 'false'} title={reason ? `${name} · ${reason}` : name}
      draggable onDragStart={(event) => { event.dataTransfer.setData(DRAG_MIME, tab.id); event.dataTransfer.effectAllowed = 'move' }}
      onDragOver={(event) => { if (!event.dataTransfer.types.includes(DRAG_MIME)) return; event.preventDefault(); setOver(true) }}
      onDragLeave={() => setOver(false)}
      onDrop={(event) => { event.preventDefault(); setOver(false); const id = event.dataTransfer.getData(DRAG_MIME); if (id) tabs.move(id, tab.id) }}
      onClick={() => tabs.activate(tab.id)} onDoubleClick={() => tabs.pin(tab.id)}
      onAuxClick={(event) => { if (event.button === 1) { event.preventDefault(); tabs.close(tab.id) } }}
      onContextMenu={(event) => { event.preventDefault(); onMenu(menuAt(event)) }}
      onKeyDown={(event) => { if (event.key === 'Enter') tabs.activate(tab.id) }}>
      <span className="aiws-dm-tab-icon">{createElement(iconOf(entity, tab.id), { size: 15, 'aria-hidden': true })}</span>
      <span className="aiws-dm-tab-name">{name}</span>
      {reason && <span className="aiws-dm-tab-ro" data-testid="aiws-dm-tab-readonly" title={reason}><Lock size={12} aria-label={reason} /></span>}
      {dirty && <span className="aiws-dm-tab-dot" data-testid="aiws-dm-tab-dirty" title={t('aiworkspace.datamode.tab.unsaved')} />}
      <button type="button" className="aiws-dm-tab-close" aria-label={t('aiworkspace.datamode.tab.closeNamed', { name })} data-testid="aiws-dm-tab-close"
        onClick={(event) => { event.stopPropagation(); tabs.close(tab.id) }}><X size={13} aria-hidden="true" /></button>
    </div>
  )
}
