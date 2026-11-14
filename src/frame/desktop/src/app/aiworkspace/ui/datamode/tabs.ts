/* The tabs of the data mode (UI 框架 PRD §5.3, §12): which objects are open and which one is current. The fixed tabs and
 * the current one are the person's state in this workspace (`ui:data-tabs`, restored when the data mode comes back);
 * the preview tab (a single click: italic, reused by the next single click) and the phone's layers are this window's.
 * At most 20 tabs: past that the earliest opened one without changes is closed. One controller per workspace store.
 *
 * Shell-level requests (`openDataEditor`) come from anywhere — the canvas's "查看数据", a phone's "展开" (§6.4), "成员与权限…"
 * (§8.1) — and the workspace shell answers them: it switches to the data mode and opens the editor here. */

import type { Json } from '../../api/types'
import { Emitter } from '../../state/emitter'
import type { WorkspaceStore } from '../../state/store'

/** The tab of the Workspace's permission editor (§5.2 "工作区权限"); entity ids never start with `@`. */
export const PERMISSIONS_TAB = '@permissions'
export const MAX_TABS = 20
const KEY = 'ui:data-tabs'

/** The three pages of a macro's editor (§5.3). */
export type MacroPage = 'task' | 'code' | 'runs'

export interface DataTab {
  /** An entity id, or `PERMISSIONS_TAB`. */
  id: string
  /** The preview tab: replaced by the next single click, fixed by a double click or a change. */
  preview: boolean
  /** When it was opened (the oldest unchanged one goes past `MAX_TABS`). */
  at: number
  page?: MacroPage
}

/** The phone's single column (§5.6): the object tree, or the current object's editor; `returnTo`: where "back" from the
 * editor leads (the canvas when "展开" opened it). */
export interface PhoneLayer { layer: 'tree' | 'editor'; returnTo: 'tree' | 'canvas' }

export interface DataTabsState {
  tabs: DataTab[]
  active: string | null
  phone: PhoneLayer
}

export interface OpenOptions {
  /** A preview (a single click in the tree); otherwise a fixed tab of its own. */
  preview?: boolean
  page?: MacroPage
  /** On a phone: show the editor (default), and where its "back" leads. */
  returnTo?: 'tree' | 'canvas'
}

interface Stored { tabs: { id: string; at: number; page?: MacroPage }[]; active: string | null }

function parse(value: Json | undefined): Stored {
  const raw = value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, Json> : {}
  const tabs = Array.isArray(raw.tabs) ? raw.tabs.flatMap((t) => {
    if (!t || typeof t !== 'object' || Array.isArray(t) || typeof t.id !== 'string') return []
    const page: MacroPage | undefined = t.page === 'task' || t.page === 'code' || t.page === 'runs' ? t.page : undefined
    return [{ id: t.id, at: typeof t.at === 'number' ? t.at : 0, ...(page ? { page } : {}) }]
  }) : []
  return { tabs: tabs.slice(0, MAX_TABS), active: typeof raw.active === 'string' ? raw.active : null }
}

/** Does an edit id name this entity (`key:<id>:…`, `wish:<id>:…`, `cell:<source>:…`)? */
export function editNames(editId: string, entityId: string): boolean {
  return editId.split(':').includes(entityId)
}

export class DataTabs {
  private state: DataTabsState
  private readonly emitter = new Emitter()
  private readonly store: WorkspaceStore
  private seq = 0
  readonly subscribe = this.emitter.subscribe
  snapshot = (): DataTabsState => this.state

  constructor(store: WorkspaceStore) {
    this.store = store
    this.state = { ...this.restored(null), phone: { layer: 'tree', returnTo: 'tree' } }
    // another window or device (a pull) changed the fixed tabs: take them, keep this window's preview
    store.userState.subscribeKey(KEY, () => {
      const next = this.restored(this.state.tabs.find((t) => t.preview) ?? null)
      if (JSON.stringify(next) === JSON.stringify({ tabs: this.state.tabs, active: this.state.active })) return
      this.state = { ...next, phone: this.state.phone }
      this.emitter.emit()
    })
  }

  private restored(preview: DataTab | null): { tabs: DataTab[]; active: string | null } {
    const stored = parse(this.store.userState.get(KEY))
    const tabs: DataTab[] = stored.tabs.map((t) => ({ ...t, preview: false }))
    if (preview && !tabs.some((t) => t.id === preview.id)) tabs.push(preview)
    const ids = new Set(tabs.map((t) => t.id))
    const current = this.state?.active
    const active = current && ids.has(current) && preview?.id === current ? current : stored.active && ids.has(stored.active) ? stored.active : tabs.at(-1)?.id ?? null
    return { tabs, active }
  }

  private commit(tabs: DataTab[], active: string | null, phone: PhoneLayer = this.state.phone) {
    this.state = { tabs, active, phone }
    const fixed = tabs.filter((t) => !t.preview)
    const stored: Stored = { tabs: fixed.map((t) => ({ id: t.id, at: t.at, ...(t.page ? { page: t.page } : {}) })), active: fixed.some((t) => t.id === active) ? active : fixed.at(-1)?.id ?? null }
    if (JSON.stringify(parse(this.store.userState.get(KEY))) !== JSON.stringify(stored)) this.store.userState.set(KEY, stored as unknown as Json)
    this.emitter.emit()
  }

  private now(): number {
    this.seq += 1
    return Date.now() * 1000 + (this.seq % 1000)
  }

  /** Has the tab changes that are not on the backend yet (its unsaved dot)? */
  dirty(id: string): boolean {
    for (const entry of this.store.edits.snapshot().values()) if (entry.state !== 'committed' && editNames(entry.id, id)) return true
    return false
  }

  /** Open `id`: a preview replaces the preview there is (a fixed tab of the same object is just made current). */
  open(id: string, options: OpenOptions = {}) {
    const { tabs } = this.state
    const existing = tabs.find((t) => t.id === id)
    let next: DataTab[]
    if (existing) {
      next = tabs.map((t) => (t.id === id ? { ...t, preview: t.preview && Boolean(options.preview), ...(options.page ? { page: options.page } : {}) } : t))
    } else {
      const tab: DataTab = { id, preview: Boolean(options.preview), at: this.now(), ...(options.page ? { page: options.page } : {}) }
      const previewAt = tabs.findIndex((t) => t.preview)
      if (previewAt >= 0) {
        // the preview tab is reused in place; a fixed tab goes right after the current one
        next = tab.preview ? tabs.map((t, i) => (i === previewAt ? tab : t)) : [...tabs]
        if (!tab.preview) {
          const at = tabs.findIndex((t) => t.id === this.state.active)
          next.splice(at < 0 ? next.length : at + 1, 0, tab)
        }
      } else {
        const at = tabs.findIndex((t) => t.id === this.state.active)
        next = [...tabs]
        next.splice(at < 0 ? next.length : at + 1, 0, tab)
      }
      next = this.trimmed(next, id)
    }
    this.commit(next, id, { layer: 'editor', returnTo: options.returnTo ?? (this.state.phone.layer === 'editor' ? this.state.phone.returnTo : 'tree') })
  }

  /** Past `MAX_TABS`, close the earliest opened tabs without changes (never `keep`). */
  private trimmed(tabs: DataTab[], keep: string): DataTab[] {
    const out = [...tabs]
    while (out.length > MAX_TABS) {
      const victim = [...out].filter((t) => t.id !== keep && !this.dirty(t.id)).sort((a, b) => (a.preview === b.preview ? a.at - b.at : a.preview ? -1 : 1))[0]
      if (!victim) break
      out.splice(out.indexOf(victim), 1)
    }
    return out
  }

  /** The preview becomes a fixed tab (a double click, a change inside it). */
  pin(id: string) {
    if (!this.state.tabs.some((t) => t.id === id && t.preview)) return
    this.commit(this.state.tabs.map((t) => (t.id === id ? { ...t, preview: false } : t)), this.state.active)
  }

  activate(id: string) {
    if (!this.state.tabs.some((t) => t.id === id)) return
    this.commit(this.state.tabs, id, { ...this.state.phone, layer: 'editor' })
  }

  setPage(id: string, page: MacroPage) {
    this.commit(this.state.tabs.map((t) => (t.id === id ? { ...t, page } : t)), this.state.active)
  }

  close(id: string) {
    const { tabs, active } = this.state
    const at = tabs.findIndex((t) => t.id === id)
    if (at < 0) return
    const next = tabs.filter((t) => t.id !== id)
    const nextActive = active === id ? (next[at] ?? next[at - 1])?.id ?? null : active
    this.commit(next, nextActive, nextActive ? this.state.phone : { layer: 'tree', returnTo: 'tree' })
  }

  closeOthers(id: string) {
    this.commit(this.state.tabs.filter((t) => t.id === id), id)
  }

  closeRight(id: string) {
    const at = this.state.tabs.findIndex((t) => t.id === id)
    if (at < 0) return
    const next = this.state.tabs.slice(0, at + 1)
    this.commit(next, next.some((t) => t.id === this.state.active) ? this.state.active : id)
  }

  /** Drag a tab to another place in the strip. */
  move(id: string, beforeId: string | null) {
    const tab = this.state.tabs.find((t) => t.id === id)
    if (!tab || id === beforeId) return
    const rest = this.state.tabs.filter((t) => t.id !== id)
    const at = beforeId ? rest.findIndex((t) => t.id === beforeId) : -1
    rest.splice(at < 0 ? rest.length : at, 0, tab)
    this.commit(rest, this.state.active)
  }

  /** The phone's single column: the tree, or the current editor. */
  showTree() {
    this.commit(this.state.tabs, this.state.active, { layer: 'tree', returnTo: 'tree' })
  }
}

const controllers = new WeakMap<WorkspaceStore, DataTabs>()

export function dataTabsOf(store: WorkspaceStore): DataTabs {
  let tabs = controllers.get(store)
  if (!tabs) { tabs = new DataTabs(store); controllers.set(store, tabs) }
  return tabs
}

// ---- the shell-level way in

export interface DataEditorRequest extends OpenOptions { entityId: string }

const requests = new Emitter()
let pending: DataEditorRequest | null = null

/**
 * Open an object's standard editor in the data mode, from anywhere in the workspace window: the shell switches to the
 * data mode and opens (or makes current) a fixed tab of it. `entityId` is a data entity, or `PERMISSIONS_TAB` for the
 * Workspace's permission editor. On a phone the editor is shown full screen; `returnTo: 'canvas'` makes its "back" go
 * straight back to the canvas (a free canvas's "展开", PRD §6.4). `page` opens a macro on its "code" page.
 */
export function openDataEditor(entityId: string, options: OpenOptions = {}) {
  pending = { entityId, ...options }
  requests.emit()
}

/** The shell's side: the request waiting, taken once. */
export function onDataEditorRequest(listener: (request: DataEditorRequest) => void): () => void {
  const take = () => { const r = pending; pending = null; if (r) listener(r) }
  const off = requests.subscribe(take)
  take()
  return off
}
