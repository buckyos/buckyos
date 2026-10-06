/* Everything one open Workspace shares between its views: the session, invalidation counters fed by
 * the change stream, save states, the UndoCoordinator, the lock manager and user-visible notices. */

import type { Schema } from 'prosemirror-model'
import type { ReadOk, SubmissionEvent, WorkspaceSession } from '../api/session'
import { ServiceFailure } from '../api/client'
import { testHooks } from '../api/testHooks'
import type { CommitConflict, CommitOutcome, CommitRejected, EntityEnvelope, Json, Operation, Placement } from '../api/types'
import type { PendingMeta, PendingRow } from '../offline/protocol'
import { richTextSchemaDef, type AiwsCore } from '../api/wasm'
import { buildRichTextSchema, type RichTextSchemaDef } from '../richtext/schema'
import { EditStore } from './edits'
import { Emitter, VersionMap } from './emitter'
import { FreshnessService } from './freshness'
import { LockManager } from './locks'
import { OutlineModel } from './outline'
import { UndoCoordinator } from './undo'
import { UserWorkState } from './userState'
import { WishService } from '../ui/wish/WishService'

const STRUCTURAL = new Set(['created', 'deleted', 'restored', 'renamed', 'moved', 'placed', 'view'])
/** A remote move of a Block this window placed within this many ms is reported as a layout conflict (D1). */
const LAYOUT_INTENT_MS = 20_000

export interface Notice { id: number; kind: 'info' | 'error'; text: string; action?: { label: string; run: () => void } }

export interface SubmitOptions {
  /** Key of the edit in the save-state list; a later edit of the same target replaces the earlier entry. */
  editId: string
  label: string
  operations: Operation[]
  /** The user's input, kept in the entry while the edit is not committed. */
  mine?: Json
  hasMine?: boolean
  /** Push the accepted commit on the undo stack (default true). */
  undoable?: boolean
  /** Read-set preconditions guarding the commit (wish application, §7.2). */
  preconditions?: unknown[]
  origin?: 'human' | 'agent' | 'program'
}

/** An input that is saved nowhere but in this window's memory (storage failure, unknown result): exportable. */
export interface UnsavedInput { label: string; operations: Operation[]; mine?: Json; reason: string; at: string }

export function saveJsonFile(fileName: string, data: unknown) {
  const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' }))
  const link = document.createElement('a')
  link.href = url
  link.download = fileName
  document.body.appendChild(link)
  link.click()
  link.remove()
  window.setTimeout(() => URL.revokeObjectURL(url), 10_000)
}

export class WorkspaceStore {
  readonly session: WorkspaceSession
  readonly core: AiwsCore
  readonly schemaDef: RichTextSchemaDef
  readonly pmSchema: Schema
  readonly versions = new VersionMap()
  readonly edits = new EditStore()
  readonly undo: UndoCoordinator
  readonly locks: LockManager
  /** The two trees, maintained incrementally from the change stream (phase two §9.3). */
  readonly outline: OutlineModel
  readonly userState: UserWorkState
  readonly freshness: FreshnessService
  /** Wish passes and application (phase two §7). */
  readonly wish: WishService
  /** Last placement this window submitted per Block, for the layout-conflict notice (D1). */
  private readonly layoutIntents = new Map<string, { placement: Placement; orderKey?: string; parentId?: string; at: number }>()
  private notices: Notice[] = []
  private noticeId = 0
  private readonly noticeEmitter = new Emitter()
  private readonly unsubscribe: () => void
  private readonly offSubmissions: (() => void) | null = null
  private readonly offStatus: () => void
  private lockPoll: number | null = null
  /** Inputs that exist only in memory, by edit id. */
  private unsaved = new Map<string, UnsavedInput>()
  private readonly unsavedEmitter = new Emitter()
  readonly subscribeUnsaved = this.unsavedEmitter.subscribe
  readonly subscribeNotices = this.noticeEmitter.subscribe

  constructor(session: WorkspaceSession, core: AiwsCore) {
    this.session = session
    this.core = core
    this.schemaDef = richTextSchemaDef(core)
    this.pmSchema = buildRichTextSchema(this.schemaDef)
    this.undo = new UndoCoordinator(session)
    this.locks = new LockManager(session)
    this.outline = new OutlineModel(
      () => session.outline(),
      async (ids) => (await session.readMany(ids.map((entity_id) => ({ entity_id })))).map((entry) => ('error' in entry ? null : (entry as EntityEnvelope))),
      session.principal,
    )
    this.outline.onRemoteLayoutChange = (entityId, author, placement) => this.onRemoteLayoutChange(entityId, author, placement)
    this.userState = new UserWorkState(session)
    this.freshness = new FreshnessService(session)
    this.wish = new WishService(this)
    void this.userState.init()
    this.unsubscribe = session.subscribeChanges((event) => {
      const keys = new Set<string>(['any'])
      for (const touched of event.touched ?? []) {
        keys.add(`e:${touched.entity_id}`)
        if (STRUCTURAL.has(touched.change)) keys.add('outline')
      }
      // the outline is patched from the event; only an uninterpretable event re-reads it
      this.outline.applyEvent(event)
      if ((event.touched ?? []).length > 0) this.freshness.invalidate()
      this.versions.bump(keys)
    })
    const offline = session.offline
    const hooks = testHooks()
    if (hooks) {
      hooks.replica = offline?.test ?? undefined
      hooks.outline = () => this.outline.all()
      hooks.commits = 0
    }
    if (offline) {
      // pending submissions of earlier runs of this replica: they are saved on this device, show them again
      for (const row of offline.pending()) this.showPending(row.idempotency_key, row.state, row.result, row.meta, false)
      this.offSubmissions = offline.subscribeSubmissions((event) => this.onSubmission(event))
    }
    this.offStatus = session.subscribeStatus(() => this.onStatus())
    this.onStatus()
  }

  unsavedSnapshot = (): ReadonlyMap<string, UnsavedInput> => this.unsaved

  /** Reads of many Blocks issued in the same tick go out as one `doc.read` batch (phase two §9.3 rule 6). */
  private batch: { ids: Map<string, { resolve: (value: ReadOk<unknown>) => void; reject: (error: Error) => void }[]>; timer: number } | null = null
  readBatched<C>(entityId: string): Promise<ReadOk<C>> {
    return new Promise<ReadOk<unknown>>((resolve, reject) => {
      if (!this.batch) {
        this.batch = { ids: new Map(), timer: window.setTimeout(() => void this.flushBatch(), 8) }
      }
      const waiters = this.batch.ids.get(entityId) ?? []
      waiters.push({ resolve, reject })
      this.batch.ids.set(entityId, waiters)
      if (this.batch.ids.size >= 200) { window.clearTimeout(this.batch.timer); void this.flushBatch() }
    }) as Promise<ReadOk<C>>
  }

  private async flushBatch() {
    const batch = this.batch
    this.batch = null
    if (!batch) return
    const ids = [...batch.ids.keys()]
    try {
      const results = await this.session.readMany(ids.map((entity_id) => ({ entity_id })))
      results.forEach((result, index) => {
        const waiters = batch.ids.get(ids[index]) ?? []
        if ('error' in result) {
          const error = new ServiceFailure({ code: result.error.code, detail: result.error.detail })
          for (const waiter of waiters) waiter.reject(error)
        } else {
          for (const waiter of waiters) waiter.resolve(result)
        }
      })
    } catch (error) {
      for (const waiters of batch.ids.values()) for (const waiter of waiters) waiter.reject(error instanceof Error ? error : new Error(String(error)))
    }
  }

  private setUnsaved(editId: string, input: UnsavedInput | null) {
    if (!input && !this.unsaved.has(editId)) return
    const next = new Map(this.unsaved)
    if (input) next.set(editId, input)
    else next.delete(editId)
    this.unsaved = next
    this.unsavedEmitter.emit()
  }

  /** An editor keeps content that is not saved anywhere (the replica database refused it): make it exportable. */
  noteUnsaved(editId: string, input: UnsavedInput | null) {
    this.setUnsaved(editId, input)
  }

  /** Everything of this window that the backend does not have: unsaved inputs (memory) and, in a replica
   * session, the pending submissions and drafts as stored in the replica database. Saved as a JSON file. */
  async exportLocal(): Promise<void> {
    const session = this.session
    let replica: unknown = null
    if (session.offline) {
      try { replica = await session.offline.exportLocal() } catch (error) { replica = { error: `无法读取本机副本：${error instanceof Error ? error.message : String(error)}` } }
    }
    saveJsonFile(`aiworkspace-local-${session.workspaceId}-${new Date().toISOString().replace(/[:.]/g, '-')}.json`, {
      format: 'buckyos.aiworkspace.local-export/1',
      exported_at: new Date().toISOString(),
      workspace_id: session.workspaceId,
      principal: session.principal,
      epoch: session.info().epoch,
      unsaved: [...this.unsaved.entries()].map(([edit_id, input]) => ({ edit_id, ...input })),
      replica,
    })
  }

  private refusalDetail(result: Json | null): { code: string; detail: string; theirs?: Json; hasTheirs: boolean; currentRev?: number } {
    const outcome = (result ?? {}) as { status?: string; code?: string; sub_code?: string; detail?: string; conflicts?: CommitConflict['conflicts']; errors?: CommitRejected['errors'] }
    const code = typeof outcome.code === 'string' ? outcome.code : 'UNKNOWN'
    if (outcome.status === 'conflict') {
      const first = outcome.conflicts?.[0]
      return {
        code, detail: `冲突（${code}）：${code === 'TARGET_DELETED' ? '目标在你编辑期间已被删除' : '此内容在你编辑期间已被修改'}`,
        theirs: first?.current_value, hasTheirs: first !== undefined && 'current_value' in first, currentRev: first?.current_rev,
      }
    }
    const first = outcome.errors?.[0]
    const text = first?.detail ?? outcome.detail
    return { code, detail: `未被接受（${code}${outcome.sub_code ? `/${outcome.sub_code}` : ''}）${text ? `：${text}` : ''}`, hasTheirs: false }
  }

  /** Reflect one pending submission of the replica in the save-state list. */
  private showPending(key: string, state: PendingRow['state'], result: Json | null, meta: PendingMeta | null, onlyIfOwned: boolean) {
    if (!meta?.editId) return
    const current = this.edits.get(meta.editId)
    if (onlyIfOwned && current && current.pendingKey !== undefined && current.pendingKey !== key && current.state !== 'committed') return
    const base = { id: meta.editId, label: meta.label ?? meta.editId, mine: meta.mine, hasMine: meta.hasMine, pendingKey: key }
    if (state === 'conflict' || state === 'rejected') {
      const refusal = this.refusalDetail(result)
      this.edits.set({ ...base, state: 'needs_attention', code: refusal.code, detail: refusal.detail, theirs: refusal.theirs, hasTheirs: refusal.hasTheirs, currentRev: refusal.currentRev })
    } else {
      this.edits.set({
        ...base, state: 'saved_locally',
        detail: state === 'unknown' ? '已发送但没有收到应答：重新连接后按幂等键确认，不会重复应用' : state === 'blocked_asset' ? '等待依赖的资产上传' : '已保存到本设备，等待发送到后台',
      })
    }
  }

  private onSubmission(event: SubmissionEvent) {
    const editId = event.meta?.editId
    if (event.state === 'accepted') {
      this.undo.resolvePending(event.key, event.commit_id ?? null)
      if (editId && this.edits.get(editId)?.pendingKey === event.key) this.edits.set({ id: editId, label: event.meta?.label ?? editId, state: 'committed' })
      return
    }
    if (event.state === 'discarded') {
      if (editId && this.edits.get(editId)?.pendingKey === event.key) this.edits.remove(editId)
      return
    }
    if (event.state === 'conflict' || event.state === 'rejected') this.undo.resolvePending(event.key, null)
    this.showPending(event.key, event.state, event.result, event.meta, true)
  }

  /** Synchronisation stopped for good: what is still queued will not be sent, so it needs the user's attention. */
  private onStatus() {
    const status = this.session.status()
    if (status.kind !== 'stopped' || !this.session.offline) return
    for (const row of this.session.offline.pending()) {
      const editId = row.meta?.editId
      if (!editId || this.edits.get(editId)?.state === 'needs_attention') continue
      this.edits.set({ id: editId, label: row.meta?.label ?? editId, state: 'needs_attention', code: status.code, mine: row.meta?.mine, hasMine: row.meta?.hasMine, pendingKey: row.idempotency_key, detail: `不会再自动发送（${status.code}）：${status.detail}` })
    }
  }

  /** "知道了 / 放弃我的输入": the entry goes away, and so does the refused submission it stands for. */
  async dismissEdit(editId: string): Promise<void> {
    const entry = this.edits.get(editId)
    if (entry?.pendingKey && this.session.offline) {
      try {
        await this.session.offline.discardPending(entry.pendingKey, { force: entry.state === 'needs_attention' })
      } catch (error) {
        this.notify('error', `无法移除「${entry.label}」：${error instanceof Error ? error.message : String(error)}`)
        return
      }
    }
    this.setUnsaved(editId, null)
    this.edits.remove(editId)
  }

  noticeSnapshot = (): readonly Notice[] => this.notices

  notify(kind: Notice['kind'], text: string, action?: Notice['action']) {
    this.noticeId += 1
    this.notices = [...this.notices, { id: this.noticeId, kind, text, action }]
    this.noticeEmitter.emit()
  }

  /** Remember what this window just placed, so a remote overwrite can be reported and re-applied (D1). */
  noteLayoutIntent(entityId: string, placement: Placement, orderKey?: string, parentId?: string) {
    this.layoutIntents.set(entityId, { placement, orderKey, parentId, at: Date.now() })
  }

  private onRemoteLayoutChange(entityId: string, author: string | null, placement: Placement | undefined) {
    const intent = this.layoutIntents.get(entityId)
    if (!intent || Date.now() - intent.at > LAYOUT_INTENT_MS) return
    if (placement && JSON.stringify(placement) === JSON.stringify(intent.placement)) return
    this.layoutIntents.delete(entityId)
    const entity = this.outline.get(entityId)
    const label = entity?.title ?? entity?.name ?? entityId
    this.notify('info', `「${label}」的位置已被${author ? ` ${author}` : '他人'}修改（后到者生效）。`, {
      label: '重新应用我的位置',
      run: () => {
        const current = this.outline.get(entityId)
        const operations: Operation[] = [current && intent.parentId && current.parent_id !== intent.parentId
          ? { op: 'tree.move', entity_id: entityId, new_parent_id: intent.parentId, order_key: intent.orderKey ?? current.order_key, placement: intent.placement }
          : { op: 'tree.place', entity_id: entityId, placement: intent.placement, ...(intent.orderKey ? { order_key: intent.orderKey } : {}) }]
        this.noteLayoutIntent(entityId, intent.placement, intent.orderKey, intent.parentId)
        void this.submit({ editId: `layout:${entityId}`, label: `重新应用位置 ${label}`, operations })
      },
    })
  }

  dismissNotice(id: number) {
    this.notices = this.notices.filter((notice) => notice.id !== id)
    this.noticeEmitter.emit()
  }

  /** Lock holders are not part of the change stream: while some entity requires a lock, re-read the outline now and then. */
  setLockPolling(active: boolean) {
    if (active && this.lockPoll === null) this.lockPoll = window.setInterval(() => { this.versions.bump(['outline']); void this.outline.reload().catch(() => undefined) }, 5000)
    if (!active && this.lockPoll !== null) { window.clearInterval(this.lockPoll); this.lockPoll = null }
  }

  /** Commit with save-state tracking. The input is never dropped: whatever is not accepted stays in the edit list. */
  async submit(options: SubmitOptions): Promise<CommitOutcome> {
    const { editId, label, mine, hasMine } = options
    const previous = this.edits.get(editId)
    if (previous?.state === 'needs_attention' && previous.pendingKey && this.session.offline) {
      // a new input for the same target replaces the refused one
      try { await this.session.offline.discardPending(previous.pendingKey, { force: true }) } catch { /* it stays listed in the export */ }
    }
    this.edits.set({ id: editId, label, state: 'unsaved', mine, hasMine })
    let outcome: CommitOutcome
    const hooks = testHooks()
    if (hooks) hooks.commits = (hooks.commits ?? 0) + 1
    try {
      outcome = await this.session.commit(options.operations, { message: label, preconditions: options.preconditions, origin: options.origin, meta: { editId, label, mine, hasMine, undoable: options.undoable !== false } })
    } catch (error) {
      outcome = { status: 'unknown', idempotency_key: '', detail: error instanceof Error ? error.message : String(error) }
    }
    return this.settle(options, outcome)
  }

  /** Resend an edit whose result is unknown (same idempotency key, same content). */
  async retry(options: SubmitOptions, idempotencyKey: string): Promise<CommitOutcome> {
    return this.settle(options, await this.session.retryUnknown(idempotencyKey))
  }

  private async settle(options: SubmitOptions, outcome: CommitOutcome): Promise<CommitOutcome> {
    const { editId, label, mine, hasMine } = options
    if (outcome.status !== 'storage_failed' && outcome.status !== 'unknown') this.setUnsaved(editId, null)
    if (outcome.status === 'saved_locally') {
      // the Worker answered after its transaction committed: durable on this device, not yet on the backend
      const now = this.session.offline?.pending().find((row) => row.idempotency_key === outcome.idempotency_key)
      if (now) this.showPending(now.idempotency_key, now.state, now.result, now.meta, false)
      else if (this.edits.get(editId)?.state === 'unsaved') this.edits.set({ id: editId, label, state: 'committed' }) // sent and settled already
      if (options.undoable !== false && now) this.undo.pushPending(outcome.idempotency_key, label)
    } else if (outcome.status === 'storage_failed') {
      this.setUnsaved(editId, { label, operations: options.operations, mine, reason: outcome.detail, at: new Date().toISOString() })
      this.edits.set({ id: editId, label, state: 'unsaved', mine, hasMine, storageFailed: true, detail: `没有保存：${outcome.detail}。输入仍在本窗口内存中，可以导出。` })
    } else if (outcome.status === 'accepted') {
      this.edits.set({ id: editId, label, state: 'committed' })
      if (options.undoable !== false) this.undo.pushCommit(outcome.commit_id, label)
      await this.session.whenApplied(outcome.seq)
    } else if (outcome.status === 'conflict') {
      const first = outcome.conflicts?.[0]
      this.edits.set({
        id: editId, label, state: 'needs_attention', code: outcome.code, mine, hasMine,
        detail: `冲突（${outcome.code}）：此内容在你编辑期间已被修改`,
        theirs: first?.current_value, hasTheirs: first !== undefined && 'current_value' in first, currentRev: first?.current_rev,
      })
    } else if (outcome.status === 'rejected') {
      const first = outcome.errors?.[0]
      if (outcome.code.startsWith('LOCK_')) {
        const locks = (first?.data?.locks ?? []) as { entity_id?: string; holder?: string }[]
        for (const lock of locks) if (lock.entity_id) this.locks.noteLost(lock.entity_id, outcome.code === 'LOCK_HELD' ? `写锁由 ${lock.holder ?? '他人'} 持有` : '写锁已失去或尚未取得')
      }
      this.edits.set({
        id: editId, label, state: 'needs_attention', code: outcome.code, mine, hasMine,
        detail: `未被接受（${outcome.code}${outcome.sub_code ? `/${outcome.sub_code}` : ''}）${first?.detail ?? outcome.detail ? `：${first?.detail ?? outcome.detail}` : ''}`,
      })
    } else {
      this.setUnsaved(editId, { label, operations: options.operations, mine, reason: `结果未知：${outcome.detail}`, at: new Date().toISOString() })
      this.edits.set({ id: editId, label, state: 'unsaved', mine, hasMine, detail: `结果未知：${outcome.detail}`, unknownKey: outcome.idempotency_key || undefined })
    }
    return outcome
  }

  private disposeTimer: number | null = null
  private disposed = false

  /** Mount/unmount pairing that survives React's development double-mount: a release is undone by a retain that follows at once. */
  retain() {
    if (this.disposeTimer !== null) { window.clearTimeout(this.disposeTimer); this.disposeTimer = null }
  }

  releaseSoon() {
    if (this.disposed || this.disposeTimer !== null) return
    this.disposeTimer = window.setTimeout(() => { void this.dispose() }, 200)
  }

  /** Resolves when the session let go of everything another session of this window would need (the replica). */
  dispose(): Promise<void> {
    if (this.disposed) return this.session.whenClosed()
    this.disposed = true
    this.retain()
    this.unsubscribe()
    this.offSubmissions?.()
    this.offStatus()
    this.setLockPolling(false)
    this.locks.dispose()
    this.edits.dispose()
    this.outline.dispose()
    this.userState.dispose()
    this.freshness.dispose()
    this.wish.dispose()
    this.session.close()
    return this.session.whenClosed()
  }
}
