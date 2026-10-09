/* UndoCoordinator (design §2.7): one per session, one stack.
 *
 *   { kind: 'editor', entity_id }  one step of a rich text editor's Loro UndoManager
 *   { kind: 'commit', commit_id }  an accepted Compensable commit → `doc.undo`
 *
 * Ctrl/Cmd+Z pops exactly one entry and hands it to its executor. Editors do not bind undo keys
 * themselves. Redo of a commit is the undo of its compensating commit (there is no redo storage on
 * the backend).
 *
 *   { kind: 'pending', key }       replica session: a submission that was not sent yet → it is removed
 *                                  from the pending queue (no network). Once the backend accepted it the
 *                                  entry becomes a `commit` entry.
 *   { kind: 'resubmit', … }        the redo of a removed pending submission: the same operations again. */

import type { WorkspaceSession } from '../api/session'
import type { CommitOutcome, Operation } from '../api/types'
import { Emitter } from './emitter'

export type UndoEntry =
  /** `steps`: how many UndoManager steps this entry groups (typing within a short pause is one entry). */
  | { kind: 'editor'; entity_id: string; editor_id: string; steps: number; last_at: number }
  | { kind: 'commit'; commit_id: string; label: string }
  | { kind: 'pending'; key: string; label: string }
  | { kind: 'resubmit'; operations: Operation[]; label: string }

export interface EditorUndoHandle {
  undo(): boolean
  redo(): boolean
}

/** Editor changes closer together than this are one undo entry. */
const EDITOR_MERGE_MS = 400

export interface UndoSnapshot {
  undo: readonly UndoEntry[]
  redo: readonly UndoEntry[]
  busy: boolean
  /** Result of the last undo/redo that needs the user's attention (conflict, NOT_UNDOABLE…). */
  problem: string | null
}

function describeFailure(action: string, label: string, outcome: CommitOutcome): string {
  if (outcome.status === 'conflict') {
    const items = outcome.conflicts?.length ?? 0
    return `无法${action}「${label}」：其中 ${items} 处内容之后已被修改（${outcome.code}）。未做任何改动。`
  }
  if (outcome.status === 'rejected') {
    const detail = outcome.errors?.[0]?.detail ?? outcome.detail ?? ''
    if (outcome.code === 'NOT_UNDOABLE') return `「${label}」不可由后台撤销（NOT_UNDOABLE）${detail ? `：${detail}` : ''}`
    return `无法${action}「${label}」：${outcome.code}${detail ? ` — ${detail}` : ''}`
  }
  if (outcome.status === 'unknown') return `${action}「${label}」的结果未知（${outcome.detail}），请稍后重试。`
  if (outcome.status === 'storage_failed') return `${action}「${label}」没有完成：${outcome.detail}`
  return ''
}

export class UndoCoordinator {
  private undoStack: UndoEntry[] = []
  private redoStack: UndoEntry[] = []
  private busy = false
  private problem: string | null = null
  private replaying = false
  private chain: Promise<void> = Promise.resolve()
  private readonly editors = new Map<string, EditorUndoHandle>()
  private readonly emitter = new Emitter()
  private cached: UndoSnapshot = { undo: [], redo: [], busy: false, problem: null }
  private readonly session: WorkspaceSession
  readonly subscribe = this.emitter.subscribe

  constructor(session: WorkspaceSession) {
    this.session = session
  }

  snapshot = (): UndoSnapshot => this.cached

  private publish() {
    this.cached = { undo: [...this.undoStack], redo: [...this.redoStack], busy: this.busy, problem: this.problem }
    this.emitter.emit()
  }

  registerEditor(editorId: string, handle: EditorUndoHandle): () => void {
    this.editors.set(editorId, handle)
    return () => {
      this.editors.delete(editorId)
      // Steps of a closed editor cannot be executed any more.
      this.undoStack = this.undoStack.filter((entry) => entry.kind !== 'editor' || entry.editor_id !== editorId)
      this.redoStack = this.redoStack.filter((entry) => entry.kind !== 'editor' || entry.editor_id !== editorId)
      this.publish()
    }
  }

  /** Called by an editor for every step its UndoManager records (not while this coordinator replays one). */
  pushEditorStep(entityId: string, editorId: string) {
    if (this.replaying) return
    const now = Date.now()
    const top = this.undoStack.at(-1)
    if (top && top.kind === 'editor' && top.editor_id === editorId && now - top.last_at < EDITOR_MERGE_MS) {
      top.steps += 1
      top.last_at = now
    } else {
      this.undoStack.push({ kind: 'editor', entity_id: entityId, editor_id: editorId, steps: 1, last_at: now })
    }
    this.redoStack = []
    this.publish()
  }

  /** Called for every accepted Compensable commit made by this session. */
  pushCommit(commitId: string, label: string) {
    this.undoStack.push({ kind: 'commit', commit_id: commitId, label })
    this.redoStack = []
    this.publish()
  }

  /** A submission of this session was saved on this device and is waiting to be sent. */
  pushPending(key: string, label: string) {
    this.undoStack.push({ kind: 'pending', key, label })
    this.redoStack = []
    this.publish()
  }

  /** The backend settled a pending submission: accepted → it is now undone like any commit; refused → nothing to undo. */
  resolvePending(key: string, commitId: string | null) {
    const settle = (stack: UndoEntry[]) => stack.flatMap((entry): UndoEntry[] => {
      if (entry.kind !== 'pending' || entry.key !== key) return [entry]
      return commitId ? [{ kind: 'commit', commit_id: commitId, label: entry.label }] : []
    })
    this.undoStack = settle(this.undoStack)
    this.redoStack = settle(this.redoStack)
    this.publish()
  }

  dismissProblem() {
    this.problem = null
    this.publish()
  }

  undo(): Promise<void> { return this.enqueue('undo') }
  redo(): Promise<void> { return this.enqueue('redo') }

  private enqueue(direction: 'undo' | 'redo'): Promise<void> {
    const run = this.chain.then(() => this.step(direction))
    this.chain = run.catch(() => undefined)
    return run
  }

  private async step(direction: 'undo' | 'redo') {
    const from = direction === 'undo' ? this.undoStack : this.redoStack
    const to = direction === 'undo' ? this.redoStack : this.undoStack
    const entry = from.pop()
    if (!entry) return
    const action = direction === 'undo' ? '撤销' : '重做'
    this.problem = null
    if (entry.kind === 'editor') {
      const handle = this.editors.get(entry.editor_id)
      this.replaying = true
      let done = false
      try {
        for (let i = 0; handle && i < entry.steps; i++) {
          if (!(direction === 'undo' ? handle.undo() : handle.redo())) break
          done = true
        }
      } finally { this.replaying = false }
      // never extend a replayed entry with later typing
      if (done) to.push({ ...entry, last_at: 0 })
      else this.problem = `编辑器中没有可${action}的步骤。`
      this.publish()
      return
    }
    if (entry.kind === 'pending') {
      // not sent yet: taking it out of the queue is the whole undo, no network involved
      try {
        const operations = await this.session.offline?.discardPending(entry.key)
        if (operations) to.push({ kind: 'resubmit', operations, label: entry.label })
        else this.problem = `「${entry.label}」已不在待提交队列中，没有可${action}的内容。`
      } catch (error) {
        from.push(entry)
        this.problem = `无法${action}「${entry.label}」：${error instanceof Error ? error.message : String(error)}`
      }
      this.publish()
      return
    }
    if (entry.kind === 'resubmit') {
      this.busy = true
      this.publish()
      try {
        const outcome = await this.session.commit(entry.operations, { message: entry.label, meta: { label: entry.label } })
        if (outcome.status === 'saved_locally') to.push({ kind: 'pending', key: outcome.idempotency_key, label: entry.label })
        else if (outcome.status === 'accepted') to.push({ kind: 'commit', commit_id: outcome.commit_id, label: entry.label })
        else {
          if (outcome.status === 'storage_failed') from.push(entry)
          this.problem = describeFailure(action, entry.label, outcome)
        }
      } catch (error) {
        from.push(entry)
        this.problem = `${action}失败：${error instanceof Error ? error.message : String(error)}`
      } finally {
        this.busy = false
        this.publish()
      }
      return
    }
    this.busy = true
    this.publish()
    try {
      const outcome = await this.session.undoCommit(entry.commit_id)
      if (outcome.status === 'accepted') {
        // The compensating commit is what the opposite direction has to undo.
        to.push({ kind: 'commit', commit_id: outcome.commit_id, label: entry.label })
        await this.session.whenApplied(outcome.seq)
      } else {
        if (outcome.status === 'unknown') from.push(entry)
        this.problem = describeFailure(action, entry.label, outcome)
      }
    } catch (error) {
      from.push(entry)
      this.problem = `${action}失败：${error instanceof Error ? error.message : String(error)}`
    } finally {
      this.busy = false
      this.publish()
    }
  }
}
