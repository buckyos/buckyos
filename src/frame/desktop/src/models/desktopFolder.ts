/**
 * The desktop as a folder (proposal: desktop-folder).
 *
 * Like Windows, what the desktop shows is the user's Desktop folder (`/home/{user}/Desktop`, also seen in
 * Files), so every device and session shows the same items; a session (this device, per form factor) only
 * keeps where they sit. Entries:
 *
 * - `*.lnk`     a shortcut: `{"app": "<app id>"}` — a built-in id, an AppId or an app instance id
 * - `*.widget`  a widget: `{"widget": "clock"}`, `{"widget": "notepad", "text": "..."}`
 * - anything else is a document or a folder, opened with its content handler or in Files
 *
 * An item's id is its file name. Apps are only on the desktop when someone puts a shortcut there; installing
 * an app does not (it shows under All apps).
 *
 * Pure functions: parsing entries, the folder a new account starts with, the default arrangement, and
 * building the layout from the entries plus the positions this device saved.
 */
import type { DesktopFolderEntry, DesktopSeedItem } from '../api/desktop_items'
import { desktopCatalogIdForLogicalApp } from '../app/backend-apps'
import type {
  AppDefinition,
  DeadZone,
  DesktopPageState,
  FormFactor,
  LayoutItem,
  LayoutState,
} from './ui'

export type DesktopEntry =
  | { name: string; kind: 'app'; app: string; raw: Record<string, unknown> }
  | { name: string; kind: 'widget'; widget: string; raw: Record<string, unknown> }
  | { name: string; kind: 'file' | 'folder'; size: number; mtime: number }

export const SHORTCUT_EXTENSION = '.lnk'
export const WIDGET_EXTENSION = '.widget'

function parseObject(text: string | undefined): Record<string, unknown> | null {
  if (!text) return null
  try {
    const value: unknown = JSON.parse(text)
    return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null
  } catch {
    return null
  }
}

/** A shortcut or widget file whose content is not understood shows as the plain file it is. */
export function parseDesktopEntry(entry: DesktopFolderEntry): DesktopEntry {
  const lower = entry.name.toLowerCase()
  if (entry.kind === 'file' && lower.endsWith(SHORTCUT_EXTENSION)) {
    const raw = parseObject(entry.text)
    if (raw && typeof raw.app === 'string' && raw.app.trim()) return { name: entry.name, kind: 'app', app: raw.app.trim(), raw }
  }
  if (entry.kind === 'file' && lower.endsWith(WIDGET_EXTENSION)) {
    const raw = parseObject(entry.text)
    if (raw && typeof raw.widget === 'string' && raw.widget.trim()) return { name: entry.name, kind: 'widget', widget: raw.widget.trim(), raw }
  }
  return { name: entry.name, kind: entry.kind === 'dir' ? 'folder' : 'file', size: entry.size, mtime: entry.mtime }
}

/** Shortcut and widget files are the desktop's own; documents and folders are changed in Files. */
export function isDesktopObject(entry: DesktopEntry): entry is Extract<DesktopEntry, { kind: 'app' | 'widget' }> {
  return entry.kind === 'app' || entry.kind === 'widget'
}

/** What an entry stands for, independent of its (renamable, localized) file name. */
export function desktopEntryTarget(entry: DesktopEntry): string {
  if (entry.kind === 'app') return `app:${entry.app}`
  if (entry.kind === 'widget') return `widget:${entry.widget}`
  return `${entry.kind}:${entry.name}`
}

export function shortcutText(app: string): string {
  return `${JSON.stringify({ app }, null, 2)}\n`
}

export function widgetText(raw: Record<string, unknown>): string {
  return `${JSON.stringify(raw, null, 2)}\n`
}

// ---------------------------------------------------------------------------
// Resolving shortcuts to apps
// ---------------------------------------------------------------------------

/**
 * The app a shortcut opens: the definition with that id, else the first instance of that app (by AppId or by
 * its built-in catalog id). Undefined when the account has no such app; the shortcut is then not shown.
 */
export function resolveShortcutApp(ref: string, apps: readonly AppDefinition[]): string | undefined {
  const exact = apps.find((app) => app.id === ref)
  if (exact) return exact.id
  return apps.find(
    (app) => app.logicalAppId && (app.logicalAppId === ref || desktopCatalogIdForLogicalApp(app.logicalAppId) === ref),
  )?.id
}

/** What a new shortcut to `app` stores: the AppId of an installed app, so it keeps working across reinstalls. */
export function shortcutRefForApp(app: AppDefinition): string {
  return app.logicalAppId ?? app.id
}

// ---------------------------------------------------------------------------
// The folder a new account starts with
// ---------------------------------------------------------------------------

type Translate = (key: string, fallback?: string) => string

interface SeedShortcut {
  app: string
  labelKey: string
  fallback: string
}

/**
 * Default shortcuts to built-in apps, in the order the first screens show them. Shortcuts to installed Apps
 * (the Text Editor) are declared by the system's install settings and added by Control Panel.
 */
const SEED_SHORTCUTS: readonly SeedShortcut[] = [
  { app: 'agent-guide', labelKey: 'apps.agentGuide', fallback: 'Jarvis' },
  { app: 'messagehub', labelKey: 'apps.messagehub', fallback: 'MessageHub' },
  { app: 'my-network', labelKey: 'apps.myNetwork', fallback: 'My Network' },
  { app: 'aiworkspace', labelKey: 'apps.aiworkspace', fallback: 'AI Workspace' },
  { app: 'ai-center', labelKey: 'apps.aiCenter', fallback: 'AI Center' },
  { app: 'homestation', labelKey: 'apps.homestation', fallback: 'HomeStation' },
  { app: 'files', labelKey: 'apps.files', fallback: 'Files' },
  { app: 'app-service', labelKey: 'apps.appService', fallback: 'App Service' },
  { app: 'settings', labelKey: 'apps.settings', fallback: 'Settings' },
  { app: 'users-agents', labelKey: 'apps.usersAgents', fallback: 'Users & Agents' },
  { app: 'task-center', labelKey: 'apps.taskCenter', fallback: 'Task Center' },
  { app: 'preview', labelKey: 'apps.preview', fallback: 'Preview' },
  { app: 'workflow', labelKey: 'apps.workflow', fallback: 'Workflow' },
]

/** A display name as one file name: no separators or control characters. */
export function fileNameStem(label: string, fallback: string): string {
  // eslint-disable-next-line no-control-regex
  const cleaned = label.replace(/[/\\\u0000-\u001f\u007f]/g, ' ').replace(/\s+/g, ' ').trim().replace(/^\.+/, '')
  return cleaned || fallback
}

/** `stem + extension`, numbered when the name is taken: `Files.lnk`, `Files (2).lnk`, … */
export function uniqueFileName(stem: string, extension: string, taken: ReadonlySet<string>): string {
  const lowerTaken = new Set([...taken].map((name) => name.toLowerCase()))
  let candidate = `${stem}${extension}`
  for (let n = 2; lowerTaken.has(candidate.toLowerCase()); n++) candidate = `${stem} (${n})${extension}`
  return candidate
}

/** The files a new account's Desktop folder is created with, named in the current language. */
export function desktopSeed(t: Translate): DesktopSeedItem[] {
  const taken = new Set<string>()
  const add = (stem: string, extension: string, text: string): DesktopSeedItem => {
    const name = uniqueFileName(stem, extension, taken)
    taken.add(name)
    return { name, text }
  }
  return [
    add(fileNameStem(t('widgets.clock', 'Clock'), 'Clock'), WIDGET_EXTENSION, widgetText({ widget: 'clock' })),
    add(
      fileNameStem(t('desktop.guide.fileName', 'Getting Started'), 'Getting Started'),
      WIDGET_EXTENSION,
      widgetText({ widget: 'notepad', text: t('desktop.guide.text') }),
    ),
    ...SEED_SHORTCUTS.map((shortcut) =>
      add(fileNameStem(t(shortcut.labelKey, shortcut.fallback), shortcut.fallback), SHORTCUT_EXTENSION, shortcutText(shortcut.app)),
    ),
  ]
}

// ---------------------------------------------------------------------------
// Default arrangement (where the default items sit on a device that has saved nothing)
// ---------------------------------------------------------------------------

export interface DefaultSlot {
  target: string
  page: number
  x: number
  y: number
  w?: number
  h?: number
}

/** In a row: the shortcuts to apps the arrangement does not name (installed Apps), in file-name order. */
const OTHER_APPS = '*'

/** A row of shortcuts from column `x`: the ones the account has sit side by side, the others leave no gap. */
interface DefaultRow {
  page: number
  x: number
  y: number
  apps: readonly string[]
}

interface Arrangement {
  widgets: readonly DefaultSlot[]
  rows: readonly DefaultRow[]
}

/**
 * Desktop: the clock and the getting-started note on the left, then rows of shortcuts; the second screen has
 * the content tools. Grid coordinates are column/row.
 */
const DESKTOP_ARRANGEMENT: Arrangement = {
  widgets: [
    { target: 'widget:clock', page: 0, x: 0, y: 0, w: 2, h: 1 },
    { target: 'widget:notepad', page: 0, x: 0, y: 1, w: 2, h: 3 },
  ],
  rows: [
    { page: 0, x: 2, y: 0, apps: ['agent-guide', 'messagehub', 'my-network'] },
    { page: 0, x: 2, y: 1, apps: ['aiworkspace', 'ai-center', 'homestation'] },
    { page: 0, x: 2, y: 2, apps: ['files', 'app-service', 'settings', 'users-agents', 'task-center'] },
    { page: 1, x: 0, y: 0, apps: ['preview', OTHER_APPS, 'workflow'] },
  ],
}

/** Phone: four columns, the note spans the screen width. */
const MOBILE_ARRANGEMENT: Arrangement = {
  widgets: [
    { target: 'widget:clock', page: 0, x: 0, y: 0, w: 2, h: 1 },
    { target: 'widget:notepad', page: 0, x: 0, y: 1, w: 4, h: 2 },
  ],
  rows: [
    { page: 0, x: 2, y: 0, apps: ['agent-guide', 'messagehub'] },
    { page: 0, x: 0, y: 3, apps: ['my-network', 'aiworkspace', 'ai-center', 'homestation'] },
    { page: 0, x: 0, y: 4, apps: ['files', 'app-service', 'settings', 'users-agents'] },
    { page: 0, x: 0, y: 5, apps: ['task-center'] },
    { page: 1, x: 0, y: 0, apps: ['preview', OTHER_APPS, 'workflow'] },
  ],
}

/** Where the default items sit, given the targets the desktop shows in order (see `desktopEntryTarget`). */
export function defaultSlots(formFactor: FormFactor, present: readonly string[]): Map<string, DefaultSlot> {
  const arrangement = formFactor === 'mobile' ? MOBILE_ARRANGEMENT : DESKTOP_ARRANGEMENT
  const slots = new Map(arrangement.widgets.map((slot) => [slot.target, slot]))
  const named = new Set(arrangement.rows.flatMap((row) => row.apps.map((id) => `app:${id}`)))
  const others = present.filter((target) => target.startsWith('app:') && !named.has(target))
  for (const row of arrangement.rows) {
    let x = row.x
    for (const id of row.apps) {
      const targets = id === OTHER_APPS ? others : [`app:${id}`]
      for (const target of targets) {
        if (!present.includes(target) || slots.has(target)) continue
        slots.set(target, { target, page: row.page, x: x++, y: row.y })
      }
    }
  }
  return slots
}

const defaultWidgetSize: Record<string, { w: number; h: number }> = {
  clock: { w: 2, h: 1 },
  notepad: { w: 2, h: 2 },
}

// ---------------------------------------------------------------------------
// Layout = folder entries + this device's positions
// ---------------------------------------------------------------------------

export interface BuildDesktopLayoutInput {
  entries: readonly DesktopEntry[]
  /** The folder as Files shows it, e.g. `/home/alice/Desktop`. */
  folderPath: string
  apps: readonly AppDefinition[]
  formFactor: FormFactor
  deadZone: DeadZone
  /** Positions this device saved (or shows now); null on a device that has saved nothing. */
  positions: LayoutState | null
}

function itemForEntry(entry: DesktopEntry, input: BuildDesktopLayoutInput): LayoutItem | null {
  if (entry.kind === 'app') {
    const appId = resolveShortcutApp(entry.app, input.apps)
    return appId ? { id: entry.name, type: 'app', appId, w: 1, h: 1 } : null
  }
  if (entry.kind === 'widget') {
    const size = defaultWidgetSize[entry.widget] ?? { w: 2, h: 2 }
    const config = Object.fromEntries(Object.entries(entry.raw).filter(([key]) => key !== 'widget'))
    return { id: entry.name, type: 'widget', widgetType: entry.widget, ...size, config }
  }
  return {
    id: entry.name,
    type: 'file',
    fileKind: entry.kind,
    path: `${input.folderPath.replace(/\/+$/, '')}/${entry.name}`,
    size: entry.size,
    w: 1,
    h: 1,
  }
}

/**
 * The layout of the folder's entries. An item keeps the place it has in `positions`; on a device without
 * saved positions the default items take their default place; anything else is left unpositioned for
 * `resolveLayout` to append (by file name). Pages past the last used one are dropped.
 */
export function buildDesktopLayout(input: BuildDesktopLayoutInput): LayoutState {
  const { formFactor, positions } = input
  const pages: DesktopPageState[] = (positions?.pages ?? []).map((page) => ({ id: page.id, items: [] }))
  const ensurePage = (index: number) => {
    while (pages.length <= index) pages.push({ id: `${formFactor}-page-${pages.length + 1}`, items: [] })
    return pages[index]
  }
  ensurePage(0)

  const saved = new Map<string, { page: number; item: LayoutItem }>()
  positions?.pages.forEach((page, pageIndex) => {
    for (const item of page.items) saved.set(item.id, { page: pageIndex, item })
  })
  const shown = input.entries.flatMap((entry) => {
    const item = itemForEntry(entry, input)
    return item ? [{ entry, item }] : []
  })
  const defaults = defaultSlots(formFactor, shown.map(({ entry }) => desktopEntryTarget(entry)))
  const usedDefaults = new Set<string>()

  shown.forEach(({ entry, item }, order) => {
    const previous = saved.get(entry.name)
    if (previous && previous.item.type === item.type) {
      const { x, y, w, h, slotIndex, preferredPage, placementType, seq } = previous.item
      ensurePage(previous.page).items.push({
        ...item,
        ...(item.type === 'widget' ? { w, h } : {}),
        x, y, slotIndex, placementType, seq,
        preferredPage: preferredPage ?? previous.page,
      })
      return
    }
    const target = desktopEntryTarget(entry)
    const slot = positions ? undefined : defaults.get(target)
    if (slot && !usedDefaults.has(target)) {
      usedDefaults.add(target)
      ensurePage(slot.page).items.push({
        ...item,
        ...(item.type === 'widget' && slot.w && slot.h ? { w: slot.w, h: slot.h } : {}),
        x: slot.x,
        y: slot.y,
        preferredPage: slot.page,
        placementType: 'auto',
        seq: order,
      })
      return
    }
    // A new item lands on the first page with room; `seq` keeps file-name order among them.
    pages[0].items.push({ ...item, x: undefined, y: undefined, slotIndex: undefined, placementType: 'auto', seq: 10_000 + order })
  })

  while (pages.length > 1 && pages[pages.length - 1].items.length === 0) pages.pop()
  return { version: 1, formFactor, deadZone: input.deadZone, pages }
}

/** True when two listings hold the same entries (names, kinds and object content). */
export function sameDesktopEntries(a: readonly DesktopEntry[], b: readonly DesktopEntry[]): boolean {
  return entriesSignature(a) === entriesSignature(b)
}

function entriesSignature(entries: readonly DesktopEntry[]): string {
  return JSON.stringify(entries.map((entry) => ('raw' in entry ? [entry.name, entry.kind, entry.raw] : [entry.name, entry.kind, entry.size, entry.mtime])))
}
