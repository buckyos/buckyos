import { messageAgentAsk, messageAgentAskAnswer, messageAgentTaskId, messageObjId, type AgentAsk, type MessageObject, type MsgContent, type MsgRelation } from '../../protocol/msgobj'

/** Reactions to one message, grouped by key (`CYFS 标准对象` §16.3). */
export interface MessageReaction {
  key: string
  dids: string[]
  /** ObjId of each reactor's reaction message: cancelling a reaction is a `redact` of that message. */
  messages: Record<string, string>
}

/** One `edit` message folded into its target. */
export interface MessageEdit {
  id?: string
  at: number
  content: MsgContent
}

/**
 * One question or answer of an Agent Turn, folded under the Turn's anchor
 * (`ui_relations.thread`). `content` is the effective content (after edits).
 */
export interface MessageThreadEntry {
  id: string
  from: string
  senderName?: string
  content: MsgContent
  at: number
  /** On a question: what it asks. */
  agent_ask?: AgentAsk
  /** On an answer that picked an option. */
  option?: string
  redacted?: boolean
}

/** What the relation messages of a timeline say about one target message (`ui_relations`). */
export interface MessageRelationsView {
  /** The complete content of the latest edit by the author, plus every edit seen (oldest first). */
  edited?: { content: MsgContent; at: number; edits: MessageEdit[] }
  redacted?: { by: string; at: number }
  reactions?: MessageReaction[]
  /** Only on a `thread` message: the quoted target, when it is in the same timeline. */
  replyTo?: { id: string; from: string; senderName?: string; content: string; found: boolean }
  /** Only on an Agent anchor (`agent_task`): the questions it asked during its Turn and their answers, oldest first. */
  thread?: MessageThreadEntry[]
}

/** ObjId of a timeline message; it lives with the protocol helpers, which build answers from it. */
export { messageObjId }

function messageIds(message: MessageObject): string[] {
  const ids = [messageObjId(message), message.ui_sent_msg_id, message.ui_message_id].filter((id): id is string => typeof id === 'string')
  return [...new Set(ids)]
}

export function messageRelation(message: MessageObject): MsgRelation | undefined {
  const relation = message.relates_to
  return relation && typeof relation === 'object' && typeof relation.target === 'string' && typeof relation.rel === 'string' ? relation : undefined
}

/** Edits, redactions and reactions are folded into their target and never shown as rows. */
export function isHiddenRelationMessage(message: MessageObject): boolean {
  const rel = messageRelation(message)?.rel
  return rel === 'edit' || rel === 'redact' || rel === 'reaction'
}

export function messageRelations(message: MessageObject): MessageRelationsView | undefined {
  const value = message.ui_relations
  return value && typeof value === 'object' ? value as MessageRelationsView : undefined
}

/** Whether the viewer is addressed by the message's structured mentions. */
export function mentionsViewer(message: MessageObject, viewerDid: string): boolean {
  const mentions = message.mentions
  if (!mentions || typeof mentions !== 'object') return false
  return mentions.all === true || (Array.isArray(mentions.dids) && mentions.dids.includes(viewerDid))
}

/**
 * Apply `relates_to` of a timeline (oldest first): relation messages vanish,
 * their targets carry the latest edit, a redaction and reaction counts under
 * `ui_relations`; a `thread` message keeps its row and quotes its target. An
 * edit only counts when its author is the target's author; a `redact` whose
 * target is a reaction message cancels that reaction (§16.3: "取消回应就是撤回
 * 这条回应消息"); the host enforces the rest and rejects anything else before
 * it reaches a timeline.
 *
 * The questions and answers of an Agent Turn are the exception to "a
 * `thread` message keeps its row" (`foldAgentQuestions`): they go under the
 * Turn's anchor as `ui_relations.thread`.
 */
export function foldMessageRelations(messages: readonly MessageObject[]): MessageObject[] {
  const indexById = new Map<string, number>()
  const visible: MessageObject[] = []
  for (const message of messages) {
    if (isHiddenRelationMessage(message)) continue
    const index = visible.length
    visible.push(message)
    for (const id of messageIds(message)) if (!indexById.has(id)) indexById.set(id, index)
  }
  const views = new Map<number, MessageRelationsView>()
  const viewOf = (index: number) => { const view = views.get(index) ?? {}; views.set(index, view); return view }
  /** Reaction message id → where it was counted, so a later redact of it can be undone. */
  const reactionsById = new Map<string, { targetIndex: number; key: string; from: string }>()
  for (const message of messages) {
    const relation = messageRelation(message)
    if (!relation || relation.rel === 'thread') continue
    if (relation.rel === 'redact') {
      const reaction = reactionsById.get(relation.target)
      if (reaction) {
        const reactions = views.get(reaction.targetIndex)?.reactions
        const entry = reactions?.find(item => item.key === reaction.key)
        if (entry && entry.messages[reaction.from] === relation.target) {
          entry.dids = entry.dids.filter(did => did !== reaction.from)
          delete entry.messages[reaction.from]
          if (entry.dids.length === 0 && reactions) reactions.splice(reactions.indexOf(entry), 1)
        }
        continue
      }
    }
    const targetIndex = indexById.get(relation.target)
    const target = targetIndex === undefined ? undefined : visible[targetIndex]
    if (targetIndex === undefined || !target) continue
    const view = viewOf(targetIndex)
    if (relation.rel === 'edit') {
      if (message.from !== target.from) continue
      const id = messageObjId(message)
      const edits = view.edited?.edits ?? []
      if (id && edits.some(edit => edit.id === id)) continue
      const edit: MessageEdit = { ...(id ? { id } : {}), at: message.created_at_ms, content: message.content }
      const latest = !view.edited || edit.at >= view.edited.at ? edit : view.edited
      view.edited = { content: latest.content, at: latest.at, edits: [...edits, edit].sort((a, b) => a.at - b.at) }
    } else if (relation.rel === 'redact') {
      if (!view.redacted || message.created_at_ms < view.redacted.at) view.redacted = { by: message.from, at: message.created_at_ms }
    } else if (relation.rel === 'reaction') {
      const key = relation.key ?? message.content.content ?? ''
      if (!key) continue
      const reactions = view.reactions ?? (view.reactions = [])
      const entry = reactions.find(item => item.key === key) ?? (reactions[reactions.push({ key, dids: [], messages: {} }) - 1])
      if (!entry.dids.includes(message.from)) entry.dids.push(message.from)
      const id = messageObjId(message)
      // The same (from, target, key) counts once; the first message is the one a redact must target.
      if (id) { entry.messages[message.from] ??= id; reactionsById.set(id, { targetIndex, key, from: message.from }) }
      for (const alias of messageIds(message)) if (!reactionsById.has(alias)) reactionsById.set(alias, { targetIndex, key, from: message.from })
    }
  }
  // A redacted message shows its placeholder whatever its edits said.
  for (const view of views.values()) if (view.redacted) delete view.edited
  const folded = foldAgentQuestions(visible, indexById, views, viewOf)
  // Quotes are resolved last so they reflect the target's final edit / redaction.
  visible.forEach((message, index) => {
    const relation = messageRelation(message)
    if (relation?.rel !== 'thread' || folded.has(index)) return
    const targetIndex = indexById.get(relation.target)
    const target = targetIndex === undefined ? undefined : visible[targetIndex]
    const targetView = targetIndex === undefined ? undefined : views.get(targetIndex)
    viewOf(index).replyTo = target
      ? { id: relation.target, from: target.from, senderName: typeof target.ui_sender_name === 'string' ? target.ui_sender_name : undefined, content: targetView?.redacted ? '' : summaryOf(targetView?.edited?.content ?? target.content), found: true }
      : { id: relation.target, from: '', content: '', found: false }
  })
  for (const [index, view] of views) {
    if (view.reactions && view.reactions.length === 0) delete view.reactions
    if (Object.keys(view).length === 0) views.delete(index)
  }
  if (views.size === 0 && folded.size === 0) return visible
  const rows: MessageObject[] = []
  visible.forEach((message, index) => { if (!folded.has(index)) rows.push(views.has(index) ? { ...message, ui_relations: views.get(index) } : message) })
  return rows
}

/**
 * The questions and answers of Agent Turns: a `thread` message to an
 * earlier anchor carrying `agent_task` is folded into that anchor's
 * `ui_relations.thread` (instead of a row of its own) when it is
 *
 * - a question: it carries `agent_ask` and comes from the anchor's author;
 * - an answer: it picked an option (`agent_ask.answer`), or it is the first
 *   message after an open question of that anchor from someone other than
 *   the asker.
 *
 * Any other reply to an Agent message keeps its row. Returns the indexes
 * folded away.
 */
function foldAgentQuestions(visible: readonly MessageObject[], indexById: ReadonlyMap<string, number>, views: ReadonlyMap<number, MessageRelationsView>, viewOf: (index: number) => MessageRelationsView): Set<number> {
  const folded = new Set<number>()
  /** Anchor index → the asker of its question still waiting for an answer. */
  const open = new Map<number, string>()
  visible.forEach((message, index) => {
    const relation = messageRelation(message)
    if (relation?.rel !== 'thread') return
    const anchorIndex = indexById.get(relation.target)
    const anchor = anchorIndex === undefined ? undefined : visible[anchorIndex]
    if (anchorIndex === undefined || !anchor || anchorIndex >= index || folded.has(anchorIndex) || !messageAgentTaskId(anchor)) return
    const ask = messageAgentAsk(message)
    const picked = messageAgentAskAnswer(message)
    if (ask && message.from === anchor.from) open.set(anchorIndex, message.from)
    else if (picked || (open.has(anchorIndex) && open.get(anchorIndex) !== message.from)) open.delete(anchorIndex)
    else return
    folded.add(index)
    const own = views.get(index)
    const entry: MessageThreadEntry = {
      id: messageObjId(message) ?? '',
      from: message.from,
      ...(typeof message.ui_sender_name === 'string' ? { senderName: message.ui_sender_name } : {}),
      content: own?.edited?.content ?? message.content,
      at: message.created_at_ms,
      ...(ask && message.from === anchor.from ? { agent_ask: ask } : {}),
      ...(picked ? { option: picked.option } : {}),
      ...(own?.redacted ? { redacted: true } : {}),
    }
    const view = viewOf(anchorIndex)
    view.thread = [...(view.thread ?? []), entry]
  })
  return folded
}

/** The viewer's own reaction message for `key` on a folded message, if any. */
export function ownReactionId(message: MessageObject, viewerDid: string, key: string): string | undefined {
  return messageRelations(message)?.reactions?.find(item => item.key === key)?.messages[viewerDid]
}

function summaryOf(content: MsgContent): string {
  const text = (content.content ?? '').trim()
  if (text) return text
  return (content.refs ?? []).map(ref => ref.label ?? (ref.target.type === 'data_obj' ? ref.target.uri_hint ?? ref.target.obj_id : ref.target.did)).join('\n')
}

/**
 * The content a bubble shows: the complete content of the latest edit
 * (format, refs and machine parts included) when there is one. The anchor
 * keeps everything else: its ObjId, its place in the timeline, its `agent_task`.
 */
export function effectiveContent(message: MessageObject): MsgContent {
  return messageRelations(message)?.edited?.content ?? message.content
}

/** Plain text standing for a message: its displayed text, or the names of its attachments when it has none. */
export function messageSummaryText(message: MessageObject): string {
  return summaryOf(effectiveContent(message))
}

export function displayedContent(message: MessageObject): string {
  return effectiveContent(message).content ?? ''
}

/**
 * The edits folded into `anchors`, taken from the unfolded timeline: their
 * mailbox records are read together with the bubble that shows them, so one
 * bubble counts once.
 */
export function editsOf(messages: readonly MessageObject[], anchors: readonly MessageObject[]): MessageObject[] {
  const authors = new Map<string, string>()
  for (const anchor of anchors) for (const id of messageIds(anchor)) authors.set(id, anchor.from)
  return messages.filter(message => {
    const relation = messageRelation(message)
    return relation?.rel === 'edit' && authors.get(relation.target) === message.from
  })
}

/**
 * The questions and answers folded under `anchors` (`ui_relations.thread`),
 * taken from the unfolded timeline: like edits, they are read together with
 * the bubble that shows them.
 */
export function threadMessagesOf(messages: readonly MessageObject[], anchors: readonly MessageObject[]): MessageObject[] {
  if (!anchors.some(anchor => messageAgentTaskId(anchor))) return []
  const anchorIds = new Set(anchors.flatMap(messageIds))
  const entryIds = new Set<string>()
  for (const row of foldMessageRelations(messages)) {
    if (!messageIds(row).some(id => anchorIds.has(id))) continue
    for (const entry of messageRelations(row)?.thread ?? []) if (entry.id) entryIds.add(entry.id)
  }
  if (entryIds.size === 0) return []
  return messages.filter(message => entryIds.has(messageObjId(message) ?? ''))
}
