/* Client document management of one open rich text (design §3.3.7).
 *
 *   confirmed doc  only updates the backend accepted (mine and, from the change stream, others')
 *   working doc    confirmed.fork() + local edits not yet accepted; bound to the editor
 *
 * editor transaction → working doc → debounce → `richtext.apply_update` (everything the working doc
 * has and the confirmed doc lacks) → accepted → the same bytes go into the confirmed doc.
 * Remote updates go into both. A refused update never stays in the working doc: its AST is kept as
 * a recoverable draft and the working doc is forked again from the confirmed one.
 *
 * Replica session (design §6.3): the confirmed doc comes from the replica's confirmed layer and the
 * working doc is "confirmed + this device's pending updates". An editor update becomes a
 * `richtext.apply_update` submission of the replica — durable on this device once the commit resolves
 * `saved_locally` — and `base doc` tracks what has been handed over that way, so each submission
 * carries only what the previous ones did not. Text typed offline therefore survives closing the tab
 * as pending rows; nothing else is persisted for it. */

import { LoroDoc } from 'loro-crdt'
import { base64ToBytes, bytesToBase64, randomId } from '../api/ids'
import type { SubmissionEvent } from '../api/session'
import type { CollabState, CommitEvent, CommitOutcome, Operation } from '../api/types'
import { Emitter } from '../state/emitter'
import type { WorkspaceStore } from '../state/store'
import { astPlainText, saveDraft } from './drafts'
import { loadLoro } from './loro'
import { loroTextStyles } from './schema'

const DEBOUNCE_MS = 500
const RICHTEXT_BLOCK_OPS = new Set(['richtext.insert_blocks', 'richtext.replace_block', 'richtext.delete_blocks', 'richtext.move_block'])

export type CollabPhase = 'clean' | 'dirty' | 'sending' | 'blocked'

export class RichTextCollab {
  readonly entityId: string
  readonly editorId = randomId('ed')
  readonly lineageId: string
  private confirmedDoc: LoroDoc
  private workingDoc: LoroDoc
  /** What the session already has: the confirmed doc itself (direct), or confirmed + locally saved updates (replica). */
  private baseDoc: LoroDoc
  private readonly replicaMode: boolean
  private rebuilding = false
  private readonly store: WorkspaceStore
  private readonly emitter = new Emitter()
  private readonly unsubscribe: (() => void)[] = []
  private timer: number | null = null
  private sending = false
  private inflight: Promise<void> | null = null
  private composing = false
  private deferredForWorking: Uint8Array[] = []
  private closed = false
  private currentGeneration = 0
  private currentPhase: CollabPhase = 'clean'
  /** Set while commits are refused for a reason that keeps the edits valid (write lock missing). */
  private blockedReason: string | null = null
  /** Reads the editor document for the draft kept on a refusal. */
  editorJson: (() => unknown) | null = null
  readonly subscribe = this.emitter.subscribe

  private constructor(store: WorkspaceStore, entityId: string, lineageId: string, docs: { confirmed: LoroDoc; working: LoroDoc; base: LoroDoc }) {
    this.store = store
    this.entityId = entityId
    this.lineageId = lineageId
    this.confirmedDoc = docs.confirmed
    this.workingDoc = docs.working
    this.baseDoc = docs.base
    this.replicaMode = store.session.offline !== null
  }

  private static documents(store: WorkspaceStore, state: CollabState): { confirmed: LoroDoc; working: LoroDoc; base: LoroDoc } {
    const styles = loroTextStyles(store.schemaDef)
    const confirmed = new LoroDoc()
    confirmed.configTextStyle(styles)
    confirmed.import(base64ToBytes(state.snapshot))
    const working = confirmed.fork()
    working.configTextStyle(styles)
    if (store.session.offline === null) return { confirmed, working, base: confirmed }
    // this device's pending updates are part of the working document (they are already saved in the replica)
    for (const update of state.pending_updates ?? []) working.import(base64ToBytes(update))
    const base = working.fork()
    base.configTextStyle(styles)
    return { confirmed, working, base }
  }

  static async open(store: WorkspaceStore, entityId: string): Promise<RichTextCollab> {
    await loadLoro()
    // Subscribe before reading the state: events that race with the read are imported afterwards
    // (CRDT imports are idempotent, so overlap is harmless).
    const early: CommitEvent[] = []
    let collab: RichTextCollab | null = null
    const off = store.session.subscribeChanges((event) => { if (collab) collab.onRemote(event); else early.push(event) })
    try {
      const state = await store.session.getCollabState(entityId)
      const created = new RichTextCollab(store, entityId, state.lineage_id, RichTextCollab.documents(store, state))
      collab = created
      collab.unsubscribe.push(off)
      if (store.session.offline) collab.unsubscribe.push(store.session.offline.subscribeSubmissions((event) => created.onSubmission(event)))
      collab.watchWorking()
      early.forEach((event) => collab?.onRemote(event))
      return collab
    } catch (error) {
      off()
      throw error
    }
  }

  get working(): LoroDoc { return this.workingDoc }
  get confirmed(): LoroDoc { return this.confirmedDoc }
  /** Bumps whenever the working doc object is replaced: the editor must be rebuilt on it. */
  snapshot = (): string => `${this.currentGeneration}:${this.currentPhase}:${this.blockedReason ?? ''}`
  get generation(): number { return this.currentGeneration }
  get phase(): CollabPhase { return this.currentPhase }
  get blocked(): string | null { return this.blockedReason }

  private get editId() { return `rt:${this.entityId}` }
  private get label() { return `富文本 ${this.entityId}` }

  private setPhase(phase: CollabPhase) {
    if (phase === this.currentPhase) return
    this.currentPhase = phase
    this.emitter.emit()
  }

  private watchWorking() {
    const doc = this.workingDoc
    const off = doc.subscribeLocalUpdates(() => {
      if (doc !== this.workingDoc) return
      this.markDirty()
    })
    this.unsubscribe.push(off)
  }

  private markDirty() {
    if (this.blockedReason === null) {
      this.store.edits.set({ id: this.editId, label: this.label, state: 'unsaved' })
      if (!this.sending) this.setPhase('dirty')
    }
    this.schedule(DEBOUNCE_MS)
  }

  private schedule(delay: number) {
    if (this.timer !== null) window.clearTimeout(this.timer)
    this.timer = window.setTimeout(() => { this.timer = null; void this.flush() }, delay)
  }

  private hasUnsent(): boolean {
    return this.workingDoc.oplogVersion().compare(this.baseDoc.oplogVersion()) !== 0
  }

  private preview(): string {
    let ast: unknown = null
    try { ast = this.editorJson ? this.editorJson() : null } catch { ast = null }
    const text = astPlainText(ast).trim()
    return text.length > 400 ? `${text.slice(0, 400)}…` : text
  }

  /** A pending update of this rich text was refused by the backend (or dropped out at a rebase). */
  private onSubmission(event: SubmissionEvent) {
    if (this.closed || event.meta?.kind !== 'richtext' || event.meta.entity_id !== this.entityId) return
    if (event.state !== 'conflict' && event.state !== 'rejected') return
    if (this.rebuilding) return // updates build on each other: one refusal takes the later ones with it, one draft covers them
    const result = (event.result ?? {}) as { code?: string; detail?: string; errors?: { detail?: string }[] }
    this.reject(result.code ?? 'UNKNOWN', result.errors?.[0]?.detail ?? result.detail ?? '', true)
  }

  /** IME composition: remote updates would rebuild the editor document under the composition. */
  setComposing(composing: boolean) {
    this.composing = composing
    if (!composing) {
      const deferred = this.deferredForWorking.splice(0)
      for (const bytes of deferred) this.workingDoc.import(bytes)
      if (this.hasUnsent()) this.schedule(DEBOUNCE_MS)
    }
  }

  private onRemote(event: CommitEvent) {
    for (const op of event.ops ?? []) {
      if (op.entity_id !== this.entityId) continue
      const encoded = op.op === 'richtext.apply_update' ? op.update : RICHTEXT_BLOCK_OPS.has(op.op) ? op.server_update : undefined
      if (typeof encoded !== 'string') continue
      const bytes = base64ToBytes(encoded)
      this.confirmedDoc.import(bytes)
      if (this.baseDoc !== this.confirmedDoc) this.baseDoc.import(bytes)
      if (this.composing) this.deferredForWorking.push(bytes)
      else this.workingDoc.import(bytes)
    }
  }

  /** Send what the working doc has beyond the confirmed doc. Safe to call at any time. */
  flush(): Promise<void> {
    if (this.closed || this.sending) return this.inflight ?? Promise.resolve()
    const run = this.flushNow().finally(() => { if (this.inflight === run) this.inflight = null })
    this.inflight = run
    return run
  }

  private async flushNow(): Promise<void> {
    if (this.composing) { this.schedule(DEBOUNCE_MS); return }
    if (!this.hasUnsent()) {
      if (this.blockedReason === null) this.setPhase('clean')
      return
    }
    this.sending = true
    this.setPhase('sending')
    const doc = this.workingDoc
    const base = this.baseDoc
    const update = doc.export({ mode: 'update', from: base.oplogVersion() })
    const operation: Operation = { op: 'richtext.apply_update', entity_id: this.entityId, lineage_id: this.lineageId, update: bytesToBase64(update) }
    let outcome: CommitOutcome
    try {
      outcome = await this.store.session.commit([operation], {
        message: '编辑富文本',
        meta: this.replicaMode ? { editId: this.editId, label: this.label, kind: 'richtext', entity_id: this.entityId, undoable: false, mine: this.preview(), hasMine: true } : undefined,
      })
    } catch (error) {
      outcome = { status: 'unknown', idempotency_key: '', detail: error instanceof Error ? error.message : String(error) }
    }
    this.sending = false
    if (this.closed) return
    if (outcome.status === 'saved_locally') {
      // durable in the replica database; the store's submission listener shows 已保存到本设备 → 已提交
      if (base === this.baseDoc) base.import(update)
      this.store.noteUnsaved(this.editId, null)
      this.blockedReason = null
      if (doc === this.workingDoc && this.hasUnsent()) {
        this.setPhase('dirty')
        this.schedule(0)
      } else {
        this.setPhase('clean')
      }
      return
    }
    if (outcome.status === 'storage_failed') {
      // NOT saved: the text is only in this editor. Say so, keep it exportable, try again later.
      let ast: unknown = null
      try { ast = this.editorJson ? this.editorJson() : null } catch { ast = null }
      this.store.noteUnsaved(this.editId, { label: this.label, operations: [operation], mine: (ast ?? null) as never, reason: outcome.detail, at: new Date().toISOString() })
      this.store.edits.set({ id: this.editId, label: this.label, state: 'unsaved', storageFailed: true, detail: `没有保存：${outcome.detail}。内容仍在编辑器中，可以导出。` })
      this.setPhase('dirty')
      this.schedule(5000)
      return
    }
    if (outcome.status === 'accepted') {
      this.confirmedDoc.import(update)
      if (this.baseDoc !== this.confirmedDoc) this.baseDoc.import(update)
      this.blockedReason = null
      if (doc === this.workingDoc && this.hasUnsent()) {
        this.setPhase('dirty')
        this.schedule(0)
      } else {
        this.store.edits.set({ id: this.editId, label: this.label, state: 'committed' })
        this.setPhase('clean')
      }
      return
    }
    if (outcome.status === 'unknown') {
      // Not known to be refused: keep the edits, try again. A superset of an already accepted update
      // is harmless (CRDT import is idempotent), so no same-key bookkeeping is needed here.
      this.store.edits.set({ id: this.editId, label: this.label, state: 'unsaved', detail: `尚未送达后台：${outcome.detail}` })
      this.setPhase('dirty')
      this.schedule(3000)
      return
    }
    const code = outcome.code
    const detail = outcome.status === 'rejected' ? (outcome.errors?.[0]?.detail ?? outcome.detail ?? '') : ''
    if (code === 'BASE_UNKNOWN') {
      // The backend lacks operations this update builds on: catch up, then send again.
      await this.store.session.whenApplied(this.store.session.info().head_seq)
      this.setPhase('dirty')
      this.schedule(DEBOUNCE_MS)
      return
    }
    if (code.startsWith('LOCK_')) {
      // The edits are valid, only the lock is missing: keep them in the working doc as pending.
      this.blockedReason = code === 'LOCK_HELD' ? '写锁由他人持有' : '写锁已失去或尚未取得'
      this.store.locks.noteLost(this.entityId, this.blockedReason)
      this.store.edits.set({ id: this.editId, label: this.label, state: 'needs_attention', code, detail: `${this.blockedReason}，修改保留在本窗口；重新取得写锁后会再次提交。` })
      this.currentPhase = 'blocked'
      this.emitter.emit()
      return
    }
    this.reject(code, detail)
  }

  /** The lock was (re)acquired: pending edits may go out again. */
  resume() {
    if (this.blockedReason === null) return
    this.blockedReason = null
    this.currentPhase = 'dirty'
    this.emitter.emit()
    this.schedule(0)
  }

  /** Refused for good: keep the working AST as a draft, rebuild the working doc from the confirmed one.
   * `pendingRow`: the refusal concerns a pending submission of the replica, which stays listed until dismissed. */
  private reject(code: string, detail: string, pendingRow = false) {
    let ast: unknown
    try { ast = this.editorJson ? this.editorJson() : this.workingDoc.getMap('doc').toJSON() } catch { ast = this.workingDoc.getMap('doc').toJSON() }
    const draft = {
      draft_id: randomId('d'), workspace_id: this.store.session.workspaceId, entity_id: this.entityId,
      saved_at: new Date().toISOString(), reason: `${code}${detail ? `: ${detail}` : ''}`, ast,
    }
    const kept = saveDraft(draft)
    // a replica session also keeps the draft in the replica database, where the export finds it
    void this.store.session.offline?.saveDraft({ draft_id: draft.draft_id, entity_id: this.entityId, kind: 'richtext_refused', base: { reason: draft.reason }, content: (ast ?? null) as never, updated_at: draft.saved_at }).catch(() => undefined)
    const previous = this.store.edits.get(this.editId)
    this.store.edits.set({
      id: this.editId, label: this.label, state: 'needs_attention', code,
      pendingKey: pendingRow ? previous?.pendingKey : undefined, mine: previous?.mine, hasMine: previous?.hasMine,
      detail: kept
        ? `修改未被接受（${code}${detail ? `：${detail}` : ''}），已保留为草稿；编辑器已恢复到已确认内容。`
        : `修改未被接受（${code}${detail ? `：${detail}` : ''}），且草稿无法写入本机存储；编辑器已恢复到已确认内容。`,
    })
    this.store.notify('error', `富文本 ${this.entityId} 的修改未被接受（${code}）${kept ? '，已保留为草稿' : '，草稿保存失败'}。`)
    if (!this.replicaMode) {
      const working = this.confirmedDoc.fork()
      working.configTextStyle(loroTextStyles(this.store.schemaDef))
      this.adoptDocuments({ confirmed: this.confirmedDoc, working, base: this.confirmedDoc })
      return
    }
    // replica: confirmed layer + whatever of this device is still pending
    const generation = this.currentGeneration
    this.rebuilding = true
    void this.store.session.getCollabState(this.entityId).then((state) => {
      this.rebuilding = false
      if (this.closed || generation !== this.currentGeneration) return
      this.adoptDocuments(RichTextCollab.documents(this.store, state))
    }, (error: unknown) => {
      this.rebuilding = false
      this.store.notify('error', `无法从本机副本重建富文本 ${this.entityId}：${error instanceof Error ? error.message : String(error)}`)
    })
  }

  private adoptDocuments(docs: { confirmed: LoroDoc; working: LoroDoc; base: LoroDoc }) {
    this.confirmedDoc = docs.confirmed
    this.workingDoc = docs.working
    this.baseDoc = docs.base
    this.deferredForWorking = []
    this.currentGeneration += 1
    this.watchWorking()
    this.currentPhase = 'clean'
    this.emitter.emit()
  }

  async close() {
    this.composing = false
    // Whatever is still local goes out before the documents are dropped.
    for (let round = 0; round < 3; round++) {
      if (this.timer !== null) { window.clearTimeout(this.timer); this.timer = null }
      if (this.inflight) await this.inflight
      if (this.blockedReason !== null || !this.hasUnsent()) break
      await this.flush()
    }
    if (this.timer !== null) { window.clearTimeout(this.timer); this.timer = null }
    this.closed = true
    this.unsubscribe.splice(0).forEach((off) => off())
  }
}
