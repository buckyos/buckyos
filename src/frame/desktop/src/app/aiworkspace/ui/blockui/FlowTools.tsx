/* The tools of a Block on a flow page (UI 框架 PRD §6.2, §10.4): in its title row — the type's icon, at most two primary
 * actions, its quick properties and ⋯ (the Block's whole menu, §10.3, the same list as the canvas's context menu). A
 * titled type carries them on its heading line, shown on hover or focus (always on a touch screen); an untitled one
 * (text, pictures, notes, shapes, frames) gets a thin row only while it is focused, selected or hovered. A locked Block
 * offers reading only.
 *
 * A control on a page (a mind map, a to-do, a calendar, an extension's composite) has the same row: its container is the
 * Block, its actions and properties are declared as a Block type's (`controlBlockDefinition`), an extension's from its
 * manifest, and what a data control's body publishes with `primary` joins them — as on a free canvas's Block toolbar. */

import type { ReactNode } from 'react'
import { Square, type LucideIcon } from 'lucide-react'
import { describeError } from '../../api/session'
import type { Json } from '../../api/types'
import { aiwsT } from '../../i18n'
import { useEntity, useStore, useWorkspaceUi } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import type { BlockDefinition, CanvasMode, ToolbarItem } from '../blocks/registry'
import { useBlockContext } from '../blocks/useBlockContext'
import { ENTRY_ICONS } from '../canvas/catalog'
import { ToolRow } from '../canvas/tools'
import { controlBlockDefinition, controlRegistry } from '../controls/registry'
import { actionOffered, configDefaults, declaredActions, declaredPrimary, runAction, type ActionHost } from '../extensions/actions'
import { usePublishedActions } from './commands'
import { toolOf, typeEntries } from './entries'
import { iconByName } from './icons'
import { quickItems } from './properties'
import './blockui.css'

/** How many primary actions a title row shows. */
const ROW_PRIMARY = 2

/** The title row's tools of one Block or control: its primary actions (two at most) and quick properties, and the icon and
 * name of its type. */
function useRowTools(entityId: string, mode: CanvasMode, editorActive: boolean, selected: boolean): { items: ToolbarItem[]; icon: LucideIcon; typeTitle: string } {
  const store = useStore()
  const ui = useWorkspaceUi()
  const entity = useEntity(entityId)
  const { cell, context, resolution } = useBlockContext({ cellId: entityId, mode, view: 'canvas', selected, editorActive })
  // a control's container is the Block: it declares its actions and properties as a Block type does
  const container = entity?.type_id === 'buckyos.container'
  const published = usePublishedActions(container ? entityId : null)
  const control = container ? controlRegistry.get(entity.kind) : undefined
  const definition: BlockDefinition | undefined = resolution?.ok ? context?.definition : container ? controlBlockDefinition(entity.kind) : undefined
  const icon = definition?.icon ?? ENTRY_ICONS[entity?.view_type ?? ''] ?? control?.icon ?? Square
  const typeTitle = definition?.title ?? control?.title ?? ''
  if (!context || !cell || cell.locked || !entity) return { items: [], icon, typeTitle }
  // an extension's declared actions come first (manifest 4, 控件机制 TODO H2): they are what this control is for, ahead of
  // its host type's own (an HTML control's "run" and "open definition", which stay in ⋯); an HTML control's from its
  // definition, a composite's from the definition its container names; primary unless they say otherwise
  const primary: ToolbarItem[] = []
  const composite = entity.kind === 'composite'
  const def = composite ? store.outline.get(entity.def_id ?? '') : undefined
  const declared = declaredActions(composite ? def?.def_actions : context.documentDefinition?.actions)
  if (declared.length > 0) {
    const host: ActionHost = composite
      ? { instanceId: entityId, mode, title: entity.title ?? def?.title ?? entityId, defaults: (def?.config_defaults ?? {}) as Record<string, Json>, openEntity: ui.openEntity }
      : { instanceId: entityId, mode, title: context.documentDefinition?.title ?? cell.title ?? entityId, defaults: configDefaults(context.documentDefinition?.config_schema), openEntity: ui.openEntity, activate: context.activateEditor }
    for (const a of declared) {
      if (!declaredPrimary(a) || !actionOffered(a, mode, entity.capabilities)) continue
      primary.push({ kind: 'button', id: `ext-action-${a.id}`, icon: iconByName(a.icon), label: a.title,
        run: () => { void runAction(store, host, a).catch((error: unknown) => store.notify('error', aiwsT('aiworkspace.canvas.extensionActionFailed', { reason: describeError(error) }))) } })
    }
  }
  // what a data control's body offers as primary (the Block toolbar shows the same), then the type's declared ones
  for (const a of published) if (a.primary) primary.push({ kind: 'button', id: `control-${a.id}`, icon: a.icon, label: a.label, active: a.checked, disabled: a.disabled ?? false, run: a.run })
  if (definition) for (const entry of typeEntries(definition, context, store)) if (entry.primary) primary.push(toolOf(entry))
  return { items: [...primary.slice(0, ROW_PRIMARY), ...quickItems(definition, context, store)], icon, typeTitle }
}

export function FlowRowTools({ cellId, mode, editorActive, selected, untitled, onMore, title }: {
  /** The Block's entity: a Block (`buckyos.cell`) or a control's container. */
  cellId: string
  mode: CanvasMode
  editorActive: boolean
  selected: boolean
  /** The thin row of an untitled type; otherwise the tools follow `title` on the heading line. */
  untitled: boolean
  onMore: (anchor: HTMLElement) => void
  title?: ReactNode
}) {
  const { t } = useAiwsText()
  const { items, icon: Icon, typeTitle } = useRowTools(cellId, mode, editorActive, selected)
  const label = t('aiworkspace.blockui.rowTools', { type: typeTitle })
  const row = <ToolRow items={items} label={label} onMore={onMore} moreTestId={`aiws-flow-more-${cellId}`} />
  if (untitled) {
    return (
      <div className="aiws-blockui-thinrow" data-testid={`aiws-flow-row-${cellId}`} onPointerDown={(event) => event.stopPropagation()}>
        <span className="aiws-blockui-typeicon" title={typeTitle}><Icon size={14} aria-hidden="true" /></span>
        {row}
      </div>
    )
  }
  return (
    <div className="aiws-blockui-titleline" data-testid={`aiws-flow-row-${cellId}`}>
      <span className="aiws-blockui-typeicon" title={typeTitle}><Icon size={15} aria-hidden="true" /></span>
      {title}
      {row}
    </div>
  )
}

/** The heading line of a control where its Blocks take no tools (presentation editing): its type's icon and its title. */
export function FlowTitleOnly({ entityId, title }: { entityId: string; title: ReactNode }) {
  const entity = useEntity(entityId)
  const control = controlRegistry.get(entity?.kind)
  const Icon = control?.icon ?? Square
  return (
    <div className="aiws-blockui-titleline" data-testid={`aiws-flow-row-${entityId}`}>
      <span className="aiws-blockui-typeicon" title={control?.title}><Icon size={15} aria-hidden="true" /></span>
      {title}
    </div>
  )
}
