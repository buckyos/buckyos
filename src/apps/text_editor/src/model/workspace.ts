import { contentRefString, contentDescriptor, type OpenRequest, type TransferableContentRef, type TransferableSessionContext } from 'buckyos/content'
import type { PreviewEmbedOpenWith } from 'buckyos/app-frame'
import { ContentError, decodeText, encodeText, FULL_FEATURE_LIMIT } from '../fs/codec.ts'
import { sourceKey, splitPath, isMissing, errorCategory, SaveConflict, SaveError, type DocumentStore, type ErrorCategory } from '../fs/documentStore.ts'
import { BufferStore, type BufferRecord, type BufferHeader } from '../fs/bufferStore.ts'
import { RecoveryStore, type RecoveryRecord } from '../fs/recoveryStore.ts'
import { archiveMine, saveDocument, type SaveDocument } from '../fs/saveFlow.ts'
import { LocalStore, holdLock } from '../fs/localStore.ts'
import { defaultSettings, type Settings, type SettingsStore } from '../platform/settings.ts'
import { implicitSession } from '../platform/launch.ts'
export interface EditorDocument extends SaveDocument {
  id: string; version: number; savedText: string; savedFormat: string; largeFile: boolean; writable: boolean
  disk: 'in-sync' | 'changed' | 'deleted' | 'saving' | 'lease-busy' | 'error'
  buffer?: BufferRecord; syncedVersion: number; localVersion: number; language?: string
  creating?: Promise<void>; flushing?: Promise<void>; saving?: boolean; retryDelay?: number; mutating?: boolean; error?: string; release?: () => void
  /** Shown read-only in the system Preview instead of the text editor; never buffered, saved or recovered. */
  preview?: PreviewView
}
/**
 * `not-text`, `too-large`, `media`: content this editor does not edit. `explicit`: the Preview action on a text
 * document, either of its unsaved `draft` (a snapshot taken at `takenAt`) or of the saved file `ref`.
 */
export interface PreviewView { reason: 'not-text' | 'too-large' | 'media' | 'explicit'; of?: string; ref?: TransferableContentRef; draft?: Blob; takenAt?: number; serial: number }
/** What went wrong, for a message that names the step, the path and whether the draft is safe. */
export interface Failure { stage: SaveError['stage'] | 'open' | 'close'; path: string; category: ErrorCategory; unknown?: boolean; draft?: 'synced' | 'local' | 'none' }
export class Problem extends Error {
  failure: Failure; retry?: () => Promise<unknown>
  constructor(key: string, failure: Failure, retry?: () => Promise<unknown>) { super(key); this.failure = failure; this.retry = retry }
}
const MEDIA = /^(image\/(?!svg)|audio\/|video\/|font\/|application\/pdf$)/
const CLOSE_SYNC_WAIT = 2000
export interface Tab { doc: string; transient: boolean }
export interface Group { id: string; tabs: Tab[]; active?: string }
export interface SessionItem { source: TransferableContentRef; title: string }
export interface Choice { title: string; options: string[]; resolve: (answer: string) => void }
export const formatKey = (doc: SaveDocument): string => JSON.stringify([doc.encoding, doc.bom, doc.eol])
export const dirty = (doc: EditorDocument): boolean => doc.text !== doc.savedText || formatKey(doc) !== doc.savedFormat
export class Workspace {
  documents = new Map<string, EditorDocument>()
  groups: Group[] = [{ id: crypto.randomUUID(), tabs: [] }]
  activeGroup = this.groups[0].id
  layout: 'single' | 'columns-2' | 'rows-2' = 'single'
  session?: TransferableSessionContext
  items: SessionItem[] = []
  sidebar = !matchMedia('(max-width: 719px)').matches; sidebarWidth = 235; outlineRatio = 50
  settings: Settings = { ...defaultSettings }
  locale = navigator.language; theme: 'dark' | 'light' = matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
  message = ''; failure?: Failure; retry?: () => Promise<unknown>
  choice?: Choice; recoveries: RecoveryRecord[] = []; abandoned: BufferRecord[] = []; recent: TransferableContentRef[] = []
  sessionLost = false; sessionLostMuted = false
  onLogin?: () => Promise<void>
  panel?: 'recovery' | 'buffers' | 'settings' | 'quick' | 'location'
  locationAction?: (path: string) => Promise<void>
  store: DocumentStore; buffers: BufferStore; recovery: RecoveryStore; local: LocalStore; settingsStore: SettingsStore
  origin: { device: string; windowId: string } = { device: '', windowId: crypto.randomUUID() }
  revision = 0
  /** `cyfs:///home/<user>`: where location choices start without an open document. */
  home = ''
  /** The Zone's system entry: where the embedded Preview is served from. */
  systemOrigin = ''
  /** Hands "Open with…" to the system chooser; false when no system entry is reachable. */
  onOpenWith?: (open: PreviewEmbedOpenWith) => boolean
  private listeners = new Set<() => void>()
  private timers = new Map<string, { local?: ReturnType<typeof setTimeout>; remote?: ReturnType<typeof setTimeout>; max?: ReturnType<typeof setTimeout> }>()
  private requests = new Set<string>()
  private owner?: () => void
  private snapshotTimer?: ReturnType<typeof setTimeout>
  private checking?: Promise<void>
  private checkAgain = false
  constructor(store: DocumentStore, buffers: BufferStore, recovery: RecoveryStore, local: LocalStore, settingsStore: SettingsStore) {
    this.store = store; this.buffers = buffers; this.recovery = recovery; this.local = local; this.settingsStore = settingsStore
  }
  subscribe = (fn: () => void): (() => void) => { this.listeners.add(fn); return () => this.listeners.delete(fn) }
  getSnapshot = (): number => this.revision
  emit(): void {
    this.revision++; this.listeners.forEach(fn => fn())
    if (this.owner && this.settings.restoreSession) {
      clearTimeout(this.snapshotTimer)
      this.snapshotTimer = setTimeout(() => { void this.saveSnapshot().catch(e => this.notify(String(e))) }, 500)
    }
  }
  /** Draft previews are snapshots of this window only and are not restored. */
  async saveSnapshot(): Promise<void> {
    if (!this.owner || !this.settings.restoreSession) return
    clearTimeout(this.snapshotTimer)
    const kept = (t: Tab) => this.documents.get(t.doc)?.preview?.reason !== 'explicit'
    await this.local.put('workspace', { groups: this.groups.map(g => ({ ...g, tabs: g.tabs.filter(kept).map(t => ({ ...t, source: this.documents.get(t.doc)?.source, bufferId: this.documents.get(t.doc)?.buffer?.header.bufferId })) })), layout: this.layout, activeGroup: this.activeGroup, session: this.session, sidebar: this.sidebar, sidebarWidth: this.sidebarWidth, outlineRatio: this.outlineRatio })
  }
  notify(message: string): void { this.message = message; this.failure = undefined; this.retry = undefined; this.revision++; this.listeners.forEach(fn => fn()) }
  report(problem: Problem): void { this.message = problem.message; this.failure = problem.failure; this.retry = problem.retry; this.revision++; this.listeners.forEach(fn => fn()) }
  run(task: Promise<unknown>): void { void task.catch(e => e instanceof Problem ? this.report(e) : this.notify(e instanceof Error ? e.message : String(e))) }
  async start(): Promise<void> {
    this.settings = await this.settingsStore.load()
    const device = await this.local.get<string>('device') ?? `${crypto.randomUUID().slice(0, 8)} · ${navigator.platform}`
    await this.local.put('device', device); this.origin.device = device
    this.recent = await this.local.get<TransferableContentRef[]>('recent') ?? []
    this.owner = await holdLock('te.session-owner')
    try { await this.buffers.replay(); await this.refreshRecords() } catch (e) { this.notify(String(e)) }
    if (this.owner && this.settings.restoreSession) {
      const saved = await this.local.get<{ groups: Array<Group & { tabs: Array<Tab & { source?: TransferableContentRef; bufferId?: string }> }>; layout: Workspace['layout']; activeGroup: string; session?: TransferableSessionContext; sidebar: boolean; sidebarWidth: number; outlineRatio: number }>('workspace')
      if (saved) {
        this.groups = saved.groups.map(g => ({ id: g.id, tabs: [] })); this.layout = saved.layout
        for (const group of saved.groups) {
          this.activeGroup = group.id
          for (const tab of group.tabs) {
            try {
              const buffer = this.abandoned.find(b => b.header.bufferId === tab.bufferId)
              if (buffer && !await this.buffers.active(buffer)) await this.restoreBuffer(buffer)
              else if (tab.source) await this.open({ requestId: crypto.randomUUID(), source: tab.source }, false, false)
            } catch (e) { this.notify(String(e)) }
          }
        }
        this.activeGroup = saved.activeGroup; this.session = saved.session; this.sidebar = saved.sidebar && !matchMedia('(max-width: 719px)').matches; this.sidebarWidth = saved.sidebarWidth; this.outlineRatio = saved.outlineRatio
        if (this.session) await this.refreshItems()
      }
    }
    this.emit()
  }
  get group(): Group { return this.groups.find(g => g.id === this.activeGroup) ?? this.groups[0] }
  get active(): EditorDocument | undefined { return this.group.active ? this.documents.get(this.group.active) : undefined }
  async open(request: OpenRequest, transient = false, checkBuffers = true): Promise<void> {
    if (this.requests.has(request.requestId)) { if (request.session) { this.session = request.session; await this.refreshItems() }; return }
    const key = contentRefString(request.source)
    let doc = [...this.documents.values()].find(d => sourceKey(d.source) === key)
    if (!doc) {
      try { doc = await this.load(request) }
      catch (e) { throw new Problem(`open-${errorCategory(e)}`, { stage: 'open', path: key, category: errorCategory(e) }, () => this.open({ ...request, requestId: crypto.randomUUID() }, transient, checkBuffers)) }
      this.documents.set(doc.id, doc)
    }
    this.requests.add(request.requestId)
    this.show(doc, transient)
    this.session = request.session && request.session.kind !== 'single' ? request.session : implicitSession(request.source)
    await this.refreshItems()
    this.recent = [request.source, ...this.recent.filter(s => sourceKey(s) !== key)].slice(0, 20)
    await this.local.put('recent', this.recent)
    if (checkBuffers) {
      await this.refreshRecords()
      if (this.abandoned.some(b => sourceKey(b.header.source) === key && b.header.bufferId !== doc.buffer?.header.bufferId)) this.panel = 'buffers'
    }
    this.emit()
  }
  /**
   * Text this editor can edit opens as text; anything else (not decodable, too large, a known media type) opens
   * in the embedded system Preview. The decision is made on the bytes, not the name: an unknown extension is read.
   */
  private async load(request: OpenRequest): Promise<EditorDocument> {
    const key = contentRefString(request.source)
    const descriptor = contentDescriptor(request.source)
    if (MEDIA.test(descriptor.mime ?? '')) {
      const stat = await this.store.stat(request.source)
      return this.previewDocument(request.source, stat.info.name ?? descriptor.name ?? key, { reason: 'media' })
    }
    let file: Awaited<ReturnType<DocumentStore['read']>>
    try { file = await this.store.read(request.source) }
    catch (e) { if (e instanceof ContentError && e.code === 'too-large') return this.previewDocument(request.source, descriptor.name ?? key, { reason: 'too-large' }); throw e }
    let decoded: ReturnType<typeof decodeText>
    try { decoded = decodeText(file.bytes) }
    catch (e) { if (e instanceof ContentError && (e.code === 'binary' || e.code === 'too-large')) return this.previewDocument(request.source, file.info.name ?? descriptor.name ?? key, { reason: e.code === 'binary' ? 'not-text' : 'too-large' }); throw e }
    const writable = request.source.kind === 'cyfs-path' && request.mode !== 'view' && file.info.capabilities.accepts_content !== false && !file.info.flags?.includes('readonly')
    return { ...decoded, id: crypto.randomUUID(), source: request.source, name: file.info.name ?? key.split('/').pop() ?? key,
      base: { etag: file.etag, size: file.size }, text: decoded.text, savedText: decoded.text, savedFormat: formatKey(decoded as SaveDocument),
      readOnly: decoded.readOnly || !writable || request.mode === 'view', writable, largeFile: file.size > FULL_FEATURE_LIMIT,
      disk: 'in-sync', version: 0, syncedVersion: -1, localVersion: -1 }
  }
  private previewDocument(source: TransferableContentRef | undefined, name: string, preview: Omit<PreviewView, 'serial'>): EditorDocument {
    return { id: crypto.randomUUID(), source, name, text: '', savedText: '', encoding: 'utf-8', bom: false, eol: '\n', mixedEol: false, savedFormat: JSON.stringify(['utf-8', false, '\n']),
      readOnly: true, writable: false, largeFile: false, disk: 'in-sync', version: 0, syncedVersion: -1, localVersion: -1, preview: { ...preview, serial: 0 } }
  }
  /** The Preview action on a text document: its unsaved draft when it has one, else the saved file. Editing state stays as it is. */
  previewOf(doc = this.active): void {
    if (!doc || doc.preview) return
    const draft = dirty(doc) || !doc.source
    const view: Omit<PreviewView, 'serial'> = { reason: 'explicit', of: doc.id, takenAt: Date.now(),
      ...draft ? { draft: new Blob([doc.text], { type: (doc.source ? contentDescriptor(doc.source, { name: doc.name }) : contentDescriptor({ kind: 'cyfs-path', path: `cyfs:///${doc.name}` })).mime }) } : { ref: doc.source } }
    const existing = [...this.documents.values()].find(d => d.preview?.of === doc.id)
    if (existing) { existing.preview = { ...view, serial: existing.preview!.serial + 1 }; existing.name = doc.name; this.show(existing); return }
    const preview = this.previewDocument(undefined, doc.name, view)
    this.documents.set(preview.id, preview); this.show(preview)
  }
  show(doc: EditorDocument, transient = false): void {
    const group = this.group
    if (!group.tabs.some(t => t.doc === doc.id)) {
      const previous = group.tabs.find(t => t.transient && !dirty(this.documents.get(t.doc)!))
      if (previous && transient) {
        group.tabs = group.tabs.filter(t => t !== previous)
        if (!this.groups.some(g => g.tabs.some(t => t.doc === previous.doc))) this.documents.delete(previous.doc)
      }
      group.tabs.push({ doc: doc.id, transient })
    } else if (!transient) group.tabs.find(t => t.doc === doc.id)!.transient = false
    group.active = doc.id; this.emit()
  }
  newDocument(text = '', name = 'untitled.txt'): EditorDocument {
    const doc: EditorDocument = { id: crypto.randomUUID(), name, text, savedText: '', encoding: 'utf-8', bom: false, eol: '\n', mixedEol: false, savedFormat: JSON.stringify(['utf-8', false, '\n']), readOnly: false, writable: true, largeFile: false, disk: 'in-sync', version: 0, syncedVersion: -1, localVersion: -1 }
    this.documents.set(doc.id, doc); this.show(doc)
    if (text) this.changed(doc)
    return doc
  }
  edit(doc: EditorDocument, text: string): void { if (doc.readOnly || doc.mutating) return; doc.text = text; this.changed(doc) }
  changed(doc: EditorDocument): void {
    if (doc.version === 0 && doc.mixedEol) this.notify('normalizeEol')
    doc.version++; this.groups.forEach(g => g.tabs.filter(t => t.doc === doc.id).forEach(t => { t.transient = false }))
    this.schedule(doc); this.emit()
  }
  private schedule(doc: EditorDocument): void {
    const timers = this.timers.get(doc.id) ?? {}; this.timers.set(doc.id, timers)
    clearTimeout(timers.local); clearTimeout(timers.remote)
    timers.local = setTimeout(() => this.run(this.persistLocal(doc)), 300)
    timers.remote = setTimeout(() => this.run(this.flush(doc)), Math.max(doc.largeFile ? 30000 : 2000, doc.retryDelay ?? 0))
    timers.max ??= setTimeout(() => { timers.max = undefined; this.run(this.flush(doc)) }, Math.max(doc.largeFile ? 30000 : 10000, doc.retryDelay ?? 0))
  }
  async persistLocal(doc: EditorDocument): Promise<void> {
    if (!dirty(doc)) { if (doc.buffer) { const record = doc.buffer; doc.buffer = undefined; await this.buffers.remove(record) }; return }
    if (!doc.buffer) {
      doc.creating ??= (async () => {
        const header: BufferHeader = { format: 'buckyos.text-editor.buffer/1', bufferId: crypto.randomUUID(), source: doc.source,
          displayName: doc.name, base: doc.base, encoding: doc.encoding, bom: doc.bom, eol: doc.eol, language: doc.language,
          createdAt: Date.now(), updatedAt: Date.now(), origin: this.origin }
        doc.buffer = await this.buffers.create(header, doc.text, doc.version)
      })().finally(() => { doc.creating = undefined })
      await doc.creating
    }
    const record: BufferRecord = { ...doc.buffer!, header: { ...doc.buffer!.header, source: doc.source, base: doc.base, displayName: doc.name, encoding: doc.encoding, bom: doc.bom, eol: doc.eol, language: doc.language, updatedAt: Date.now() }, text: doc.text, version: doc.version }
    doc.buffer = record; await this.buffers.localWrite(record); doc.localVersion = record.version
  }
  async flush(doc: EditorDocument): Promise<void> {
    if (doc.flushing) { await doc.flushing; if (doc.syncedVersion === doc.version) return }
    doc.flushing = (async () => {
      do {
        await this.persistLocal(doc)
        if (!doc.buffer || !dirty(doc)) return
        const record = doc.buffer
        await this.buffers.flush(record); doc.syncedVersion = record.version
      } while (doc.syncedVersion !== doc.version)
      doc.error = undefined; doc.retryDelay = 0
    })().catch(e => { doc.error = String(e); doc.retryDelay = Math.min(60000, (doc.retryDelay || 1000) * 2); this.schedule(doc); throw e }).finally(() => { doc.flushing = undefined; this.emit() })
    await doc.flushing
  }
  private services(doc: EditorDocument) { return { store: this.store, flush: () => this.flush(doc), archive: this.recovery.archive.bind(this.recovery), annotate: this.recovery.annotate.bind(this.recovery), origin: this.origin } }
  ask(title: string, options: string[]): Promise<string> {
    if (this.choice) return Promise.resolve('cancel')
    return new Promise(resolve => { this.choice = { title, options, resolve: value => { this.choice = undefined; this.emit(); resolve(value) } }; this.emit() })
  }
  async save(doc = this.active, options: Parameters<typeof saveDocument>[2] = {}): Promise<boolean> {
    if (!doc || doc.saving || doc.mutating || doc.preview) return false
    if ((!doc.source || doc.readOnly || doc.source.kind === 'object-id') && !options.path) { this.chooseLocation(path => this.save(doc, { path }).then(() => {})); return false }
    doc.saving = true; doc.disk = 'saving'; this.emit()
    try {
      let text = doc.text
      if (this.settings.trimTrailingWhitespaceOnSave) text = text.replace(/[ \t]+$/gm, '')
      if (this.settings.ensureFinalNewline && !text.endsWith('\n')) text += '\n'
      if (text !== doc.text) { doc.text = text; this.changed(doc) }
      const version = doc.version; const format = formatKey(doc); const snapshot = { ...doc }
      // The draft is kept on this device before anything is sent; its server copy follows without holding the save up.
      const local = await this.persistLocal(doc).then(() => true, () => false)
      void this.flush(doc).catch(() => {})
      let saved: Awaited<ReturnType<typeof saveDocument>>
      try { saved = await saveDocument(snapshot, this.services(doc), { ...options, locale: this.locale }) }
      catch (e) {
        if (e instanceof SaveConflict) throw e
        const draft = doc.syncedVersion === doc.version ? 'synced' : local ? 'local' : 'none'
        throw new Problem(e instanceof SaveError && e.unknown ? 'saveUnknown' : 'saveFailed', { stage: e instanceof SaveError ? e.stage : 'prepare', path: e instanceof SaveError ? e.path : options.path ?? sourceKey(doc.source), category: errorCategory(e), unknown: e instanceof SaveError && e.unknown, draft }, () => this.save(doc, options))
      }
      // Committed: nothing below may turn this into "not saved".
      doc.source = saved.source; doc.base = { etag: saved.etag, size: saved.size }; doc.name = splitPath(contentRefString(saved.source)).name
      doc.savedText = snapshot.text; doc.savedFormat = format; doc.readOnly = false; doc.writable = true; doc.disk = 'in-sync'; doc.error = undefined
      if (doc.version === version && doc.buffer) {
        const record = doc.buffer; doc.buffer = undefined
        await this.buffers.remove(record).catch(() => { if (!doc.buffer) { doc.buffer = record; this.schedule(doc) } })
      } else if (dirty(doc)) { await this.persistLocal(doc).catch(() => {}); this.schedule(doc) }
      this.notify('saved'); return true
    } catch (e) {
      doc.disk = e instanceof SaveConflict && e.kind === 'deleted' ? 'deleted' : e instanceof SaveConflict && e.kind === 'lease-busy' ? 'lease-busy' : e instanceof SaveConflict ? 'changed' : 'error'
      if (!(e instanceof SaveConflict)) { doc.error = e instanceof Error ? e.message : String(e); if (e instanceof Problem) this.report(e); else this.notify(doc.error); return false }
      const answer = await this.ask(e.kind, e.kind === 'deleted' ? ['recreate', 'saveAs', 'cancel'] : e.kind === 'lease-busy' ? ['retry', 'saveAs', 'cancel'] : e.kind === 'copy-failed' ? ['overwrite', 'cancel'] : options.path && options.path !== sourceKey(doc.source) ? ['overwrite', 'overwriteCopy', 'saveAs', 'cancel'] : ['reload', 'overwrite', 'overwriteCopy', 'saveAs', 'cancel'])
      doc.saving = false
      if (answer === 'reload') await this.reload(doc)
      if (answer === 'saveAs') this.chooseLocation(path => this.save(doc, { path }).then(() => {}))
      if (['overwrite', 'overwriteCopy', 'retry', 'recreate'].includes(answer)) return this.save(doc, { ...options, overwrite: answer.startsWith('overwrite'), conflictCopy: answer === 'overwriteCopy', recreate: answer === 'recreate' })
      return false
    } finally { doc.saving = false; this.emit() }
  }
  private async replaceContents(doc: EditorDocument, action: () => Promise<void>): Promise<void> {
    if (doc.mutating) return
    doc.mutating = true; this.emit()
    try { await action() }
    finally { doc.mutating = false; if (dirty(doc)) this.schedule(doc); this.emit() }
  }
  async reload(doc: EditorDocument, encoding?: string): Promise<void> {
    if (!doc.source || doc.preview) return
    await this.replaceContents(doc, async () => {
      const file = await this.store.read(doc.source!); const decoded = decodeText(file.bytes, encoding)
      if (dirty(doc)) await archiveMine(doc, this.services(doc), 'conflict-reload')
      if (doc.buffer) { await this.buffers.remove(doc.buffer); doc.buffer = undefined }
      Object.assign(doc, decoded, { savedText: decoded.text, savedFormat: formatKey(decoded as SaveDocument), base: { etag: file.etag, size: file.size }, disk: 'in-sync', version: doc.version + 1, readOnly: decoded.readOnly || !doc.writable })
    })
  }
  convertEncoding(doc: EditorDocument, encoding: string): void {
    if (doc.mutating || !doc.writable) return
    doc.encoding = encoding; doc.bom = encoding !== 'utf-8'; doc.readOnly = false
    if (encoding === 'utf-8' && doc.text === doc.savedText) doc.savedFormat = 'unconverted'
    this.changed(doc)
  }
  /** One check at a time: calls that arrive while one runs share it, plus one more pass after it. */
  checkDisk(): Promise<void> {
    if (this.checking) { this.checkAgain = true; return this.checking }
    this.checking = (async () => { do { this.checkAgain = false; await this.checkDiskOnce() } while (this.checkAgain) })().finally(() => { this.checking = undefined })
    return this.checking
  }
  private async checkDiskOnce(): Promise<void> {
    let changed = false
    for (const doc of this.documents.values()) {
      if (!doc.source || doc.saving || doc.mutating || doc.preview) continue
      const before = `${doc.disk}|${doc.error}`
      try {
        const stat = await this.store.stat(doc.source)
        // A save whose new etag could not be read back: the size we wrote identifies our own version.
        if (doc.base && !doc.base.etag && stat.size === doc.base.size) doc.base = { etag: stat.etag, size: stat.size }
        else if (doc.base && stat.etag !== doc.base.etag) {
          if (!dirty(doc) && this.settings.autoReloadClean) { await this.reload(doc); changed = true }
          else doc.disk = 'changed'
        }
      } catch (e) { if (isMissing(e)) doc.disk = 'deleted'; else doc.error = String(e) }
      changed ||= before !== `${doc.disk}|${doc.error}`
    }
    if (this.session?.kind === 'container') await this.refreshItems()
    else if (changed) this.emit()
  }
  async refreshItems(): Promise<void> {
    const session = this.session
    const item = (source: TransferableContentRef, title: string): SessionItem => ({ source, title })
    if (session?.kind === 'list') this.items = session.items.map(i => item(i.source, i.title ?? sourceKey(i.source).split('/').pop()!))
    else if (session?.kind === 'container' && session.container.kind === 'cyfs-path') {
      const path = session.container.path
      this.items = (await this.store.list(path)).filter(e => e.target.kind === 'file').sort((a, b) => a.name.localeCompare(b.name)).map(e => item({ kind: 'cyfs-path', path: `${path.replace(/\/$/, '')}/${e.name}` }, e.name))
    } else this.items = []
    this.emit()
  }
  async refreshRecords(): Promise<void> { this.abandoned = (await this.buffers.list()).filter(b => ![...this.documents.values()].some(d => d.buffer?.key === b.key)); this.recoveries = await this.recovery.list(); this.emit() }
  async restoreBuffer(record: BufferRecord): Promise<void> {
    if (await this.buffers.active(record)) throw new Error('buffer-in-use')
    let doc: EditorDocument
    if (record.header.source) {
      try { await this.open({ requestId: crypto.randomUUID(), source: record.header.source }, false, false); doc = this.active! }
      catch (e) { if (!isMissing(e)) throw e; doc = this.newDocument('', record.header.displayName); doc.source = record.header.source; doc.disk = 'deleted' }
    } else doc = this.newDocument('', record.header.displayName)
    await this.replaceContents(doc, async () => {
    if (dirty(doc)) await archiveMine(doc, this.services(doc), 'taken-over')
    await this.recovery.archive(encodeText(record.text, { ...record.header, mixedEol: false }), { ...record.header, displayName: record.header.displayName, reason: 'taken-over', side: 'mine', baseEtag: record.header.base?.etag })
    Object.assign(doc, { text: record.text, encoding: record.header.encoding, bom: record.header.bom, eol: record.header.eol, language: record.header.language })
    if (doc.base?.etag !== record.header.base?.etag) doc.disk = doc.disk === 'deleted' ? 'deleted' : 'changed'
    if (doc.base?.etag !== record.header.base?.etag) doc.savedText = '\u0000'
    doc.base = record.header.base; doc.readOnly = !doc.writable; this.changed(doc)
    await this.flush(doc); await this.buffers.remove(record); this.panel = undefined; await this.refreshRecords()
    })
  }
  async discardBuffer(record: BufferRecord): Promise<void> {
    if (await this.buffers.active(record)) throw new Error('buffer-in-use')
    await this.recovery.archive(encodeText(record.text, { ...record.header, mixedEol: false }), { ...record.header, reason: 'discarded', side: 'mine', baseEtag: record.header.base?.etag })
    await this.buffers.remove(record); await this.refreshRecords()
  }
  async openRecovery(record: RecoveryRecord, restore = false): Promise<void> {
    const decoded = decodeText(await this.recovery.read(record))
    let doc: EditorDocument
    if (restore && record.entry.source) {
      try { await this.open({ requestId: crypto.randomUUID(), source: record.entry.source }, false, false); doc = this.active! }
      catch (e) { if (!isMissing(e)) throw e; doc = this.newDocument('', record.entry.displayName); doc.source = record.entry.source; doc.disk = 'deleted' }
    } else doc = this.newDocument('', record.entry.displayName)
    await this.replaceContents(doc, async () => {
      if (dirty(doc)) await archiveMine(doc, this.services(doc), 'taken-over')
      Object.assign(doc, decoded); doc.readOnly = !restore || !doc.writable; this.panel = undefined
      if (restore) { this.changed(doc); await this.flush(doc) }
      else { doc.savedText = doc.text; doc.savedFormat = formatKey(doc); doc.version++ }
    })
  }
  async close(doc: EditorDocument, group = this.group): Promise<boolean> {
    const elsewhere = this.groups.some(g => g !== group && g.tabs.some(t => t.doc === doc.id))
    if (!elsewhere && !await this.prepareClose([doc])) return false
    group.tabs = group.tabs.filter(t => t.doc !== doc.id); group.active = group.tabs.at(-1)?.doc
    if (!elsewhere) { this.documents.delete(doc.id); if (doc.buffer) this.buffers.release(doc.buffer); const timers = this.timers.get(doc.id); clearTimeout(timers?.local); clearTimeout(timers?.remote); clearTimeout(timers?.max) }
    if (!group.tabs.length && this.groups.length > 1) { this.groups = this.groups.filter(g => g !== group); this.activeGroup = this.groups[0].id; this.layout = 'single' }
    this.emit(); return true
  }
  /**
   * Closing never waits on the server before the user can choose. Changes are first written to this device
   * (IndexedDB, replayed to the server on the next start); the server copy gets a short head start. "Keep" is
   * offered only for what is actually safe: synced, or at least stored here — and then it says so.
   */
  async prepareClose(documents = [...this.documents.values()], reason = 'user'): Promise<boolean> {
    const changed = documents.filter(dirty)
    if (!changed.length) return true
    const local = (await Promise.all(changed.map(doc => this.persistLocal(doc).then(() => true, () => false)))).every(Boolean)
    await this.saveSnapshot().catch(() => {})
    const syncing = Promise.allSettled(changed.map(doc => this.flush(doc)))
    if (reason !== 'logout') await Promise.race([syncing, new Promise(resolve => setTimeout(resolve, CLOSE_SYNC_WAIT))])
    const synced = changed.every(doc => doc.syncedVersion === doc.version)
    if (reason === 'logout' || this.settings.closeKeepsChanges) {
      if (local || synced) return true
      if (reason === 'logout') { this.notify('closeUnsafe'); return false }
    }
    const answer = await this.ask(synced ? 'closeChanges' : local ? 'closeChangesLocal' : 'closeUnsafe', ['save', ...synced ? ['keep'] : local ? ['keepLocal'] : [], 'discard', 'cancel'])
    if (answer === 'cancel') return false
    if (answer === 'save') { for (const doc of changed) if (!await this.save(doc)) return false }
    if (answer === 'discard') {
      for (const doc of changed) {
        try { await archiveMine(doc, this.services(doc), doc.disk === 'deleted' ? 'deleted-close' : 'discarded') }
        catch (e) { this.report(new Problem('discardFailed', { stage: 'close', path: sourceKey(doc.source), category: errorCategory(e), draft: synced ? 'synced' : local ? 'local' : 'none' })); return false }
        if (doc.buffer) { await this.buffers.remove(doc.buffer).catch(() => {}); doc.buffer = undefined }
      }
    }
    return true
  }
  split(layout: Workspace['layout']): void {
    if (layout !== 'single' && this.groups.length < 2) this.groups.push({ id: crypto.randomUUID(), tabs: this.active ? [{ doc: this.active.id, transient: false }] : [], active: this.active?.id })
    if (layout === 'single' && this.groups.length > 1) { for (const group of this.groups.slice(1)) for (const tab of group.tabs) if (!this.groups[0].tabs.some(t => t.doc === tab.doc)) this.groups[0].tabs.push(tab); this.groups = [this.groups[0]]; this.activeGroup = this.groups[0].id }
    this.layout = layout; this.emit()
  }
  moveTab(doc: string, from: string, to: string, before?: string): void {
    const source = this.groups.find(g => g.id === from); const target = this.groups.find(g => g.id === to)
    const tab = source?.tabs.find(t => t.doc === doc); if (!tab || !target || !source) return
    source.tabs = source.tabs.filter(t => t !== tab); target.tabs = target.tabs.filter(t => t.doc !== doc)
    const at = target.tabs.findIndex(t => t.doc === before); target.tabs.splice(at < 0 ? target.tabs.length : at, 0, tab); target.active = doc; this.activeGroup = to; this.emit()
  }
  chooseLocation(action: (path: string) => Promise<void>): void { this.locationAction = action; this.panel = 'location'; this.emit() }
  async updateSettings(settings: Settings): Promise<void> { await this.settingsStore.save(settings); this.settings = settings; this.emit() }
}
