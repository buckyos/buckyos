/* The tools of a Block on a flow page (UI 框架 PRD §6.2, §10.4): in its title row — the type's icon, at most two primary
 * actions, its quick properties and ⋯ (the Block's whole menu, §10.3, the same list as the canvas's context menu). A
 * titled type carries them on its heading line, shown on hover or focus; an untitled one (text, pictures, notes, shapes,
 * frames) gets a thin row only while it is focused, selected or hovered. A locked Block offers reading only. */

import type { ReactNode } from 'react'
import { Square, type LucideIcon } from 'lucide-react'
import { describeError } from '../../api/session'
import { aiwsT } from '../../i18n'
import { useStore, useWorkspaceUi } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import type { BlockDefinition, CanvasMode, ToolbarItem } from '../blocks/registry'
import { useBlockContext } from '../blocks/useBlockContext'
import { ENTRY_ICONS } from '../canvas/catalog'
import { ToolRow } from '../canvas/tools'
import { actionOffered, configDefaults, declaredActions, runAction, type ActionHost } from '../extensions/actions'
import { toolOf, typeEntries } from './entries'
import { iconByName } from './icons'
import { quickItems } from './properties'
import './blockui.css'

/** How many primary actions a title row shows. */
const ROW_PRIMARY = 2

/** The title row's tools of one Block: its primary actions (two at most) and quick properties. */
function useRowTools(cellId: string, mode: CanvasMode, editorActive: boolean, selected: boolean): { items: ToolbarItem[]; definition: BlockDefinition | undefined } {
  const store = useStore()
  const ui = useWorkspaceUi()
  const { cell, context, resolution } = useBlockContext({ cellId, mode, view: 'canvas', selected, editorActive })
  const definition = resolution?.ok ? context?.definition : undefined
  if (!context || !cell || cell.locked) return { items: [], definition }
  const primary: ToolbarItem[] = []
  if (definition) for (const entry of typeEntries(definition, context, store)) if (entry.primary) primary.push(toolOf(entry))
  // an extension's declared actions are primary (manifest 4)
  const declared = declaredActions(context.documentDefinition?.actions)
  if (declared.length > 0) {
    const host: ActionHost = { instanceId: cellId, mode, title: context.documentDefinition?.title ?? cell.title ?? cellId, defaults: configDefaults(context.documentDefinition?.config_schema), openEntity: ui.openEntity, activate: context.activateEditor }
    for (const a of declared) {
      if (!actionOffered(a, mode, cell.capabilities)) continue
      primary.push({ kind: 'button', id: `ext-action-${a.id}`, icon: iconByName(a.icon), label: a.title,
        run: () => { void runAction(store, host, a).catch((error: unknown) => store.notify('error', aiwsT('aiworkspace.canvas.extensionActionFailed', { reason: describeError(error) }))) } })
    }
  }
  return { items: [...primary.slice(0, ROW_PRIMARY), ...quickItems(definition, context, store)], definition }
}

export function FlowRowTools({ cellId, mode, editorActive, selected, untitled, onMore, title }: {
  cellId: string
  mode: CanvasMode
  editorActive: boolean
  selected: boolean
  /** The thin row of an untitled type; otherwise the tools follow `title` on the heading line. */
  untitled: boolean
  onMore: (anchor: HTMLElement) => void
  title?: ReactNode
}) {
  const store = useStore()
  const { t } = useAiwsText()
  const { items, definition } = useRowTools(cellId, mode, editorActive, selected)
  const type = store.outline.get(cellId)?.view_type
  const Icon: LucideIcon = definition?.icon ?? (type ? ENTRY_ICONS[type] : undefined) ?? Square
  const label = t('aiworkspace.blockui.rowTools', { type: definition?.title ?? '' })
  const row = <ToolRow items={items} label={label} onMore={onMore} moreTestId={`aiws-flow-more-${cellId}`} />
  if (untitled) {
    return (
      <div className="aiws-blockui-thinrow" data-testid={`aiws-flow-row-${cellId}`} onPointerDown={(event) => event.stopPropagation()}>
        <span className="aiws-blockui-typeicon" title={definition?.title}><Icon size={14} aria-hidden="true" /></span>
        {row}
      </div>
    )
  }
  return (
    <div className="aiws-blockui-titleline" data-testid={`aiws-flow-row-${cellId}`}>
      <span className="aiws-blockui-typeicon" title={definition?.title}><Icon size={15} aria-hidden="true" /></span>
      {title}
      {row}
    </div>
  )
}
