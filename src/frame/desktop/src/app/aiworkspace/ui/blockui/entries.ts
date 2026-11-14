/* The Block's actions as one list (UI 框架 PRD §10.3–§10.5): every entry of the context menu, the toolbar's "more" and the
 * flow title row's ⋯ comes from here, so an action has one name, icon and reason wherever it shows. An entry belongs to
 * one group of the menu skeleton — the type's actions, open, collaborate, clipboard, arrange, relations, delete — and
 * may also be a toolbar button: `primary` (the type's primary actions and a few common ones such as group) or `common`
 * (the common zone: comment, lock, AI). The host (CanvasView) builds the common entries with its own commands; this
 * module turns a type's declared actions into entries and the list into menu items and toolbar items. */

import type { LucideIcon } from 'lucide-react'
import { describeError } from '../../api/session'
import { aiwsT } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'
import { actionStableId, type BlockAction, type BlockDefinition, type RenderContext, type ToolbarItem } from '../blocks/registry'
import type { MenuItem } from '../shell/popover'

export type EntryGroup = 'type' | 'open' | 'collab' | 'clipboard' | 'arrange' | 'relate' | 'delete'
export const GROUP_ORDER: readonly EntryGroup[] = ['type', 'open', 'collab', 'clipboard', 'arrange', 'relate', 'delete']
/** Above this many items (desktop), "arrange" and "relations" fold into a submenu each (§10.3). */
export const FOLD_ABOVE = 12

export interface BlockEntry {
  /** The local id: the test ids `aiws-menu-<id>` (context menu) and `aiws-near-<id>` (toolbar, "more"). */
  id: string
  /** The stable action id (`block.*`, `block.<type>.*`, an extension's `ext.<def>.<id>`). */
  actionId: string
  group: EntryGroup
  label: string
  icon?: LucideIcon
  key?: string
  /** A toolbar button (the type's primary actions; group / ungroup). */
  primary?: boolean
  /** A button of the toolbar's common zone (comment, lock, AI). */
  common?: boolean
  ai?: boolean
  active?: boolean
  danger?: boolean
  /** Why it is offered disabled (missing capability), or false. */
  disabled?: string | false
  /** The menu item's test id, when it keeps an older one (the flow page's ⋮⋮ menu). */
  testId?: string
  run: () => void
}

/** Is the declared action offered here and now (mode, view, capabilities, `when`)? Its reason when it is offered disabled. */
function offered(action: BlockAction, context: RenderContext, store: WorkspaceStore): { show: boolean; reason: string | false } {
  if (!action.modes.includes(context.mode) || (action.views && !action.views.includes(context.view))) return { show: false, reason: false }
  const needs = action.needs ?? []
  const target = action.onSource ? context.source : context.cell
  if (needs.some((cap) => !(target?.capabilities ?? []).includes(cap))) return { show: false, reason: false }
  const writes = needs.some((cap) => ['update', 'append', 'structure', 'delete', 'manage'].includes(cap))
  if (writes && context.readOnlyReason) return { show: false, reason: false }
  if (action.when && !action.when(context, store)) return { show: false, reason: false }
  return { show: true, reason: action.disabled?.(context, store) ?? false }
}

/** The type's declared actions as entries: the one bound to Enter is its "edit content" (the open group), the others the
 * type group, primary ones first. A failing `when` or `disabled` offers the action disabled with the error. */
export function typeEntries(definition: BlockDefinition, context: RenderContext, store: WorkspaceStore): BlockEntry[] {
  const out: BlockEntry[] = []
  for (const action of definition.actions ?? []) {
    let state: { show: boolean; reason: string | false }
    try { state = offered(action, context, store) } catch (error) { state = { show: true, reason: describeError(error) } }
    if (!state.show) continue
    const run = () => {
      void Promise.resolve().then(() => action.run(context, store)).catch((error: unknown) => store.notify('error', aiwsT('aiworkspace.canvas.extensionActionFailed', { reason: describeError(error) })))
    }
    let active = false
    try { active = Boolean(action.active?.(context)) } catch { /* drawn unpressed */ }
    out.push({
      id: action.id, actionId: actionStableId(definition.type, action.id), group: action.key === 'Enter' ? 'open' : 'type', label: action.label, icon: action.icon,
      key: action.key, primary: action.primary, ai: action.ai, active, disabled: state.reason, run: state.reason ? () => undefined : run,
    })
  }
  return [...out.filter((e) => e.primary), ...out.filter((e) => !e.primary)]
}

/** The entries in menu order: the groups of the skeleton, primary first inside the type group. */
export function ordered(entries: readonly BlockEntry[]): BlockEntry[] {
  return GROUP_ORDER.flatMap((group) => {
    const inGroup = entries.filter((e) => e.group === group)
    return group === 'type' ? [...inGroup.filter((e) => e.primary), ...inGroup.filter((e) => !e.primary)] : inGroup
  })
}

const GROUP_LABEL: Partial<Record<EntryGroup, string>> = { arrange: 'aiworkspace.blockui.group.arrange', relate: 'aiworkspace.blockui.group.relate' }

/** One entry as a menu item (`prefix`: `menu` in the context menu, `near` in the toolbar's "more"). */
export function menuItemOf(entry: BlockEntry, prefix: 'menu' | 'near'): MenuItem {
  return {
    id: entry.id, testId: entry.testId ?? `aiws-${prefix}-${entry.id}`, label: entry.label, hint: entry.key, checked: entry.active || undefined, danger: entry.danger,
    disabled: Boolean(entry.disabled), reason: entry.disabled || null, explain: Boolean(entry.disabled), run: entry.run,
  }
}

/** The whole list as menu items: groups apart; with more than `FOLD_ABOVE` items (and `fold`), "arrange" and "relations"
 * each become a submenu. */
export function menuItemsOf(entries: readonly BlockEntry[], prefix: 'menu' | 'near', fold = true): MenuItem[] {
  const list = ordered(entries)
  const folding = fold && list.length > FOLD_ABOVE
  const out: MenuItem[] = []
  for (const group of GROUP_ORDER) {
    const inGroup = list.filter((e) => e.group === group)
    if (inGroup.length === 0) continue
    if (out.length > 0) out.push({ id: `sep-${group}`, separator: true })
    const label = GROUP_LABEL[group]
    if (folding && label && inGroup.length > 1) out.push({ id: `group-${group}`, testId: `aiws-${prefix}-group-${group}`, label: aiwsT(label), items: inGroup.map((e) => menuItemOf(e, prefix)) })
    else out.push(...inGroup.map((e) => menuItemOf(e, prefix)))
  }
  return out
}

/** An entry as a toolbar button (an icon; the label when it has none). */
export function toolOf(entry: BlockEntry): ToolbarItem {
  return { kind: 'button', id: entry.id, icon: entry.icon, label: entry.label, key: entry.key, active: entry.active || undefined, ai: entry.ai, disabled: entry.disabled ?? false, run: entry.run }
}
