/* The draft protocol of form Blocks (内核架构改进 D13, T5-1): a Block keeps what the user typed under its own
 * private-state key (`block:<id>/…`), records the idempotency key it will submit with *before* sending, and a
 * later window — this device or another — asks `doc.get_submission` first: an accepted submission clears the
 * draft without resending; an unknown one is resent under the same key, never as a new commit. */

import type { CommitOutcome, Json } from '../api/types'
import { randomId } from '../api/ids'
import type { SubmitOptions, WorkspaceStore } from './store'

export interface Draft { text: string; at: string; pending_key?: string; epoch?: string }

export function draftKey(blockId: string, field: string): string {
  return `block:${blockId}/draft:${field}`
}

export function readDraft(store: WorkspaceStore, blockId: string, field: string): Draft | undefined {
  const value = store.userState.get<Json>(draftKey(blockId, field))
  return value && typeof value === 'object' && !Array.isArray(value) && typeof (value as { text?: unknown }).text === 'string' ? value as unknown as Draft : undefined
}

export function saveDraft(store: WorkspaceStore, blockId: string, field: string, text: string) {
  const previous = readDraft(store, blockId, field)
  store.userState.set(draftKey(blockId, field), { ...(previous ?? {}), text, at: new Date().toISOString() } as unknown as Json)
}

export function clearDraft(store: WorkspaceStore, blockId: string, field: string) {
  store.userState.set(draftKey(blockId, field), null)
}

/** Submit the draft's content: the key is written into the draft first, the draft goes once the commit is accepted. */
export async function submitDraft(store: WorkspaceStore, blockId: string, field: string, text: string, options: Omit<SubmitOptions, 'idempotencyKey'>): Promise<CommitOutcome> {
  const previous = readDraft(store, blockId, field)
  const key = previous?.pending_key ?? `${store.session.sessionId}/draft/${randomId()}`
  store.userState.set(draftKey(blockId, field), { text, at: new Date().toISOString(), pending_key: key, epoch: store.session.info().epoch } as unknown as Json)
  const outcome = await store.submit({ ...options, idempotencyKey: key })
  if (outcome.status === 'accepted') clearDraft(store, blockId, field)
  return outcome
}

/** What a window finds when it opens a Block with a draft: `accepted` (the submission happened; the draft is cleared
 * here), `resend` (nothing was recorded: the text is kept and goes out again under the same key), or `draft`
 * (never submitted: only the text). */
export async function recoverDraft(store: WorkspaceStore, blockId: string, field: string): Promise<'accepted' | 'resend' | 'draft' | 'none'> {
  const draft = readDraft(store, blockId, field)
  if (!draft) return 'none'
  if (!draft.pending_key || !draft.epoch) return 'draft'
  try {
    const status = await store.session.submissionStatus(draft.epoch, draft.pending_key)
    if (status.status === 'accepted') {
      clearDraft(store, blockId, field)
      return 'accepted'
    }
    return 'resend'
  } catch {
    return 'draft'
  }
}
