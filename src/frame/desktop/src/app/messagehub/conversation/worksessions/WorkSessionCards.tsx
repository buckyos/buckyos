import { useContext, useState } from 'react'
import { Ban, ChevronDown, ChevronRight, Circle, CircleSlash, MessageCircleQuestion } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import type { DID } from 'buckyos'
import { messageAgentTaskId, type MessageObject } from '../../protocol/msgobj'
import { ConversationMessageActionsContext } from '../history/actions'
import { messageObjId } from '../history/relations'
import { useTaskSummary } from '../tasks'
import { MessageTaskArea, ToneIcon } from '../tasks/MessageTask'
import { toneColor, type TaskText } from '../tasks/taskView'
import { taskWorklogTarget } from '../worklog/model'
import { useSecondClock, useTodoExpanded, useWorkCards } from './index'
import { cardUsage, displayModel, formatDuration, formatTokens, isTodoEnded, isWorkFinished, localizeLine, todoTitle, visibleTodos, workDuration, workTone, type WorkCard, type WorkCards, type WorkState, type WorkTodo, type WorkUsage } from './model'

/**
 * What an Agent reply shows under its body: the cards of the work sessions
 * its UI Turn created, once there are any; otherwise (a quick reply, intent
 * analysis still running, cards that cannot be read) the Turn's task area.
 * Cards are only asked for when the Turn's task has sub-tasks.
 */
export function MessageAgentWork({ message, selfDid }: { message: MessageObject; selfDid?: DID }) {
  const taskId = messageAgentTaskId(message)
  const { snapshot, children } = useTaskSummary(taskId)
  const task = snapshot?.status === 'ready' ? snapshot.task : undefined
  const binding = task ? taskWorklogTarget(task) : null
  const request = taskId && binding && (children?.items.length ?? 0) > 0 ? { agentDid: binding.agentDid, sessionId: binding.sessionId, taskId } : undefined
  const cards = useWorkCards(request, task?.phase === 'Terminal')
  if (request && cards?.status === 'ready' && cards.cards && cards.cards.sessions.length > 0) return <WorkSessionCards message={message} cards={cards.cards} stale={cards.stale} selfDid={selfDid} />
  return <MessageTaskArea message={message} />
}

function useText() {
  const { t } = useI18n()
  return (text: TaskText | string) => typeof text === 'string' ? text : t(text.key, undefined, text.vars)
}

function WorkStateIcon({ state, stale, size }: { state: WorkState; stale: boolean; size: number }) {
  if (state === 'needs_input') return <MessageCircleQuestion size={size} aria-hidden style={{ color: toneColor.waiting }} className="mt-0.5 shrink-0" />
  return <ToneIcon tone={workTone(state)} size={size} spin={!stale} />
}

function exactUsage(t: ReturnType<typeof useI18n>['t'], usage: WorkUsage): string {
  return t('messagehub.work.usageExact', undefined, { input: usage.input_tokens.toLocaleString(), output: usage.output_tokens.toLocaleString(), rounds: usage.rounds })
}

function cardRunning(card: WorkCard): boolean {
  return !isWorkFinished(card.state) || (card.todos?.items ?? []).some(todo => !!todo.card && !isWorkFinished(todo.card.state))
}

/** The cards of one bubble: work session cards in creation order, a plan's TodoList under its card. */
export function WorkSessionCards({ message, cards, stale, selfDid }: { message: MessageObject; cards: WorkCards; stale: boolean; selfDid?: DID }) {
  const now = useSecondClock(!stale && cards.sessions.some(cardRunning))
  const { openDetails } = useContext(ConversationMessageActionsContext)
  const open = (card: WorkCard, title: string) => openDetails?.(message, { target: { agentDid: cards.agent_did, sessionId: card.session_id }, title, card })
  return (
    <div className="mh-work mt-2 space-y-0.5 rounded-xl p-1" data-testid="work-cards" data-no-details data-complete={cards.complete} data-stale={stale || undefined}>
      {cards.sessions.map(card => <div key={card.session_id}>
        <WorkCardRow card={card} title={card.title} usage={cardUsage(card)} planUsage={card.shape === 'plan' && card.subtree_usage ? card.usage : undefined} now={now} stale={stale} onOpen={() => open(card, card.title)} />
        {card.todos && card.todos.items.length > 0 ? <TodoListSection card={card} anchorId={messageObjId(message) ?? ''} viewerDid={selfDid ?? ''} now={now} stale={stale} onOpen={open} /> : null}
      </div>)}
    </div>
  )
}

function WorkCardRow({ card, title, usage, planUsage, now, stale, todo = false, onOpen }: { card: WorkCard; title: string; usage: WorkUsage; planUsage?: WorkUsage; now: number; stale: boolean; todo?: boolean; onOpen: () => void }) {
  const { t } = useI18n()
  const text = useText()
  const line = text(localizeLine(card.line, card.state))
  const model = displayModel(card.model)
  const models = card.usage.models.map(item => item.model)
  const tokens = planUsage
    ? [t('messagehub.work.usagePlan', undefined, { usage: exactUsage(t, planUsage) }), t('messagehub.work.usageTodos', undefined, { usage: exactUsage(t, { input_tokens: usage.input_tokens - planUsage.input_tokens, output_tokens: usage.output_tokens - planUsage.output_tokens, rounds: usage.rounds - planUsage.rounds, models: [] }) })].join('\n')
    : exactUsage(t, usage)
  const staleNote = stale && !isWorkFinished(card.state) ? t('messagehub.work.stale') : ''
  const second = [line, staleNote].filter(Boolean).join(' · ')
  return (
    <div role="button" tabIndex={0} className={`mh-work-card rounded-lg px-2 py-1 ${todo ? 'ml-5 text-[12px]' : 'text-[13px]'}`} onClick={onOpen} onKeyDown={event => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); onOpen() } }} aria-label={t('messagehub.work.open', undefined, { title })} data-testid={todo ? 'todo-card' : 'work-card'} data-session-id={card.session_id} data-state={card.state} data-shape={card.shape}>
      <div className="flex items-center gap-2">
        <WorkStateIcon state={card.state} stale={stale} size={todo ? 13 : 15} />
        <span className="min-w-0 flex-1 truncate font-medium" title={title} data-testid="work-card-title">{title}</span>
        <span className="shrink-0 tabular-nums text-[color:var(--cp-muted)]" data-testid="work-card-time">{formatDuration(workDuration(card, now))}</span>
        {usage.rounds > 0 ? <span className="shrink-0 tabular-nums" title={tokens} data-testid="work-card-tokens">
          <span className="mh-work-in text-[color:var(--cp-muted)]">{t('messagehub.work.tokensIn', undefined, { value: formatTokens(usage.input_tokens) })} · </span>
          <span>{t('messagehub.work.tokensOut', undefined, { value: formatTokens(usage.output_tokens) })}</span>
        </span> : null}
      </div>
      {second || model ? <div className={`flex items-center gap-2 ${todo ? 'pl-[21px] text-[11px]' : 'pl-[23px] text-[12px]'}`}>
        <span className="min-w-0 flex-1 truncate opacity-80" title={second} data-testid="work-card-line">{second}</span>
        {model ? <span className="mh-work-model shrink-0 text-[color:var(--cp-muted)]" title={models.length > 1 ? models.join('\n') : card.model ?? undefined} data-testid="work-card-model">{model}</span> : null}
      </div> : null}
    </div>
  )
}

function TodoListSection({ card, anchorId, viewerDid, now, stale, onOpen }: { card: WorkCard; anchorId: string; viewerDid: string; now: number; stale: boolean; onOpen: (card: WorkCard, title: string) => void }) {
  const { t } = useI18n()
  const todos = card.todos!
  const [expanded, setExpanded] = useTodoExpanded(viewerDid, anchorId, card.session_id, !isWorkFinished(card.state))
  const [showAll, setShowAll] = useState(false)
  const shown = visibleTodos(todos.items, showAll)
  return (
    <div className="ml-[23px]" data-testid="todo-list" data-expanded={expanded}>
      <button type="button" className="mh-work-toggle flex min-h-7 items-center gap-1 rounded-lg px-1 text-[12px] font-medium text-[color:var(--cp-muted)]" aria-expanded={expanded} onClick={() => setExpanded(!expanded)} data-testid="todo-list-toggle">
        {expanded ? <ChevronDown size={14} aria-hidden /> : <ChevronRight size={14} aria-hidden />}
        {t('messagehub.work.todoList', undefined, { done: todos.done, total: todos.total })}
      </button>
      {expanded ? <ul className="-ml-5 space-y-0.5" data-testid="todo-items">
        {shown.items.map(todo => <li key={todo.todo_id}>{todo.card
          ? <WorkCardRow card={todo.card} title={todoTitle(todo)} usage={todo.card.usage} now={now} stale={stale} todo onOpen={() => onOpen(todo.card!, todoTitle(todo))} />
          : <UnboundTodoRow todo={todo} />}</li>)}
        {shown.hidden > 0 ? <li><button type="button" className="mh-work-toggle ml-5 rounded-lg px-2 py-0.5 text-[12px] text-[color:var(--cp-muted)]" onClick={() => setShowAll(true)} data-testid="todo-more">{t('messagehub.work.todoMore', undefined, { count: shown.hidden })}</button></li> : null}
      </ul> : null}
    </div>
  )
}

/** A Todo no session works on (pending, blocked, cancelled before it ran): its title only. */
function UnboundTodoRow({ todo }: { todo: WorkTodo }) {
  const { t } = useI18n()
  const cancelled = todo.status === 'cancelled'
  const blocked = todo.status === 'blocked'
  const icon = { size: 13, 'aria-hidden': true, className: 'shrink-0', style: { color: blocked ? toneColor.waiting : 'var(--cp-muted)' } }
  const reason = blocked ? [todo.blocked_by.length ? t('messagehub.work.blockedBy', undefined, { ids: todo.blocked_by.join(', ') }) : '', todo.block_reason ?? ''].filter(Boolean).join('\n') : undefined
  const state = cancelled ? 'cancelled' : blocked ? 'blocked' : isTodoEnded(todo) ? todo.status : 'pending'
  return (
    <div className="ml-5 flex items-center gap-2 px-2 py-0.5 text-[12px]" title={reason} data-testid="todo-row" data-todo-id={todo.todo_id} data-status={todo.status}>
      {cancelled ? <Ban {...icon} /> : blocked ? <CircleSlash {...icon} /> : <Circle {...icon} />}
      <span className={`min-w-0 flex-1 truncate ${cancelled ? 'text-[color:var(--cp-muted)] line-through' : ''}`}>{todoTitle(todo)}</span>
      <span className="sr-only">{t(`messagehub.work.todo.${state}`, state)}</span>
    </div>
  )
}
