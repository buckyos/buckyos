/* Toolbar configuration (UI 框架 PRD §4.6): four layers, the later over the earlier — built-in defaults, the workspace,
 * the canvas, the person. The shared layers (workspace, canvas) decide what there is: add items (an action or a macro,
 * with a name and an icon of their own), hide items, reorder, hide a whole zone, lock a zone or an item. The personal
 * layer only arranges what the built-in and shared layers offer — show or hide, order — and never touches what is
 * locked. Items merge by their stable key, so the result is determined by the layers alone.
 *
 * Storage (K4): the Workspace layer is the root's `config.toolbar` (state/settings.ts `useWorkspaceSettings`), the canvas
 * layer the Surface's `config.toolbar` (`useSurfaceToolbar`), the personal layer the cross-workspace preference `zones`
 * (ui/framework/personal.ts). Hiding every zone of a canvas is App mode (§4.6): the title zone hidden leaves whoever
 * may change the layout the collapsed ⋯ of `AppModeEntry`. */

import type { Json } from '../../api/types'
import type { CanvasMode } from '../blocks/registry'
import { actionRegistry, type ActionContext, type ZoneId } from './actions'

export const ZONES: ZoneId[] = ['title', 'collab', 'tools', 'context', 'status']

/** One item a layer declares. `id` is its stable key; without one the action (and its arguments) is the key. */
export interface ToolItemConfig {
  id?: string
  action: string
  args?: Json
  title?: string
  icon?: string
  hidden?: boolean
  locked?: boolean
  /** Shown only in these modes, on these devices, to holders of this capability. */
  when?: { modes?: CanvasMode[]; devices?: ('phone' | 'desktop')[]; capability?: string }
}

/** How a canvas relates to what comes before it (kernel 06): inherit (only flags of existing items), append (new items
 * at the end, existing ones updated in place), override (this list replaces the items), hide (the zone is hidden). */
export type ZoneRelation = 'inherit' | 'append' | 'override' | 'hide'

export interface ZoneConfig {
  relation?: ZoneRelation
  items?: ToolItemConfig[]
  hidden?: boolean
  locked?: boolean
  /** Personal layer: the keys in the order the person wants them. */
  order?: string[]
}

export interface ToolbarConfig { zones?: Partial<Record<ZoneId, ZoneConfig>> }

export type ConfigLayerName = 'builtin' | 'workspace' | 'surface' | 'personal'
export interface ConfigLayer { layer: ConfigLayerName; config: ToolbarConfig | null | undefined }

export interface ResolvedItem { key: string; action: string; args?: Json; title?: string; icon?: string; locked: boolean; when?: ToolItemConfig['when']; /** Only with `keepHidden`. */ hidden?: boolean }
export interface ResolvedZone { hidden: boolean; locked: boolean; items: ResolvedItem[] }
export type ResolvedToolbar = Record<ZoneId, ResolvedZone>

/** The zones a shared layer may arrange in the customize dialog (§4.6): the title zone and the status bar as a whole,
 * the collaboration zone and the tool selection zone's object group item by item. The context zone only shows with a
 * situation the person started; nothing configures it. */
export const CUSTOMIZABLE_ZONES: ZoneId[] = ['title', 'collab', 'tools', 'status']

function stable(value: Json | undefined): string {
  if (value === null || typeof value !== 'object') return JSON.stringify(value ?? null)
  if (Array.isArray(value)) return `[${value.map((v) => stable(v)).join(',')}]`
  return `{${Object.keys(value).sort().map((k) => `${JSON.stringify(k)}:${stable(value[k])}`).join(',')}}`
}

export function itemKey(item: Pick<ToolItemConfig, 'id' | 'action' | 'args'>): string {
  if (item.id) return item.id
  return item.args === undefined ? item.action : `${item.action}#${stable(item.args)}`
}

/** The object group of the tool selection zone (§4.3): the built-in list of Block types, by catalog entry id. */
export const TOOL_OBJECTS = ['shape-text', 'richtext', 'asset', 'table', 'mindmap', 'shape', 'connector', 'frame', 'wish', 'note']
export const INSERT_ACTION = 'block.insert'

/** The built-in layer: the registered actions' zone declarations, and the tool selection zone's object list. */
export function builtinToolbar(): ToolbarConfig {
  const zones: Partial<Record<ZoneId, ZoneConfig>> = {}
  for (const zone of ZONES) {
    const items: ToolItemConfig[] = actionRegistry.inZone(zone).map((def) => ({ action: def.id }))
    if (zone === 'tools') items.push(...TOOL_OBJECTS.map((entry) => ({ id: `insert:${entry}`, action: INSERT_ACTION, args: { entry } })))
    zones[zone] = { relation: 'override', items }
  }
  return { zones }
}

/** The object group of a resolved tool selection zone, by catalog entry id. */
export function insertEntries(zone: ResolvedZone): string[] {
  return zone.items.flatMap((item) => {
    const args = item.args
    return item.action === INSERT_ACTION && args && typeof args === 'object' && !Array.isArray(args) && typeof args.entry === 'string' ? [args.entry] : []
  })
}

const RELATIONS: ZoneRelation[] = ['inherit', 'append', 'override', 'hide']

/** A layer read from a document (a canvas's `config.toolbar`): only the known shape passes, the rest is dropped. */
export function parseToolbarConfig(value: unknown): ToolbarConfig | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const zonesIn = (value as { zones?: unknown }).zones
  if (!zonesIn || typeof zonesIn !== 'object' || Array.isArray(zonesIn)) return null
  const zones: Partial<Record<ZoneId, ZoneConfig>> = {}
  for (const zone of ZONES) {
    const raw = (zonesIn as Record<string, unknown>)[zone]
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) continue
    const r = raw as Record<string, unknown>
    const items = Array.isArray(r.items) ? r.items.flatMap((entry): ToolItemConfig[] => {
      if (!entry || typeof entry !== 'object') return []
      const e = entry as Record<string, unknown>
      if (typeof e.action !== 'string' || !e.action) return []
      return [{
        ...(typeof e.id === 'string' ? { id: e.id } : {}), action: e.action, ...(e.args !== undefined ? { args: e.args as Json } : {}),
        ...(typeof e.title === 'string' ? { title: e.title } : {}), ...(typeof e.icon === 'string' ? { icon: e.icon } : {}),
        ...(typeof e.hidden === 'boolean' ? { hidden: e.hidden } : {}), ...(typeof e.locked === 'boolean' ? { locked: e.locked } : {}),
        ...(e.when && typeof e.when === 'object' ? { when: e.when as ToolItemConfig['when'] } : {}),
      }]
    }) : undefined
    const order = Array.isArray(r.order) ? r.order.filter((key): key is string => typeof key === 'string') : undefined
    zones[zone] = {
      ...(typeof r.relation === 'string' && RELATIONS.includes(r.relation as ZoneRelation) ? { relation: r.relation as ZoneRelation } : {}),
      ...(items ? { items } : {}),
      ...(typeof r.hidden === 'boolean' ? { hidden: r.hidden } : {}),
      ...(typeof r.locked === 'boolean' ? { locked: r.locked } : {}),
      ...(order ? { order } : {}),
    }
  }
  return { zones }
}

interface Working { hidden: boolean; locked: boolean; items: (ResolvedItem & { hidden: boolean })[] }

function resolve(item: ToolItemConfig): ResolvedItem & { hidden: boolean } {
  return { key: itemKey(item), action: item.action, args: item.args, title: item.title, icon: item.icon, locked: item.locked ?? false, when: item.when, hidden: item.hidden ?? false }
}

function update(target: ResolvedItem & { hidden: boolean }, item: ToolItemConfig) {
  if (item.title !== undefined) target.title = item.title
  if (item.icon !== undefined) target.icon = item.icon
  if (item.hidden !== undefined) target.hidden = item.hidden
  if (item.locked) target.locked = true
  if (item.when !== undefined) target.when = item.when
}

function shared(zone: Working, config: ZoneConfig) {
  const relation = config.relation ?? (config.items ? 'append' : 'inherit')
  if (relation === 'hide') zone.hidden = true
  if (relation === 'override') {
    const seen = new Set<string>()
    zone.items = (config.items ?? []).map(resolve).filter((item) => !seen.has(item.key) && Boolean(seen.add(item.key)))
  } else {
    for (const item of config.items ?? []) {
      const existing = zone.items.find((entry) => entry.key === itemKey(item))
      if (existing) update(existing, item)
      else if (relation === 'append') zone.items.push(resolve(item))
    }
  }
  if (config.hidden !== undefined) zone.hidden = config.hidden
  if (config.locked) zone.locked = true
}

function personal(zone: Working, config: ZoneConfig) {
  if (zone.locked) return
  if (config.hidden !== undefined) zone.hidden = config.hidden
  for (const item of config.items ?? []) {
    const existing = zone.items.find((entry) => entry.key === itemKey(item))
    if (existing && !existing.locked && item.hidden !== undefined) existing.hidden = item.hidden
  }
  if (config.order && !zone.items.some((item) => item.locked)) {
    const rank = new Map(config.order.map((key, i) => [key, i]))
    zone.items = zone.items.map((item, i) => ({ item, i })).sort((a, b) => (rank.get(a.item.key) ?? config.order!.length + a.i) - (rank.get(b.item.key) ?? config.order!.length + b.i)).map(({ item }) => item)
  }
}

/** Merge the layers (in the order given: built-in, workspace, canvas, personal). `keepHidden`: the hidden items stay in
 * the result, marked (the customize dialog lists them unticked). */
export function mergeToolbar(layers: ConfigLayer[], { keepHidden = false }: { keepHidden?: boolean } = {}): ResolvedToolbar {
  const work = Object.fromEntries(ZONES.map((zone): [ZoneId, Working] => [zone, { hidden: false, locked: false, items: [] }])) as Record<ZoneId, Working>
  for (const { layer, config } of layers) {
    for (const zone of ZONES) {
      const zc = config?.zones?.[zone]
      if (!zc) continue
      if (layer === 'personal') personal(work[zone], zc)
      else shared(work[zone], zc)
    }
  }
  return Object.fromEntries(ZONES.map((zone) => [zone, {
    hidden: work[zone].hidden, locked: work[zone].locked,
    items: work[zone].items.filter((item) => keepHidden || !item.hidden).map((item): ResolvedItem => ({
      key: item.key, action: item.action, args: item.args, title: item.title, icon: item.icon, locked: item.locked, when: item.when, ...(keepHidden ? { hidden: item.hidden } : {}),
    })),
  }])) as ResolvedToolbar
}

/** Whether an item's own display condition holds. */
export function itemApplies(item: ResolvedItem, ctx: ActionContext): boolean {
  const when = item.when
  if (!when) return true
  if (when.modes && !when.modes.includes(ctx.mode)) return false
  if (when.devices && !when.devices.includes(ctx.shell.phone ? 'phone' : 'desktop')) return false
  if (when.capability && !ctx.store.session.info().capabilities.includes(when.capability as never)) return false
  return true
}

/** A shared layer without what hides whole zones: "临时显示工具区" of App mode (§4.6) shows them in this window. */
export function withZonesShown(config: ToolbarConfig | null): ToolbarConfig | null {
  if (!config?.zones) return config
  const zones: Partial<Record<ZoneId, ZoneConfig>> = {}
  for (const [zone, zc] of Object.entries(config.zones) as [ZoneId, ZoneConfig][]) {
    const rest: ZoneConfig = { ...zc }
    delete rest.hidden
    zones[zone] = rest.relation === 'hide' ? { ...rest, relation: 'inherit' } : rest
  }
  return { zones }
}

/** App mode (§4.6): a shared layer hid the title zone (and with it the main menu), so the canvas's own controls are the
 * way in; only `structure` or `manage` get the collapsed ⋯ back to the zones. */
export function isAppMode(layout: ResolvedToolbar): boolean {
  return layout.title.hidden
}
