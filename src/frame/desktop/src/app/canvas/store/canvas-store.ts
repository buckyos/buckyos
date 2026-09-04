/* ── Editor store: document + history + UI state. All doc changes go through dispatch(). ── */

import type { CanvasCommand } from '../domain/commands'
import { QUIET_COMMANDS } from '../domain/commands'
import { applyCommand } from '../domain/reducer'
import type { CanvasBlock, CanvasDocument, Camera, WishState } from '../domain/types'
import type { CanvasStorageAdapter } from '../storage/indexeddb'
import { trackEvent } from '../events'
import { loadSettings, saveSettings, type CanvasSettings } from './settings'

export type SaveStatus = 'idle' | 'saving' | 'saved' | 'error'

export interface RunLogLine {
  at: string
  text: string
  level: 'info' | 'warning' | 'error'
}

export interface RunState {
  wishId: string
  runId: string
  stage: WishState
  message: string
  percent?: number
  log: RunLogLine[]
  warnings: string[]
  startedAt: string
  error?: string
  errorDetails?: string[]
  errorKind?: string
  cellKey?: string
}

export type PlacementTool = 'text' | 'table' | 'wish' | 'frame' | 'metric' | 'image'

export interface PresentationSession {
  pathId: string
  index: number
  deviated: boolean
}

export interface EditorUiState {
  selection: string[]
  editingBlockId: string | null
  tool: PlacementTool | null
  sidebarTab: 'sheets' | 'data' | 'presentation'
  selectedPathId: string | null
  selectedStepId: string | null
  presentation: PresentationSession | null
  highlightBlockId: string | null
  highlightRun: boolean
  spaceHeld: boolean
  tableSelection: { blockId: string; range: { rowStart: number; rowEnd: number; colStart: number; colEnd: number } } | null
  toast: { id: number; text: string; tone: 'info' | 'error' | 'success' } | null
}

export interface StoreState {
  doc: CanvasDocument
  past: CanvasDocument[]
  future: CanvasDocument[]
  ui: EditorUiState
  runs: Record<string, RunState>
  saveStatus: SaveStatus
  saveError?: string
  dirty: boolean
  settings: CanvasSettings
  clipboard: CanvasBlock[]
}

const MAX_HISTORY = 100

export class CanvasStore {
  private listeners = new Set<() => void>()
  private state: StoreState
  private saveTimer: ReturnType<typeof setTimeout> | null = null
  private transientBase: CanvasDocument | null = null
  private toastSeq = 0

  private readonly storage: CanvasStorageAdapter

  constructor(doc: CanvasDocument, storage: CanvasStorageAdapter) {
    this.storage = storage
    doc = sanitizeDocument(doc)
    this.state = {
      doc,
      past: [],
      future: [],
      ui: {
        selection: [],
        editingBlockId: null,
        tool: null,
        sidebarTab: 'sheets',
        selectedPathId: doc.presentationPaths[0]?.id ?? null,
        selectedStepId: null,
        presentation: null,
        highlightBlockId: null,
        highlightRun: false,
        spaceHeld: false,
        tableSelection: null,
        toast: null,
      },
      runs: {},
      saveStatus: 'idle',
      dirty: false,
      settings: loadSettings(),
      clipboard: [],
    }
  }

  getState = () => this.state
  subscribe = (fn: () => void) => {
    this.listeners.add(fn)
    return () => {
      this.listeners.delete(fn)
    }
  }
  private set(patch: Partial<StoreState>) {
    this.state = { ...this.state, ...patch }
    for (const l of this.listeners) l()
  }

  get doc() {
    return this.state.doc
  }

  /* ── commands / history ── */

  dispatch(cmd: CanvasCommand): boolean {
    const prev = this.state.doc
    let next: CanvasDocument
    try {
      next = applyCommand(prev, cmd)
    } catch (e) {
      this.toast(e instanceof Error ? e.message : String(e), 'error')
      return false
    }
    if (next === prev) return false
    if (QUIET_COMMANDS.has(cmd.type) || this.transientBase) {
      this.set({ doc: next, dirty: true })
    } else {
      this.set({ doc: next, past: [...this.state.past, prev].slice(-MAX_HISTORY), future: [], dirty: true })
    }
    this.scheduleSave()
    return true
  }

  /** Drag/resize: many intermediate updates, one history entry. */
  beginTransient() {
    if (!this.transientBase) this.transientBase = this.state.doc
  }
  endTransient() {
    const base = this.transientBase
    this.transientBase = null
    if (base && base !== this.state.doc) {
      this.set({ past: [...this.state.past, base].slice(-MAX_HISTORY), future: [] })
    }
  }

  undo() {
    const past = this.state.past
    if (!past.length) return
    const prev = past[past.length - 1]
    const current = this.state.doc
    // keep runtime-only wish states from the current doc so undo doesn't resurrect "running"
    this.set({ doc: carryQuietState(current, prev), past: past.slice(0, -1), future: [current, ...this.state.future], dirty: true })
    this.setUi({ selection: this.state.ui.selection.filter((id) => this.state.doc.blocks[id]) })
    trackEvent('undo_used')
    this.scheduleSave()
  }

  redo() {
    const future = this.state.future
    if (!future.length) return
    const next = future[0]
    const current = this.state.doc
    this.set({ doc: carryQuietState(current, next), past: [...this.state.past, current], future: future.slice(1), dirty: true })
    this.setUi({ selection: this.state.ui.selection.filter((id) => this.state.doc.blocks[id]) })
    this.scheduleSave()
  }

  get canUndo() {
    return this.state.past.length > 0
  }
  get canRedo() {
    return this.state.future.length > 0
  }

  /* ── ui ── */

  setUi(patch: Partial<EditorUiState>) {
    this.set({ ui: { ...this.state.ui, ...patch } })
  }

  select(ids: string[], options: { additive?: boolean } = {}) {
    const base = options.additive ? this.state.ui.selection : []
    const merged = [...new Set([...base, ...ids])]
    this.setUi({ selection: merged, editingBlockId: merged.length === 1 && this.state.ui.editingBlockId === merged[0] ? this.state.ui.editingBlockId : null })
  }

  toggleSelect(id: string) {
    const sel = this.state.ui.selection
    this.setUi({ selection: sel.includes(id) ? sel.filter((x) => x !== id) : [...sel, id] })
  }

  clearSelection() {
    if (this.state.ui.selection.length || this.state.ui.editingBlockId) this.setUi({ selection: [], editingBlockId: null })
  }

  setCamera(camera: Camera) {
    this.dispatch({ type: 'SET_CAMERA', sheetId: this.state.doc.activeSheetId, camera })
  }

  toast(text: string, tone: 'info' | 'error' | 'success' = 'info') {
    const id = ++this.toastSeq
    this.setUi({ toast: { id, text, tone } })
    setTimeout(() => {
      if (this.state.ui.toast?.id === id) this.setUi({ toast: null })
    }, tone === 'error' ? 6000 : 3000)
  }

  setClipboard(blocks: CanvasBlock[]) {
    this.set({ clipboard: blocks })
  }

  /* ── settings ── */

  updateSettings(patch: Partial<CanvasSettings>) {
    const settings = { ...this.state.settings, ...patch }
    saveSettings(settings)
    this.set({ settings })
  }

  /* ── runs ── */

  setRun(wishId: string, patch: Partial<RunState> | null) {
    const runs = { ...this.state.runs }
    if (patch === null) delete runs[wishId]
    else runs[wishId] = { ...(runs[wishId] ?? emptyRun(wishId)), ...patch }
    this.set({ runs })
  }

  appendRunLog(wishId: string, text: string, level: RunLogLine['level'] = 'info') {
    const run = this.state.runs[wishId]
    if (!run) return
    this.setRun(wishId, { log: [...run.log, { at: new Date().toISOString(), text, level }] })
  }

  /* ── persistence ── */

  private scheduleSave() {
    if (this.saveTimer) clearTimeout(this.saveTimer)
    this.saveTimer = setTimeout(() => void this.saveNow(), 800)
  }

  async saveNow(): Promise<boolean> {
    if (this.saveTimer) {
      clearTimeout(this.saveTimer)
      this.saveTimer = null
    }
    const doc = this.state.doc
    this.set({ saveStatus: 'saving' })
    try {
      await this.storage.save(doc)
      this.set({ saveStatus: 'saved', dirty: this.state.doc !== doc, saveError: undefined })
      return true
    } catch (e) {
      this.set({ saveStatus: 'error', saveError: e instanceof Error ? e.message : String(e) })
      return false
    }
  }

  replaceDocument(doc: CanvasDocument) {
    this.set({ doc, past: [], future: [], dirty: true, runs: {} })
    this.setUi({ selection: [], editingBlockId: null, presentation: null })
    this.scheduleSave()
  }

  dispose() {
    if (this.saveTimer) clearTimeout(this.saveTimer)
  }
}

function emptyRun(wishId: string): RunState {
  return { wishId, runId: '', stage: 'idle', message: '', log: [], warnings: [], startedAt: new Date().toISOString() }
}

const RUNNING = new Set<WishState>(['planning', 'running', 'validating', 'applying', 'waiting_permission'])

/** Documents loaded from storage may carry a run that never finished (page closed mid-run). */
export function sanitizeDocument(doc: CanvasDocument): CanvasDocument {
  let blocks = doc.blocks
  for (const [id, b] of Object.entries(doc.blocks)) {
    if (b.type === 'wish' && RUNNING.has(b.content.state)) {
      blocks = { ...blocks, [id]: { ...b, content: { ...b.content, state: 'failed', lastError: '上次运行未完成（页面已关闭），请重新运行' } } }
    }
  }
  return blocks === doc.blocks ? doc : { ...doc, blocks }
}

/**
 * After undo/redo, runtime-only wish fields (state, error, history) always come from the live doc:
 * history snapshots are taken mid-run (e.g. while "validating") and must never resurrect that state.
 */
function carryQuietState(live: CanvasDocument, target: CanvasDocument): CanvasDocument {
  const blocks = { ...target.blocks }
  for (const [id, b] of Object.entries(live.blocks)) {
    const t = blocks[id]
    if (b.type === 'wish' && t?.type === 'wish') {
      blocks[id] = { ...t, content: { ...t.content, state: b.content.state, lastError: b.content.lastError, lastRunId: b.content.lastRunId, runHistory: b.content.runHistory } }
    }
  }
  for (const [id, t] of Object.entries(blocks)) {
    if (t.type === 'wish' && !live.blocks[id] && RUNNING.has(t.content.state)) {
      blocks[id] = { ...t, content: { ...t.content, state: 'idle' } }
    }
  }
  const sheets = target.sheets.map((s) => {
    const liveSheet = live.sheets.find((x) => x.id === s.id)
    return liveSheet ? { ...s, camera: liveSheet.camera } : s
  })
  return { ...target, blocks, sheets, activeSheetId: target.sheets.some((s) => s.id === live.activeSheetId) ? live.activeSheetId : target.activeSheetId }
}
