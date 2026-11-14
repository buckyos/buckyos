/* The common Block actions that are new with the framework (UI 框架 PRD §10.5): copy a link to the Block, expand it, and
 * the actions a data control publishes from its body (its view menu) so the Block menu lists them too. */

import { useEffect, useSyncExternalStore } from 'react'
import type { LucideIcon } from 'lucide-react'
import { isDataControl } from '@controls/contract'
import type { EntityEnvelope } from '../../api/types'
import { aiwsT } from '../../i18n'
import { targetQuery, workspacePageUrl } from '../../links'
import { Emitter } from '../../state/emitter'
import type { WorkspaceStore } from '../../state/store'
import { requestIntent } from '../blocks/editorToolbar'
import { openControlPage } from '../controls/data/focus'

/** The address of a Block: the Workspace page naming its canvas and the Block, else this page's address with the same query. */
export function blockLink(store: WorkspaceStore, surface: EntityEnvelope, blockId: string): string {
  const info = store.session.info()
  const page = workspacePageUrl(info.public_base, info.title, { id: surface.entity_id, title: surface.title ?? surface.name ?? '' }, blockId)
  if (page) return page
  const url = new URL(window.location.href)
  url.search = targetQuery(surface.entity_id, blockId).toString()
  return url.toString()
}

/** Copy the Block's link (a notice says so; where the clipboard is refused the notice shows the link). */
export function copyBlockLink(store: WorkspaceStore, surface: EntityEnvelope, blockId: string) {
  const link = blockLink(store, surface, blockId)
  const done = () => store.notify('info', aiwsT('aiworkspace.blockui.linkCopied'))
  const refused = () => store.notify('info', aiwsT('aiworkspace.blockui.linkManual', { link }))
  if (!navigator.clipboard) { refused(); return }
  void navigator.clipboard.writeText(link).then(done, refused)
}

/** "Expand" (§6.4, §10.5): a data control opens as its own page, an asset its full preview, a Block with data that data in
 * the data view (the same editor as its tab there); pure UI objects have none. On a phone the full-screen editor of the
 * data comes later (the phone package); here it is the data view. */
export function expandOf(entity: EntityEnvelope, source: EntityEnvelope | undefined, openEntity: (id: string) => void): (() => void) | null {
  if (entity.type_id === 'buckyos.container') return isDataControl(entity.kind) ? () => openControlPage(entity.entity_id) : null
  if (entity.view_type === 'asset' && source) return () => requestIntent(entity.entity_id, 'preview')
  if (source && !source.deleted) return () => openEntity(source.entity_id)
  return null
}

// ---- actions a control's body publishes (a data control's view menu, PRD §10.1 last rule)

export interface PublishedAction {
  id: string
  label: string
  icon?: LucideIcon
  checked?: boolean
  disabled?: string | false
  primary?: boolean
  run: () => void
}

const published = new Map<string, { actions: PublishedAction[]; key: string }>()
const emitter = new Emitter()
let version = 0

/** Listeners hear of a change of what is offered; a new render with the same offer only refreshes the handlers. */
function publish(containerId: string, actions: PublishedAction[] | null) {
  if (!actions || actions.length === 0) { if (!published.delete(containerId)) return }
  else {
    const key = JSON.stringify(actions.map((a) => [a.id, a.label, a.checked ?? false, a.disabled ?? false, a.primary ?? false]))
    const same = published.get(containerId)?.key === key
    published.set(containerId, { actions, key })
    if (same) return
  }
  version += 1
  emitter.emit()
}

/** A control's menu items as published actions: "open as a page" is the Block menu's own "expand", so it is left out. */
export function publishedOf(items: readonly ({ id: string; label: string; checked?: boolean; disabled?: string | false; run: () => void } | 'separator')[]): PublishedAction[] {
  return items.flatMap((item) => (item === 'separator' || item.id === 'page' ? [] : [{ id: item.id, label: item.label, checked: item.checked, disabled: item.disabled, run: item.run }]))
}

/** A control's body publishes its actions while it is mounted (they change with its state). */
export function usePublishActions(containerId: string, actions: PublishedAction[]) {
  useEffect(() => { publish(containerId, actions) })
  useEffect(() => () => publish(containerId, null), [containerId])
}

/** The actions the control `containerId` publishes now (re-rendering when they change). */
export function usePublishedActions(containerId: string | null): PublishedAction[] {
  useSyncExternalStore(emitter.subscribe, () => version)
  return containerId ? published.get(containerId)?.actions ?? [] : []
}
