import type { TaskWatchNode } from '../../../../api/task_mgr'
import { isMessageActivity } from '../../sessionModel'
import { messageAgentTaskId, messageObjId, type MessageObject } from '../../protocol/msgobj'
import { messageRelation } from '../history/relations'

/**
 * What the Agent does with the message in front: `running` — it is being
 * handled; `interrupted` — the host restarted in the middle of it;
 * `paused` — its session was paused; `idle` — nothing in front (no
 * placeholder, a finished Turn, or a Turn that only waits for its work or
 * for something outside).
 */
export type ForegroundMode = 'idle' | 'running' | 'interrupted' | 'paused'

/** The placeholder of a Turn: an Agent message carrying `agent_task`. */
export interface TurnAnchor { message: MessageObject; taskId: string }

export interface ForegroundState { mode: ForegroundMode; anchor?: TurnAnchor }

const IDLE: ForegroundState = { mode: 'idle' }

type TurnTask = Pick<TaskWatchNode, 'phase' | 'message'>

/** State of one Turn from its TaskMgr task; a task that cannot be read is `idle`. */
export function turnMode(task: TurnTask | undefined): ForegroundMode {
  if (!task) return 'idle'
  if (task.phase === 'Paused') return /^interrupted\b/.test(task.message?.trim() ?? '') ? 'interrupted' : 'paused'
  return task.phase === 'Running' || task.phase === 'Accepted' ? 'running' : 'idle'
}

/**
 * The newest placeholder of the conversation someone else posted, from the
 * timeline alone (oldest first, unfolded). Relation messages, status pills
 * and redacted placeholders are not placeholders.
 */
export function latestTurnAnchor(timeline: readonly MessageObject[], viewerDid: string): TurnAnchor | null {
  const redacted = new Set<string>()
  for (let index = timeline.length - 1; index >= 0; index--) {
    const message = timeline[index]
    const relation = messageRelation(message)
    if (relation?.rel === 'redact') { redacted.add(relation.target); continue }
    if (relation?.rel === 'edit' || relation?.rel === 'reaction' || message.from === viewerDid) continue
    const taskId = messageAgentTaskId(message)
    if (!taskId) continue
    const id = messageObjId(message)
    if (id && redacted.has(id)) continue
    return { message, taskId }
  }
  return null
}

/**
 * The foreground state of a conversation: the newest placeholder and what
 * its Turn does. While it is `running`, `interrupted` or `paused` nothing
 * new may be sent (an open question of the Agent is answered all the same:
 * the caller gives the answer state precedence).
 */
export function foregroundState(anchor: TurnAnchor | null, task: TurnTask | undefined): ForegroundState {
  if (!anchor) return IDLE
  const mode = turnMode(task)
  return mode === 'idle' ? IDLE : { mode, anchor }
}

/**
 * The viewer's message a Turn handles: the one whose ObjId the Turn's task
 * names (`inputKeys`), else the viewer's newest message before the
 * placeholder. Retracting it retracts the input.
 */
export function turnInputMessage(timeline: readonly MessageObject[], anchor: MessageObject, viewerDid: string, inputKeys: readonly string[] = []): MessageObject | null {
  const redacted = new Set(timeline.flatMap(message => messageRelation(message)?.rel === 'redact' ? [messageRelation(message)!.target] : []))
  const own = timeline.filter(message => message.from === viewerDid && !messageRelation(message) && isMessageActivity(message) && message.ui_delivery_status !== 'failed' && !!messageObjId(message) && !redacted.has(messageObjId(message)!))
  const named = own.filter(message => inputKeys.includes(messageObjId(message)!)).pop()
  if (named) return named
  const at = timeline.indexOf(anchor)
  return own.filter(message => at < 0 ? message.created_at_ms <= anchor.created_at_ms : timeline.indexOf(message) < at).pop() ?? null
}
