import type { MessageObject, MsgContent, MsgRelation } from '../../protocol/msgobj'

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

/** What the relation messages of a timeline say about one target message (`ui_relations`). */
export interface MessageRelationsView {
  /** The complete content of the latest edit by the author, plus every edit seen (oldest first). */
  edited?: { content: MsgContent; at: number; edits: MessageEdit[] }
  redacted?: { by: string; at: number }
  reactions?: MessageReaction[]
  /** Only on a `thread` message: the quoted target, when it is in the same timeline. */
  replyTo?: { id: string; from: string; senderName?: string; content: string; found: boolean }
}

/** ObjId of a timeline message: the stored record's `msg_id`, else the accepted or local id. */
export function messageObjId(message: MessageObject): string | undefined {
  const record = message.ui_record
  const msgId = record && typeof record === 'object' ? (record as { msgId?: unknown }).msgId : undefined
  if (typeof msgId === 'string') return msgId
  if (typeof message.ui_sent_msg_id === 'string') return message.ui_sent_msg_id
  return typeof message.ui_message_id === 'string' ? message.ui_message_id : undefined
}

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
  // Quotes are resolved last so they reflect the target's final edit / redaction.
  visible.forEach((message, index) => {
    const relation = messageRelation(message)
    if (relation?.rel !== 'thread') return
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
  if (views.size === 0) return visible
  return visible.map((message, index) => views.has(index) ? { ...message, ui_relations: views.get(index) } : message)
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
