import { useContext, useState, type ReactNode } from 'react'
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
import { TurnControls } from '../tasks/TurnControls'
import { getWorkCardsWatch, useBranchExpanded, useSecondClock, useTodoExpanded, useWorkCards } from './index'
import { WorkControls } from './WorkControls'
import { cardRunning, cardUsage, displayModel, flattenWork, formatDuration, formatTokens, isTodoEnded, isWorkFinished, localizeLine, todoTitle, visibleTodos, workChildren, workDuration, workTone, type WorkCard, type WorkCards, type WorkState, type WorkTodo, type WorkUsage } from './model'

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
  if (taskId && request && cards?.status === 'ready' && cards.cards && cards.cards.sessions.length > 0) return <WorkSessionCards message={message} cards={cards.cards} stale={cards.stale} selfDid={selfDid} taskId={taskId} stopped={task?.phase === 'Terminal' && task.outcome === 'Canceled'} />
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

/** What every row of one card list shares. */
export interface WorkListContext {
  now: number
  stale: boolean
  /** Remembers which TodoLists the viewer opened or closed. */
  viewerDid: string
  anchorId: string
  onOpen: (card: WorkCard, title: string) => void
  /** Controls at the end of a card's first line (pause / resume / stop). */
  actions?: (card: WorkCard, title: string) => ReactNode
}

/**
 * The cards of one bubble: work session cards in creation order, a plan's
 * TodoList under its card. A Turn the owner stopped says which of the work
 * sessions it started are still running and lets each be stopped: stopping a
 * message does not stop the work it already started.
 */
export function WorkSessionCards({ message, cards, stale, selfDid, taskId, stopped = false }: { message: MessageObject; cards: WorkCards; stale: boolean; selfDid?: DID; taskId?: string; stopped?: boolean }) {
  const { t } = useI18n()
  const now = useSecondClock(!stale && cards.sessions.some(cardRunning))
  const { openDetails, agentTurn } = useContext(ConversationMessageActionsContext)
  const running = stopped && agentTurn ? flattenWork(cards.sessions).filter(card => !isWorkFinished(card.state)).length : 0
  const context: WorkListContext = {
    now, stale, viewerDid: selfDid ?? '', anchorId: messageObjId(message) ?? '',
    onOpen: (card, title) => openDetails?.(message, { target: { agentDid: cards.agent_did, sessionId: card.session_id }, title, card }),
    actions: running > 0 ? (card, title) => <WorkControls agentDid={cards.agent_did} card={card} title={title} only={['stop']} onChanged={() => getWorkCardsWatch().resync()} /> : undefined,
  }
  return (
    <div className="mh-work mt-2 space-y-0.5 rounded-xl p-1" data-testid="work-cards" data-no-details data-complete={cards.complete} data-stale={stale || undefined}>
      {taskId ? <TurnControls anchor={message} taskId={taskId} place="bubble" /> : null}
      {running > 0 ? <p className="px-2 py-1 text-[12px]" role="note" data-testid="turn-works-running">{t('messagehub.turn.worksRunning', undefined, { count: running })}</p> : null}
      <WorkCardList cards={cards.sessions} context={context} />
    </div>
  )
}

/** Top-level cards: each with its TodoList and, under it, the sessions it created outside its Todos. */
export function WorkCardList({ cards, context }: { cards: readonly WorkCard[]; context: WorkListContext }) {
  return <>{cards.map(card => <div key={card.session_id}>
    <WorkCardRow card={card} title={card.title} usage={cardUsage(card)} planUsage={card.shape === 'plan' && card.subtree_usage ? card.usage : undefined} context={context} />
    {card.todos && card.todos.items.length > 0 ? <TodoListSection card={card} context={context} open={!isWorkFinished(card.state)} /> : null}
    {(card.children ?? []).length > 0 ? <div className="ml-[23px]" data-testid="work-children">{card.children!.map(child => <WorkBranch key={child.session_id} card={child} title={child.title} context={context} />)}</div> : null}
  </div>)}</>
}

/** A session below the top level: its row and, folded until asked for, what is under it. */
function WorkBranch({ card, title, context }: { card: WorkCard; title: string; context: WorkListContext }) {
  const { t } = useI18n()
  const [open, setOpen] = useBranchExpanded(context.anchorId, card.session_id)
  const below = workChildren(card).length + (card.todos?.items.filter(todo => !todo.card).length ?? 0)
  return <>
    <WorkCardRow card={card} title={title} usage={card.usage} context={context} todo />
    {below > 0 ? <div className="ml-5" data-testid="work-branch" data-expanded={open}>
      <button type="button" className="mh-work-toggle flex min-h-7 items-center gap-1 rounded-lg px-1 text-[12px] font-medium text-[color:var(--cp-muted)]" aria-expanded={open} onClick={() => setOpen(!open)} data-testid="work-branch-toggle">
        {open ? <ChevronDown size={14} aria-hidden /> : <ChevronRight size={14} aria-hidden />}
        {t('messagehub.work.subSessions', undefined, { count: workChildren(card).length })}
      </button>
      {open ? <>
        {card.todos && card.todos.items.length > 0 ? <TodoListSection card={card} context={context} open /> : null}
        {(card.children ?? []).map(child => <WorkBranch key={child.session_id} card={child} title={child.title} context={context} />)}
      </> : null}
    </div> : null}
  </>
}

export function WorkCardRow({ card, title, usage, planUsage, context, todo = false }: { card: WorkCard; title: string; usage: WorkUsage; planUsage?: WorkUsage; context: WorkListContext; todo?: boolean }) {
  const { t } = useI18n()
  const text = useText()
  const { now, stale } = context
  const onOpen = () => context.onOpen(card, title)
  const line = text(localizeLine(card.line, card.state, card.paused))
  const model = displayModel(card.model)
  const models = card.usage.models.map(item => item.model)
  const tokens = planUsage
    ? [t('messagehub.work.usagePlan', undefined, { usage: exactUsage(t, planUsage) }), t('messagehub.work.usageTodos', undefined, { usage: exactUsage(t, { input_tokens: usage.input_tokens - planUsage.input_tokens, output_tokens: usage.output_tokens - planUsage.output_tokens, rounds: usage.rounds - planUsage.rounds, models: [] }) })].join('\n')
    : exactUsage(t, usage)
  const staleNote = stale && !isWorkFinished(card.state) ? t('messagehub.work.stale') : ''
  const second = [line, staleNote].filter(Boolean).join(' · ')
  return (
    <div role="button" tabIndex={0} className={`mh-work-card rounded-lg px-2 py-1 ${todo ? 'ml-5 text-[12px]' : 'text-[13px]'}`} onClick={onOpen} onKeyDown={event => { if (event.target === event.currentTarget && (event.key === 'Enter' || event.key === ' ')) { event.preventDefault(); onOpen() } }} aria-label={t('messagehub.work.open', undefined, { title })} data-testid={todo ? 'todo-card' : 'work-card'} data-session-id={card.session_id} data-state={card.state} data-paused={card.paused} data-shape={card.shape}>
      <div className="flex items-center gap-2">
        <WorkStateIcon state={card.state} stale={stale} size={todo ? 13 : 15} />
        <span className="min-w-0 flex-1 truncate font-medium" title={title} data-testid="work-card-title">{title}</span>
        <span className="shrink-0 tabular-nums text-[color:var(--cp-muted)]" data-testid="work-card-time">{formatDuration(workDuration(card, now))}</span>
        {usage.rounds > 0 ? <span className="shrink-0 tabular-nums" title={tokens} data-testid="work-card-tokens">
          <span className="mh-work-in text-[color:var(--cp-muted)]">{t('messagehub.work.tokensIn', undefined, { value: formatTokens(usage.input_tokens) })} · </span>
          <span>{t('messagehub.work.tokensOut', undefined, { value: formatTokens(usage.output_tokens) })}</span>
        </span> : null}
        {context.actions?.(card, title)}
      </div>
      {second || model ? <div className={`flex items-center gap-2 ${todo ? 'pl-[21px] text-[11px]' : 'pl-[23px] text-[12px]'}`}>
        <span className="min-w-0 flex-1 truncate opacity-80" title={second} data-testid="work-card-line">{second}</span>
        {model ? <span className="mh-work-model shrink-0 text-[color:var(--cp-muted)]" title={models.length > 1 ? models.join('\n') : card.model ?? undefined} data-testid="work-card-model">{model}</span> : null}
      </div> : null}
    </div>
  )
}

function TodoListSection({ card, context, open }: { card: WorkCard; context: WorkListContext; open: boolean }) {
  const { t } = useI18n()
  const todos = card.todos!
  const [expanded, setExpanded] = useTodoExpanded(context.viewerDid, context.anchorId, card.session_id, open)
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
          ? <WorkBranch card={todo.card} title={todoTitle(todo)} context={context} />
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
