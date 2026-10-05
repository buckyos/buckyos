/* Recoverable drafts of rich text edits the backend refused (design §3.3.7): kept in this browser,
 * viewable and exportable, never uploaded. */

export interface RichTextDraft {
  draft_id: string
  workspace_id: string
  entity_id: string
  saved_at: string
  reason: string
  /** Editor document JSON at the moment of the refusal. */
  ast: unknown
}

const KEY = 'aiworkspace.richtext-drafts'
const MAX_DRAFTS = 50

export function listDrafts(workspaceId?: string, entityId?: string): RichTextDraft[] {
  try {
    const all = JSON.parse(window.localStorage.getItem(KEY) ?? '[]') as RichTextDraft[]
    return all.filter((draft) => (!workspaceId || draft.workspace_id === workspaceId) && (!entityId || draft.entity_id === entityId))
  } catch {
    return []
  }
}

/** Returns false when the browser refused to store it (the caller must then say so). */
export function saveDraft(draft: RichTextDraft): boolean {
  try {
    const all = [draft, ...listDrafts()].slice(0, MAX_DRAFTS)
    window.localStorage.setItem(KEY, JSON.stringify(all))
    return true
  } catch {
    return false
  }
}

export function deleteDraft(draftId: string) {
  try {
    window.localStorage.setItem(KEY, JSON.stringify(listDrafts().filter((draft) => draft.draft_id !== draftId)))
  } catch { /* nothing to delete from */ }
}

export function astPlainText(node: unknown): string {
  if (!node || typeof node !== 'object') return ''
  const { text, content, type } = node as { text?: string; content?: unknown[]; type?: string }
  if (typeof text === 'string') return text
  const inner = (content ?? []).map(astPlainText).join('')
  return type === 'paragraph' || type === 'heading' ? `${inner}\n` : inner
}
