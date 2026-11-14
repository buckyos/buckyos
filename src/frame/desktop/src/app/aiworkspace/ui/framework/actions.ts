/* Actions (UI 框架 PRD §3.3): the one unit of function. An action has a stable id, a name and an icon, a display
 * condition (when it is not shown), an availability (why it is disabled), optional candidates (several targets:
 * presentation paths, macros) and what it runs. Where it appears is declared, not coded into the shell: a main menu
 * group (§4.1.2) and a tool zone (§4.2–§4.5) with an order. The main menu, the tool zones and (later) the phone's
 * main menu read the same registry, so one action has one name, one icon and one reason everywhere. */

import { testHooks } from '../../api/testHooks'
import { aiwsT } from '../../i18n'
import type { Json } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import type { CanvasMode } from '../blocks/registry'
import type { CanvasCommands } from '../shell/MainMenu'
import type { MenuItem } from '../shell/popover'
import type { ShellApi } from '../shell/shellContext'
import type { ResolvedToolbar } from './config'
import type { IconRef } from './icons'

/** The first level of the main menu (§4.1.2): groups only, at most nine rows. `account` (a guest's sign-in) and `mode`
 * (data mode / back to canvas) are rows of their own, without a second level. */
export type MenuGroup = 'account' | 'workspace' | 'surface' | 'insert' | 'edit' | 'view' | 'collab' | 'mode' | 'help'
/** The phone's main menu (§6.1) has two groups of its own: "导入文档" and "手写" (a row without a second level). */
export type PhoneMenuGroup = MenuGroup | 'import-doc' | 'ink'

/** The five tool zones (§3.2): title (top left), collaboration (top right), tool selection (left), context (bottom
 * centre), status (bottom right). */
export type ZoneId = 'title' | 'collab' | 'tools' | 'context' | 'status'

/** What an action sees when it is asked whether it shows, why it is disabled, what its candidates are, and runs. */
export interface ActionContext {
  store: WorkspaceStore
  shell: ShellApi
  /** The open canvas's commands; null in data mode and without a canvas. */
  canvas: CanvasCommands | null
  mode: CanvasMode
  /** Show an entity: a Block on its canvas, data in data mode. */
  openEntity: (entityId: string) => void
  /** The tool zones as the configuration layers resolve them here (§4.6); absent where nothing resolved them. */
  layout?: ResolvedToolbar
}

/** One target of an action with several (a presentation path, a guide, a macro), or one entry of its submenu. */
export interface ActionCandidate {
  id: string
  label?: string
  testId?: string
  hint?: string
  checked?: boolean
  /** Why it cannot run now (shown disabled), or null. */
  reason?: string | null
  explain?: boolean
  danger?: boolean
  /** A line between entries. */
  separator?: boolean
  /** Offered only in the drop-down, never counted: with one real candidate the tool item runs it directly. */
  extra?: boolean
  run?: () => void
}

export interface MenuPlacement {
  group: MenuGroup
  order: number
  /** The menu entry's id; its test id is `aiws-menu-<id>` unless `testId` says otherwise. */
  id: string
  testId?: string
  hint?: (ctx: ActionContext) => string | undefined
  /** The candidates are the entry's second (or third) level. */
  submenu?: boolean
  /** A separator before the entry. */
  separator?: boolean
  danger?: boolean
}

export interface ZonePlacement {
  zone: ZoneId
  order: number
  testId?: string
  /** A built-in control the zone draws itself (zoom, avatars, share) instead of a button. */
  control?: string
  /** Moves into the zone's "more" menu in a narrow window (§4.2). */
  overflow?: 'narrow'
}

export interface ActionDef {
  /** Stable and namespaced (§3.3): `ws.*`, `surface.*`, `view.*`, `collab.*`, `block.*`, `data.*`, extensions, macros. */
  id: string
  label: (ctx: ActionContext) => string
  /** The tool item's tooltip; the label when absent. */
  tip?: (ctx: ActionContext) => string
  icon?: IconRef
  menu?: MenuPlacement | null
  zone?: ZonePlacement | null
  /** Present on the phone's main menu (§6.1). */
  phone?: boolean
  /** Where the phone's main menu puts it, when that is not where `menu` does. */
  phoneMenu?: Partial<Omit<MenuPlacement, 'group'>> & { group?: PhoneMenuGroup }
  /** Display condition: not shown when false. */
  visible?: (ctx: ActionContext) => boolean
  /** Availability: disabled with this reason, or null. */
  reason?: (ctx: ActionContext) => string | null
  /** The reason is printed in the menu (structural limits), not only in the tooltip. */
  explain?: boolean
  checked?: (ctx: ActionContext) => boolean
  pressed?: (ctx: ActionContext) => boolean
  /** `args`: a configured tool item's own arguments (a macro bound to a button, §4.6). */
  candidates?: (ctx: ActionContext, args?: Json) => ActionCandidate[]
  run?: (ctx: ActionContext, args?: Json) => void
}

export class ActionRegistry {
  private readonly defs = new Map<string, ActionDef>()
  private readonly listeners = new Set<() => void>()
  private version = 0

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }
  snapshot = () => this.version

  private changed() {
    this.version += 1
    for (const listener of [...this.listeners]) listener()
  }

  /** Registers (or replaces) an action; returns its removal. */
  register(def: ActionDef): () => void {
    this.defs.set(def.id, def)
    this.changed()
    return () => { if (this.defs.get(def.id) === def) { this.defs.delete(def.id); this.changed() } }
  }

  get(id: string): ActionDef | undefined { return this.defs.get(id) }

  list(): ActionDef[] { return [...this.defs.values()] }

  inMenu(group: MenuGroup): ActionDef[] {
    return this.list().filter((def) => def.menu?.group === group).sort((a, b) => (a.menu?.order ?? 0) - (b.menu?.order ?? 0))
  }

  inZone(zone: ZoneId): ActionDef[] {
    return this.list().filter((def) => def.zone?.zone === zone).sort((a, b) => (a.zone?.order ?? 0) - (b.zone?.order ?? 0))
  }
}

export const actionRegistry = new ActionRegistry()

// an e2e test plays an extension registering its own action (§15 acceptance 1)
const hooks = testHooks()
if (hooks) Object.assign(hooks, { actions: actionRegistry })

// ---- the main menu (§4.1.2)

interface GroupDef { id: MenuGroup; label: () => string; inline?: boolean; reason?: (ctx: ActionContext) => string | null; explain?: boolean }

export const MENU_GROUPS: GroupDef[] = [
  { id: 'account', label: () => '', inline: true },
  { id: 'workspace', label: () => aiwsT('aiworkspace.framework.group.workspace') },
  { id: 'surface', label: () => aiwsT('aiworkspace.framework.group.surface') },
  {
    id: 'insert', label: () => aiwsT('aiworkspace.shell.menu.insert'), explain: true,
    reason: ({ canvas }) => {
      const notOnCanvas = aiwsT('aiworkspace.shell.notOnCanvas')
      const insertReason = canvas ? canvas.insertReason : notOnCanvas
      const noteReason = canvas ? canvas.noteReason : notOnCanvas
      return insertReason !== null && noteReason !== null ? insertReason : null
    },
  },
  { id: 'edit', label: () => aiwsT('aiworkspace.common.edit') },
  { id: 'view', label: () => aiwsT('aiworkspace.shell.menu.view') },
  { id: 'collab', label: () => aiwsT('aiworkspace.framework.group.collab') },
  { id: 'mode', label: () => '', inline: true },
  { id: 'help', label: () => aiwsT('aiworkspace.shell.menu.help') },
]

export function candidateItem(candidate: ActionCandidate): MenuItem {
  if (candidate.separator) return { id: candidate.id, separator: true }
  return {
    id: candidate.id, testId: candidate.testId, label: candidate.label, hint: candidate.hint, checked: candidate.checked, danger: candidate.danger,
    disabled: Boolean(candidate.reason), reason: candidate.reason ?? null, explain: candidate.explain, run: candidate.run,
  }
}

/** Separators only between entries: none first, last or twice. */
function tidy(items: MenuItem[]): MenuItem[] {
  const out: MenuItem[] = []
  for (const item of items) {
    if (item.separator && (out.length === 0 || out[out.length - 1].separator)) continue
    out.push(item)
  }
  while (out.length > 0 && out[out.length - 1].separator) out.pop()
  return out
}

export function shown(def: ActionDef, ctx: ActionContext, phone = false): boolean {
  if (phone && !def.phone) return false
  return def.visible ? def.visible(ctx) : true
}

/** The menu entry of one action, or null when it is not shown. */
export function menuItemOf(def: ActionDef, ctx: ActionContext, placement: MenuPlacement | null | undefined = def.menu): MenuItem | null {
  if (!placement) return null
  const reason = def.reason?.(ctx) ?? null
  const base = { id: placement.id, testId: placement.testId, label: def.label(ctx), hint: placement.hint?.(ctx), explain: def.explain, danger: placement.danger }
  if (placement.submenu) {
    const items = tidy((def.candidates?.(ctx) ?? []).map(candidateItem))
    return { ...base, items, disabled: reason !== null || items.length === 0, reason }
  }
  return { ...base, checked: def.checked?.(ctx), disabled: reason !== null, reason, run: () => def.run?.(ctx) }
}

/** The whole main menu from the registry: the first level holds groups (and the two inline rows), each group its
 * actions in order. `phone`: only what the phone's main menu offers (§6.1). */
export function buildMenu(ctx: ActionContext, options: { phone?: boolean } = {}): MenuItem[] {
  if (options.phone) return buildPhoneMenu(ctx)
  const out: MenuItem[] = []
  for (const group of MENU_GROUPS) {
    const items: MenuItem[] = []
    for (const def of actionRegistry.inMenu(group.id)) {
      if (!shown(def, ctx, options.phone)) continue
      const item = menuItemOf(def, ctx)
      if (!item) continue
      if (def.menu?.separator) items.push({ id: `sep-${item.id}`, separator: true })
      items.push(item)
    }
    const entries = tidy(items)
    if (entries.length === 0) continue
    if (group.inline) { out.push(...entries.filter((item) => !item.separator)); continue }
    const reason = group.reason?.(ctx) ?? null
    out.push({ id: group.id, label: group.label(), items: entries, disabled: reason !== null, reason, explain: group.explain })
  }
  return out
}

// ---- the phone's main menu (§6.1): only what a phone can do, entered one level at a time

const PHONE_GROUPS: { id: PhoneMenuGroup; label: () => string; inline?: boolean }[] = [
  { id: 'account', label: () => '', inline: true },
  { id: 'workspace', label: () => aiwsT('aiworkspace.framework.group.workspace') },
  { id: 'surface', label: () => aiwsT('aiworkspace.framework.group.surface') },
  { id: 'import-doc', label: () => aiwsT('aiworkspace.phone.group.import') },
  { id: 'ink', label: () => '', inline: true },
  { id: 'collab', label: () => aiwsT('aiworkspace.framework.group.collab') },
  { id: 'edit', label: () => aiwsT('aiworkspace.common.edit') },
  { id: 'view', label: () => aiwsT('aiworkspace.shell.menu.view') },
  { id: 'mode', label: () => '', inline: true },
]

function phonePlacement(def: ActionDef): (MenuPlacement & { group: PhoneMenuGroup }) | null {
  if (!def.menu && !def.phoneMenu) return null
  return { group: 'workspace', order: 0, id: def.id, ...def.menu, ...def.phoneMenu } as MenuPlacement & { group: PhoneMenuGroup }
}

/** Declared `phone` actions only (what is desktop-only is not there at all, §6.1), in the phone's groups; a group of one
 * submenu is that submenu ("导入文档" is the file imports). */
function buildPhoneMenu(ctx: ActionContext): MenuItem[] {
  const placed = actionRegistry.list().flatMap((def) => { const at = shown(def, ctx, true) ? phonePlacement(def) : null; return at ? [{ def, at }] : [] })
  const out: MenuItem[] = []
  for (const group of PHONE_GROUPS) {
    const items: MenuItem[] = []
    for (const { def, at } of placed.filter((p) => p.at.group === group.id).sort((a, b) => a.at.order - b.at.order)) {
      const item = menuItemOf(def, ctx, at)
      if (!item) continue
      if (at.separator) items.push({ id: `sep-${item.id}`, separator: true })
      items.push(item)
    }
    const entries = tidy(items)
    if (entries.length === 0) continue
    if (group.inline) { out.push(...entries.filter((item) => !item.separator)); continue }
    const only = entries.length === 1 && entries[0].items ? entries[0] : null
    out.push(only
      ? { id: group.id, label: group.label(), items: only.items, disabled: only.disabled, reason: only.reason, explain: true }
      : { id: group.id, label: group.label(), items: entries })
  }
  return out
}

// ---- a tool item (§3.3): a button, a toggle, a drop-down

export interface ToolItemModel {
  def: ActionDef
  label: string
  tip: string
  icon: IconRef | undefined
  reason: string | null
  pressed: boolean | undefined
  /** Several candidates: a drop-down of these. */
  menu: ActionCandidate[] | null
  /** Otherwise what a click runs. */
  run: (() => void) | null
}

/** How an action shows as a tool item: one candidate runs directly, several drop down, none falls back to the action's
 * own run (or hides it). Null when it is not shown. */
export function toolItemOf(def: ActionDef, ctx: ActionContext, args?: Json, override?: { title?: string; icon?: string }): ToolItemModel | null {
  if (def.visible && !def.visible(ctx)) return null
  const label = override?.title ?? def.label(ctx)
  const base = { def, label, tip: override?.title ?? def.tip?.(ctx) ?? label, icon: override?.icon ?? def.icon, reason: def.reason?.(ctx) ?? null, pressed: def.pressed?.(ctx) }
  const candidates = def.candidates ? def.candidates(ctx, args) : null
  if (candidates) {
    const real = candidates.filter((c) => !c.separator && !c.extra)
    if (real.length > 1) return { ...base, menu: candidates, run: null }
    if (real.length === 1) {
      const only = real[0]
      return { ...base, tip: override?.title ?? only.label ?? base.tip, reason: base.reason ?? only.reason ?? null, menu: null, run: only.run ?? null }
    }
  }
  if (!def.run) return null
  return { ...base, menu: null, run: () => def.run?.(ctx, args) }
}
