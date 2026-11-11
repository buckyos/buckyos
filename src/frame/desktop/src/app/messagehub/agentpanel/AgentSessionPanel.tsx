import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { Bell, BellOff, Download, ExternalLink, File, FileText, Film, Forward, Image, LocateFixed, MoreHorizontal, Music, Pin, PinOff, Play, RefreshCw, Star, X } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useWindowDialog } from '../../../desktop/windows/dialogs'
import { fetchAgentList } from '../../../api/user_mgr'
import { desktopUIStore } from '../../../models/DesktopUIDataModel'
import { bucketObjectRef, bucketObjectUrl } from '../../../components/preview/bucketSource'
import { openAgentHome } from '../../agent-setup/guide'
import { openPreview } from '../../preview/launch'
import { confirmGroupAction } from '../confirmDialog'
import { getObjectAccess } from '../conversation/history/objectAccess'
import { effectiveContent, messageRelations } from '../conversation/history/relations'
import { attachmentKindOf, objectContextOf } from '../conversation/media/source'
import { latestTurnAnchor } from '../conversation/input/foregroundState'
import { CopyValue } from '../conversation/tasks/MessageTask'
import { cardRunning, flattenWork, interruptedRoots, isWorkFinished, localizeLine, type WorkCard, type WorkTree } from '../conversation/worksessions/model'
import { WorkControls } from '../conversation/worksessions/WorkControls'
import { WorkCardList, type WorkListContext } from '../conversation/worksessions/WorkSessionCards'
import { useSecondClock } from '../conversation/worksessions'
import type { WorklogDestination } from '../conversation/worklog/model'
import type { EntityPreferencePatch } from '../EntityList'
import { HeaderMenu, type HeaderMenuItem } from '../EntityListHeader'
import { HubAvatar } from '../HubAvatar'
import { messageObjId, type MessageObject } from '../protocol/msgobj'
import { hubButtonClass } from '../SessionDialogs'
import { relativeActivity, sessionDescription } from '../sessionModel'
import { useMessageHubClock, useMessageHubStore } from '../store'
import type { EntityDetail, MessageHubContext, Session } from '../types'
import { rememberGenerationBreaks } from './breaks'
import { usePanelAction, usePanelResource, type PanelResource } from './hooks'
import { agentConversationState, formatSize, generationBreaks, generationNumber, type AgentSummary, type Delivery, type UiConversation } from './model'
import { getAgentPanelSource } from './source'

const sectionTitleClass = 'text-xs font-semibold uppercase tracking-wide text-[color:var(--cp-muted)]'
const DELIVERY_PAGE = 20

interface AgentSessionPanelProps {
  entity: EntityDetail
  session: Session | null
  context: MessageHubContext
  /** The composer is locked to a question of the Agent. */
  answering: boolean
  /** Loaded messages of the conversation: a new one re-reads the Agent's state. */
  timeline: readonly MessageObject[]
  onClose: () => void
  /** Mute / pin of the Agent as a whole (every session of it); absent for viewers who may not change them. */
  onEntityPreferences?: (entityIds: string[], patch: EntityPreferencePatch) => Promise<void>
  /** Opens a work session's whole work log. */
  onOpenWorklog: (destination: WorklogDestination) => void
  /** Brings a message of this conversation into view. */
  onLocateMessage: (messageId: string) => void
  onForward: (message: MessageObject) => void
  /** The conversation is being archived: nothing may be sent meanwhile. */
  onRotating: (busy: boolean) => void
  /** The session's own details (members, preferences, shared state, management, technical info). */
  sessionDetails: ReactNode
  /** The Agent's profile, relation and session creation policy. */
  entityDetails: ReactNode
}

/**
 * The side panel of a conversation with one of the viewer's own Agents: the
 * Agent as a whole above (state summary, home, mute / pin), this
 * conversation below (what the Agent does in it, the work it has running,
 * starting over, what it delivered). Everything about the Agent is read
 * from the Agent's own service while the panel is open; a part that cannot
 * be read says so and the rest stays usable.
 */
export function AgentSessionPanel({ entity, session, context, answering, timeline, onClose, onEntityPreferences, onOpenWorklog, onLocateMessage, onForward, onRotating, sessionDetails, entityDetails }: AgentSessionPanelProps) {
  const { t } = useI18n()
  const agentDid = entity.id
  const source = getAgentPanelSource()
  const sessionId = session?.id
  // The Turn in front: what its task tree reports is when the Agent's state here changes.
  const turnTaskId = useMemo(() => latestTurnAnchor(timeline, context.ownerDid)?.taskId, [timeline, context.ownerDid])
  const conversation = usePanelResource<UiConversation>(sessionId ? `conversation:${agentDid}:${sessionId}` : null, () => source.conversation(agentDid, context.ownerDid, sessionId!), {
    live: value => !!value.current && (!!value.current.open_turn || value.current.unfinished_works > 0 || !!value.current.paused),
    hintTaskId: turnTaskId,
    signal: timeline.length,
  })
  const current = conversation.value?.current ?? null
  const breaks = useMemo(() => conversation.value ? generationBreaks(conversation.value) : null, [conversation.value])
  useEffect(() => { if (breaks && sessionId) rememberGenerationBreaks(context.viewerDid, sessionId, breaks) }, [breaks, sessionId, context.viewerDid])
  return (
    <section className="flex h-full flex-col bg-[color:var(--cp-surface)]" data-testid="agent-panel" data-agent={agentDid}>
      <header className="flex shrink-0 items-center justify-between border-b border-[color:var(--cp-border)] py-1 pl-4 pr-1"><h2 className="text-sm font-semibold">{t('messagehub.agentPanel.title')}</h2><button type="button" className="flex min-h-11 min-w-11 items-center justify-center" aria-label={t('messagehub.close')} title={t('messagehub.close')} onClick={onClose}><X size={18} /></button></header>
      <div className="shell-scrollbar min-h-0 flex-1 space-y-5 overflow-y-auto overflow-x-hidden p-4 text-sm" data-testid="agent-panel-body">
        <AgentOverview entity={entity} context={context} onEntityPreferences={onEntityPreferences} />
        <hr className="border-[color:var(--cp-border)]" />
        {session ? <>
          <ConversationState session={session} context={context} conversation={conversation} answering={answering} />
          <SubSessions agentDid={agentDid} context={context} sessionId={session.id} uiSessionId={current?.session_id ?? null} hintTaskId={current?.open_turn?.task_id ?? turnTaskId} signal={`${timeline.length}:${current?.unfinished_works ?? 0}:${current?.paused ?? ''}`} onOpenWorklog={onOpenWorklog} onChanged={conversation.refresh} />
          <NewConversation agentDid={agentDid} session={session} conversation={conversation} onRotating={onRotating} />
          <Deliveries agentDid={agentDid} routeKey={conversation.value?.route_key ?? null} timeline={timeline} onLocateMessage={onLocateMessage} onForward={onForward} context={context} />
          <details className="rounded-xl border border-[color:var(--cp-border)] px-3" data-testid="agent-panel-advanced">
            <summary className="flex min-h-11 cursor-pointer items-center text-[13px] font-medium text-[color:var(--cp-muted)]">{t('messagehub.agentPanel.advanced')}</summary>
            <div className="pb-3"><PromptVersionNote conversation={conversation} /></div>
          </details>
          <details className="rounded-xl border border-[color:var(--cp-border)] px-3" data-testid="agent-panel-session">
            <summary className="flex min-h-11 cursor-pointer items-center text-[13px] font-medium text-[color:var(--cp-muted)]">{t('messagehub.sessionDetails')}</summary>
            <div className="pb-3">{sessionDetails}</div>
          </details>
        </> : <p className="text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.noSessions')}</p>}
        <details className="rounded-xl border border-[color:var(--cp-border)] px-3" data-testid="agent-panel-entity">
          <summary className="flex min-h-11 cursor-pointer items-center text-[13px] font-medium text-[color:var(--cp-muted)]">{t('messagehub.agentPanel.profile')}</summary>
          <div className="pb-3">{entityDetails}</div>
        </details>
      </div>
    </section>
  )
}

function Unavailable({ testId }: { testId: string }) {
  const { t } = useI18n()
  return <p className="text-[13px] text-[color:var(--cp-muted)]" role="note" data-testid={testId}>{t('messagehub.agentPanel.unavailable')}</p>
}

function Stat({ label, value, tone, testId }: { label: string; value: string | number; tone?: 'warning' | 'danger'; testId: string }) {
  return <div className="min-w-0 rounded-lg bg-[color:color-mix(in_srgb,var(--cp-text)_5%,transparent)] px-2 py-1.5" data-testid={testId}>
    <p className="truncate text-[15px] font-semibold tabular-nums" style={tone ? { color: `var(--cp-${tone})` } : undefined}>{value}</p>
    <p className="break-words text-[11px] leading-tight text-[color:var(--cp-muted)]">{label}</p>
  </div>
}

/** The Agent as a whole: who it is, how it is doing, its home, and whether it may notify. */
function AgentOverview({ entity, context, onEntityPreferences }: { entity: EntityDetail; context: MessageHubContext; onEntityPreferences?: (entityIds: string[], patch: EntityPreferencePatch) => Promise<void> }) {
  const { t } = useI18n()
  const now = useMessageHubClock()
  const source = getAgentPanelSource()
  const summary = usePanelResource<AgentSummary>(`summary:${entity.id}`, () => source.summary(entity.id), { live: () => false })
  const action = usePanelAction()
  const hasDesktop = desktopUIStore.getSnapshot().apps.some(app => app.id === 'users-agents')
  const openHome = () => action.run('home', async () => {
    const { data } = await fetchAgentList()
    const agent = data?.agents.find(entry => entry.agent_did === entity.id)
    if (agent) await openAgentHome(agent)
    else desktopUIStore.openAppWindow('users-agents')
  })
  const prefer = (patch: EntityPreferencePatch) => action.run('preferences', () => onEntityPreferences!([entity.id], patch))
  const value = summary.value
  return (
    <section className="space-y-3" data-testid="agent-overview">
      <div className="flex items-center gap-3">
        <HubAvatar type={entity.type} name={entity.name} avatar={entity.avatar} size={40} />
        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-1.5"><span className="truncate text-[15px] font-semibold">{entity.name}</span>{entity.isOnline ? <span className="shrink-0 rounded-full bg-[color:color-mix(in_srgb,var(--cp-success)_14%,transparent)] px-1.5 text-[11px] text-[color:var(--cp-success)]" data-testid="agent-online">{t('messagehub.agentPanel.online')}</span> : null}</p>
          <CopyValue label="DID" value={entity.id} testId="agent-did" />
        </div>
      </div>
      {summary.status === 'loading' ? <p className="text-[13px] text-[color:var(--cp-muted)]" role="status">{t('messagehub.agentPanel.loading')}</p>
        : !value ? <Unavailable testId="agent-summary-unavailable" />
        : <div data-testid="agent-summary" data-stale={summary.stale || undefined}>
          <div className="grid grid-cols-3 gap-1.5">
            <Stat label={t('messagehub.agentPanel.working')} value={value.sessions.working} testId="summary-working" />
            <Stat label={t('messagehub.agentPanel.waiting')} value={value.sessions.waiting} testId="summary-waiting" />
            <Stat label={t('messagehub.agentPanel.failed')} value={value.sessions.failed} tone={value.sessions.failed > 0 ? 'danger' : undefined} testId="summary-failed" />
            {value.sessions.interrupted > 0 ? <Stat label={t('messagehub.agentPanel.interrupted')} value={value.sessions.interrupted} tone="warning" testId="summary-interrupted" /> : null}
            <Stat label={t('messagehub.agentPanel.cognitions')} value={value.memory.cognitions} testId="summary-cognitions" />
            <Stat label={value.memory.pending > 0 ? t('messagehub.agentPanel.perceptionsPending', undefined, { count: value.memory.pending }) : t('messagehub.agentPanel.perceptions')} value={value.memory.perceptions} testId="summary-perceptions" />
          </div>
          <p className="mt-1.5 text-xs text-[color:var(--cp-muted)]" data-testid="summary-consolidation">{value.memory.last_consolidation_at_ms ? t('messagehub.agentPanel.lastConsolidation', undefined, { time: relativeActivity(value.memory.last_consolidation_at_ms, now, t('messagehub.now')) }) : t('messagehub.agentPanel.neverConsolidated')}</p>
        </div>}
      <div className="flex flex-wrap gap-2">
        {hasDesktop ? <button type="button" className={`${hubButtonClass} flex items-center gap-2`} disabled={action.pending === 'home'} onClick={() => void openHome()} data-testid="agent-open-home"><ExternalLink size={15} aria-hidden />{t('messagehub.agentPanel.openHome')}</button> : null}
        <button type="button" className={`${hubButtonClass} flex items-center gap-2`} disabled={!onEntityPreferences || context.mode === 'observe' || !!action.pending} aria-pressed={!!entity.isMuted} onClick={() => void prefer({ muted: !entity.isMuted })} data-testid="agent-mute">{entity.isMuted ? <Bell size={15} aria-hidden /> : <BellOff size={15} aria-hidden />}{t(entity.isMuted ? 'messagehub.unmute' : 'messagehub.agentPanel.mute')}</button>
        <button type="button" className={`${hubButtonClass} flex items-center gap-2`} disabled={!onEntityPreferences || context.mode === 'observe' || !!action.pending} aria-pressed={!!entity.isPinned} onClick={() => void prefer({ pinned: !entity.isPinned })} data-testid="agent-pin">{entity.isPinned ? <PinOff size={15} aria-hidden /> : <Pin size={15} aria-hidden />}{t(entity.isPinned ? 'messagehub.unpin' : 'messagehub.pin')}</button>
      </div>
      {action.error ? <p role="alert" className="text-xs text-[color:var(--cp-danger)]">{t('messagehub.agentPanel.actionFailed')}</p> : null}
    </section>
  )
}

/** This conversation: its title, what the Agent does in it, and which generation of its context this is. */
function ConversationState({ session, context, conversation, answering }: { session: Session; context: MessageHubContext; conversation: PanelResource<UiConversation>; answering: boolean }) {
  const { t } = useI18n()
  const store = useMessageHubStore()
  const description = sessionDescription(session, store.preferences(context, session.id))
  const value = conversation.value
  const state = value ? agentConversationState(value.current, answering) : null
  const tone = state === 'interrupted' ? 'var(--cp-warning)' : state === 'working' ? 'var(--cp-accent)' : 'var(--cp-muted)'
  // The runtime's fixed English status lines are shown in the viewer's language; what the model wrote stays as it is.
  const line = localizeLine(value?.current?.one_line_status ?? '', 'running')
  const status = typeof line === 'string' ? line : t(line.key, undefined, line.vars)
  return (
    <section className="space-y-2" data-testid="agent-conversation">
      <h3 className={sectionTitleClass}>{t('messagehub.agentPanel.conversation')}</h3>
      <div>
        <p className="break-words text-[15px] font-semibold leading-snug">{store.title(context, session)}</p>
        {description ? <p className="mt-1 whitespace-pre-wrap break-words text-[13px] text-[color:var(--cp-muted)]">{description}</p> : null}
      </div>
      {conversation.status === 'loading' ? <p className="text-[13px] text-[color:var(--cp-muted)]" role="status">{t('messagehub.agentPanel.loading')}</p>
        : !value || !state ? <Unavailable testId="agent-conversation-unavailable" />
        : <div className="space-y-1" data-testid="agent-conversation-state" data-state={state} data-stale={conversation.stale || undefined}>
          <p className="flex items-center gap-2 text-[13px]"><span className="h-2 w-2 shrink-0 rounded-full" style={{ background: tone }} aria-hidden /><span className="font-medium">{t(`messagehub.agentPanel.state.${state}`)}</span></p>
          {state === 'interrupted' ? <p className="text-xs text-[color:var(--cp-muted)]">{t('messagehub.agentPanel.interruptedHint')}</p> : null}
          {status && state === 'working' ? <p className="truncate text-xs text-[color:var(--cp-muted)]" title={status} data-testid="agent-status-line">{status}</p> : null}
          <p className="text-xs text-[color:var(--cp-muted)]" data-testid="agent-generation">{t('messagehub.agentPanel.generation', undefined, { number: generationNumber(value) })}</p>
        </div>}
    </section>
  )
}

/**
 * The unfinished work under this conversation's UI Session, as a tree: a
 * plan with its TodoList open, what is under a Todo's session folded. Each
 * node can be paused, resumed and stopped; nodes a host restart interrupted
 * can be resumed together.
 */
function SubSessions({ agentDid, context, sessionId, uiSessionId, hintTaskId, signal, onOpenWorklog, onChanged }: { agentDid: string; context: MessageHubContext; sessionId: string; uiSessionId: string | null; hintTaskId?: string; signal: string; onOpenWorklog: (destination: WorklogDestination) => void; onChanged: () => void }) {
  const { t } = useI18n()
  const source = getAgentPanelSource()
  const tree = usePanelResource<WorkTree>(uiSessionId ? `tree:${agentDid}:${uiSessionId}` : null, () => source.workTree(agentDid, uiSessionId!), { live: value => !value.complete, hintTaskId, signal })
  const [stopped, setStopped] = useState<{ key: string; cards: WorkCard[] }>({ key: '', cards: [] })
  const stoppedHere = stopped.key === uiSessionId ? stopped.cards : []
  const sessions = tree.value?.sessions ?? []
  const now = useSecondClock(sessions.some(cardRunning))
  const action = usePanelAction()
  const interrupted = interruptedRoots(sessions)
  const changed = () => { tree.refresh(); onChanged() }
  const listContext: WorkListContext = {
    now, stale: tree.stale, viewerDid: context.viewerDid, anchorId: `panel:${sessionId}`,
    onOpen: (card, title) => onOpenWorklog({ target: { agentDid, sessionId: card.session_id }, title, card }),
    actions: (card, title) => <WorkControls agentDid={agentDid} card={card} title={title} onChanged={() => {
      // A stopped node leaves the list of unfinished work: it stays below with a way to its record.
      if (!isWorkFinished(card.state)) void source.workTree(agentDid, uiSessionId!).then(next => {
        if (!flattenWork(next.sessions).some(item => item.session_id === card.session_id)) setStopped(previous => ({ key: uiSessionId!, cards: [...(previous.key === uiSessionId ? previous.cards : []).filter(item => item.session_id !== card.session_id), { ...card, state: 'stopped', paused: undefined, finished_at_ms: Date.now(), todos: null, children: [] }] }))
      }).catch(() => undefined)
      changed()
    }} />,
  }
  const resumeAll = () => action.run('resume-all', async () => {
    for (const card of interrupted) await source.resume(agentDid, card.session_id)
    changed()
  })
  return (
    <section className="space-y-2" data-testid="agent-sub-sessions">
      <h3 className={sectionTitleClass}>{t('messagehub.agentPanel.subSessions')}</h3>
      {!uiSessionId ? <p className="text-[13px] text-[color:var(--cp-muted)]" data-testid="sub-sessions-empty">{t('messagehub.agentPanel.noWork')}</p>
        : tree.status === 'loading' ? <p className="text-[13px] text-[color:var(--cp-muted)]" role="status">{t('messagehub.agentPanel.loading')}</p>
        : !tree.value ? <Unavailable testId="sub-sessions-unavailable" />
        : <>
          {interrupted.length > 0 ? <div className="flex items-center gap-2 rounded-xl bg-[color:color-mix(in_srgb,var(--cp-warning)_10%,transparent)] px-3 py-2 text-[13px]" role="note" data-testid="sub-sessions-interrupted">
            <span className="min-w-0 flex-1">{t('messagehub.agentPanel.interruptedWorks', undefined, { count: flattenWork(sessions).filter(card => card.paused === 'interrupted').length })}</span>
            <button type="button" className="mh-turn-action shrink-0" disabled={!!action.pending} onClick={() => void resumeAll()} data-testid="sub-sessions-resume-all"><Play size={11} aria-hidden />{t(action.pending ? 'messagehub.turn.resuming' : 'messagehub.agentPanel.resumeAll')}</button>
          </div> : null}
          {action.error ? <p role="alert" className="text-xs text-[color:var(--cp-danger)]">{t('messagehub.agentPanel.actionFailed')}</p> : null}
          {sessions.length === 0 && stoppedHere.length === 0 ? <p className="text-[13px] text-[color:var(--cp-muted)]" data-testid="sub-sessions-empty">{t('messagehub.agentPanel.noWork')}</p> : null}
          {sessions.length > 0 ? <div className="mh-work space-y-0.5 rounded-xl p-1" data-testid="work-tree" data-complete={tree.value.complete} data-stale={tree.stale || undefined}><WorkCardList cards={sessions} context={listContext} /></div> : null}
          {stoppedHere.length > 0 ? <ul className="space-y-1" data-testid="work-stopped">{stoppedHere.map(card => <li key={card.session_id} className="flex items-center gap-2 px-2 text-[12px] text-[color:var(--cp-muted)]" data-session-id={card.session_id}>
            <span className="min-w-0 flex-1 truncate">{t('messagehub.agentPanel.stoppedWork', undefined, { title: card.title })}</span>
            <StoppedWorkLink agentDid={agentDid} />
          </li>)}</ul> : null}
        </>}
    </section>
  )
}

/** A stopped session is no longer in the tree; its record (and deleting it) is on the Agent's home. */
function StoppedWorkLink({ agentDid }: { agentDid: string }) {
  const { t } = useI18n()
  if (!desktopUIStore.getSnapshot().apps.some(app => app.id === 'users-agents')) return null
  const open = async () => {
    const { data } = await fetchAgentList()
    const agent = data?.agents.find(entry => entry.agent_did === agentDid)
    if (agent) await openAgentHome(agent)
  }
  return <button type="button" className="shrink-0 underline" onClick={() => void open()} data-testid="work-stopped-home">{t('messagehub.agentPanel.viewOnHome')}</button>
}

/** "Start a new conversation": the Agent's UI Session is archived and the next message starts from a fresh context; no message is deleted. */
function NewConversation({ agentDid, session, conversation, onRotating }: { agentDid: string; session: Session; conversation: PanelResource<UiConversation>; onRotating: (busy: boolean) => void }) {
  const { t } = useI18n()
  const dialog = useWindowDialog()
  const action = usePanelAction()
  const current = conversation.value?.current ?? null
  const works = current?.unfinished_works ?? 0
  const rotating = useRef(onRotating)
  useEffect(() => { rotating.current = onRotating })
  useEffect(() => () => rotating.current(false), [])
  if (session.lifecycle === 'archived' || conversation.status !== 'ready') return null
  const start = async () => {
    if (!current) return
    if (!await confirmGroupAction(dialog, t, { title: t('messagehub.agentPanel.newConversation'), body: t('messagehub.agentPanel.newConversationConfirm'), confirm: t('messagehub.agentPanel.newConversation') })) return
    onRotating(true)
    await action.run('rotate', () => getAgentPanelSource().rotate(agentDid, current.session_id))
    onRotating(false)
    conversation.refresh()
  }
  const reason = !current ? t('messagehub.agentPanel.alreadyNew') : works > 0 ? t('messagehub.agentPanel.newConversationBlocked', undefined, { count: works }) : ''
  return (
    <section className="space-y-2" data-testid="agent-new-conversation">
      <h3 className={sectionTitleClass}>{t('messagehub.agentPanel.control')}</h3>
      <button type="button" className={hubButtonClass} disabled={!current || works > 0 || !!action.pending} onClick={() => void start()} data-testid="agent-rotate">{t(action.pending ? 'messagehub.agentPanel.newConversationPending' : 'messagehub.agentPanel.newConversation')}</button>
      <p className="text-xs text-[color:var(--cp-muted)]" data-testid="agent-rotate-note">{reason || t('messagehub.agentPanel.newConversationHint')}</p>
      {action.error ? <p role="alert" className="break-words text-xs text-[color:var(--cp-danger)]" data-testid="agent-rotate-error">{t('messagehub.agentPanel.newConversationFailed')}</p> : null}
    </section>
  )
}

/** Whether the session still runs on the Agent's current prompt (identity, behaviors, skills). */
function PromptVersionNote({ conversation }: { conversation: PanelResource<UiConversation> }) {
  const { t } = useI18n()
  const prompt = conversation.value?.current?.prompt
  if (conversation.status === 'loading') return <p className="text-[13px] text-[color:var(--cp-muted)]" role="status">{t('messagehub.agentPanel.loading')}</p>
  if (!conversation.value) return <Unavailable testId="agent-prompt-unavailable" />
  if (!prompt) return <p className="text-[13px] text-[color:var(--cp-muted)]" data-testid="agent-prompt" data-current="none">{t('messagehub.agentPanel.promptNone')}</p>
  return (
    <div className="space-y-1 text-[13px]" data-testid="agent-prompt" data-current={prompt.current}>
      <p className="font-medium">{t(prompt.current ? 'messagehub.agentPanel.promptCurrent' : 'messagehub.agentPanel.promptChanged')}</p>
      {prompt.current ? null : <p className="text-xs text-[color:var(--cp-muted)]">{t('messagehub.agentPanel.promptChangedHint')}</p>}
      {!prompt.current && prompt.changed.length > 0 ? <ul className="list-disc space-y-0.5 pl-5 text-xs text-[color:var(--cp-muted)]" data-testid="agent-prompt-changed">{prompt.changed.map(item => <li key={item} className="break-all">{item}</li>)}</ul> : null}
      {prompt.frozen_at_ms ? <p className="text-xs text-[color:var(--cp-muted)]">{t('messagehub.agentPanel.promptFrozenAt', undefined, { time: new Date(prompt.frozen_at_ms).toLocaleString() })}</p> : null}
    </div>
  )
}

function DeliveryIcon({ delivery }: { delivery: Delivery }) {
  const kind = attachmentKindOf(delivery.mime, delivery.name)
  const props = { size: 18, 'aria-hidden': true, className: 'shrink-0 text-[color:var(--cp-muted)]' }
  if (kind === 'image') return <Image {...props} />
  if (kind === 'video') return <Film {...props} />
  if (kind === 'audio') return <Music {...props} />
  return /^text\/|pdf|json|xml/.test(delivery.mime ?? '') ? <FileText {...props} /> : <File {...props} />
}

/** The message of the loaded conversation that carries `objId`. */
function carrierOf(timeline: readonly MessageObject[], objId: string): MessageObject | undefined {
  return timeline.findLast(message => !messageRelations(message)?.redacted && (effectiveContent(message).refs ?? []).some(ref => ref.target.type === 'data_obj' && ref.target.obj_id === objId))
}

/**
 * What the Agent delivered lately, newest first: everything, or only what
 * this conversation (its UI Session generations and the work under them)
 * delivered. Files are read through their binding in the Agent's
 * `artifacts` bucket, which is the owner's own.
 */
function Deliveries({ agentDid, routeKey, timeline, context, onLocateMessage, onForward }: { agentDid: string; routeKey: string | null; timeline: readonly MessageObject[]; context: MessageHubContext; onLocateMessage: (messageId: string) => void; onForward: (message: MessageObject) => void }) {
  const { t } = useI18n()
  const now = useMessageHubClock()
  const [open, setOpen] = useState(false)
  const [tab, setTab] = useState<'all' | 'conversation'>('conversation')
  const scope = tab === 'conversation' ? routeKey : ''
  const key = `${agentDid}\n${scope ?? ''}`
  const source = getAgentPanelSource()
  const query = (before?: number) => source.deliveries(agentDid, { limit: DELIVERY_PAGE, ...(before !== undefined ? { before } : {}), ...(scope ? { routeKey: scope } : {}) })
  // The first page is read when the section opens (and again when the scope changes); later pages on scroll.
  const first = usePanelResource(open && scope !== null ? `deliveries:${key}` : null, () => query(), { live: () => false })
  const [older, setOlder] = useState<{ key: string; entries: Delivery[]; next?: number; loading: boolean }>({ key: '', entries: [], loading: false })
  const more = older.key === key ? older : { key, entries: [], next: first.value?.next_before, loading: false }
  const entries = [...(first.value?.entries ?? []), ...more.entries]
  const loadMore = () => {
    if (more.next === undefined || more.loading) return
    setOlder({ ...more, loading: true })
    query(more.next).then(
      page => setOlder(previous => previous.key === key ? { key, entries: [...previous.entries, ...page.entries], next: page.next_before, loading: false } : previous),
      () => setOlder(previous => previous.key === key ? { ...previous, loading: false } : previous),
    )
  }
  const onScroll = (event: React.UIEvent<HTMLUListElement>) => {
    const list = event.currentTarget
    if (list.scrollTop + list.clientHeight >= list.scrollHeight - 48) loadMore()
  }
  return (
    <details className="rounded-xl border border-[color:var(--cp-border)] px-3" open={open} onToggle={event => setOpen(event.currentTarget.open)} data-testid="agent-deliveries">
      <summary className="flex min-h-11 cursor-pointer items-center text-[13px] font-medium text-[color:var(--cp-muted)]">{t('messagehub.agentPanel.deliveries')}</summary>
      {open ? <div className="space-y-2 pb-3">
        <div className="flex gap-4 border-b border-[color:var(--cp-border)]" role="tablist" aria-label={t('messagehub.agentPanel.deliveries')}>
          {(['conversation', 'all'] as const).map(value => <button key={value} type="button" role="tab" aria-selected={tab === value} className={`border-b-2 py-2 text-xs font-medium ${tab === value ? 'border-[color:var(--cp-accent)] text-[color:var(--cp-accent)]' : 'border-transparent text-[color:var(--cp-muted)]'}`} onClick={() => setTab(value)} data-testid={`deliveries-tab-${value}`}>{t(`messagehub.agentPanel.deliveriesTab.${value}`)}</button>)}
        </div>
        {scope === null ? <Unavailable testId="deliveries-unavailable" />
          : first.status === 'loading' ? <p className="text-[13px] text-[color:var(--cp-muted)]" role="status">{t('messagehub.agentPanel.loading')}</p>
          : first.status === 'unavailable' ? <p className="flex items-center gap-2 text-[13px] text-[color:var(--cp-muted)]" role="note" data-testid="deliveries-unavailable">{t('messagehub.agentPanel.unavailable')}<button type="button" className="flex h-8 w-8 items-center justify-center" aria-label={t('messagehub.retry')} title={t('messagehub.retry')} onClick={first.refresh}><RefreshCw size={14} /></button></p>
          : entries.length === 0 ? <p className="text-[13px] text-[color:var(--cp-muted)]" data-testid="deliveries-empty">{t('messagehub.agentPanel.noDeliveries')}</p>
          : <ul className="shell-scrollbar max-h-72 space-y-0.5 overflow-y-auto" onScroll={onScroll} data-testid="deliveries-list">
            {entries.map(delivery => <DeliveryRow key={`${delivery.session_id}:${delivery.obj_id}:${delivery.at_ms}`} delivery={delivery} agentDid={agentDid} context={context} now={now} carrier={carrierOf(timeline, delivery.obj_id)} onLocateMessage={onLocateMessage} onForward={onForward} />)}
            {more.loading ? <li className="py-1 text-center text-xs text-[color:var(--cp-muted)]" role="status">{t('messagehub.agentPanel.loading')}</li> : null}
          </ul>}
      </div> : null}
    </details>
  )
}

function DeliveryRow({ delivery, agentDid, context, now, carrier, onLocateMessage, onForward }: { delivery: Delivery; agentDid: string; context: MessageHubContext; now: number; carrier?: MessageObject; onLocateMessage: (messageId: string) => void; onForward: (message: MessageObject) => void }) {
  const { t } = useI18n()
  const [note, setNote] = useState<'collected' | 'failed' | ''>('')
  const owner = context.mode === 'self' && context.ownerDid === context.viewerDid
  const carrierContext = carrier ? objectContextOf(carrier) : undefined
  const collect = getObjectAccess()?.collect
  const url = delivery.url
  const open = () => { if (url) openPreview({ source: bucketObjectRef(url), origin: { app: 'messagehub' } }) }
  const download = () => {
    const anchor = document.createElement('a')
    if (!url) return
    anchor.href = bucketObjectUrl(url)
    anchor.download = delivery.name
    anchor.rel = 'noopener'
    anchor.click()
  }
  const forward = () => onForward({
    from: agentDid, to: [context.ownerDid], kind: 'chat', created_at_ms: delivery.at_ms,
    content: { format: 'text/plain', content: '', refs: [{ role: 'output', label: delivery.name, target: { type: 'data_obj', obj_id: delivery.obj_id, ...(url ? { uri_hint: url } : {}) } }] },
    ui_message_id: `delivery:${delivery.obj_id}`,
  })
  const items: HeaderMenuItem[] = [
    { key: 'download', label: t('messagehub.agentPanel.delivery.download'), icon: <Download size={16} />, onSelect: download, disabled: !url, testId: 'delivery-download' },
    { key: 'collect', label: t(note === 'collected' ? 'messagehub.media.collected' : 'messagehub.agentPanel.delivery.collect'), icon: <Star size={16} />, onSelect: () => { collect!(delivery.obj_id, carrierContext!).then(() => setNote('collected'), () => setNote('failed')) }, disabled: !owner || !collect || !carrierContext, testId: 'delivery-collect' },
    { key: 'forward', label: t('messagehub.message.forward'), icon: <Forward size={16} />, onSelect: forward, disabled: !owner, testId: 'delivery-forward' },
    { key: 'locate', label: t('messagehub.agentPanel.delivery.locate'), icon: <LocateFixed size={16} />, onSelect: () => onLocateMessage(messageObjId(carrier!)!), disabled: !carrier || !messageObjId(carrier), testId: 'delivery-locate' },
  ]
  return (
    <li className="flex items-center gap-2 rounded-lg px-1 hover:bg-[color:color-mix(in_srgb,var(--cp-text)_5%,transparent)]" data-testid="delivery-row" data-obj-id={delivery.obj_id} data-source={delivery.source}>
      <button type="button" className="flex min-h-11 min-w-0 flex-1 items-center gap-2 text-left" disabled={!url} onClick={open} title={t('messagehub.agentPanel.delivery.open', undefined, { name: delivery.name })} data-testid="delivery-open">
        <DeliveryIcon delivery={delivery} />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[13px]">{delivery.name}</span>
          <span className="block truncate text-[11px] text-[color:var(--cp-muted)]">{[relativeActivity(delivery.at_ms, now, t('messagehub.now')), delivery.size !== undefined ? formatSize(delivery.size) : '', t(`messagehub.agentPanel.delivery.source.${delivery.source}`), delivery.session_id].filter(Boolean).join(' · ')}{note === 'failed' ? ` · ${t('messagehub.media.collectFailed')}` : ''}</span>
        </span>
      </button>
      <HeaderMenu label={`${t('messagehub.agentPanel.delivery.more')}: ${delivery.name}`} icon={<MoreHorizontal size={16} />} items={items} testId="delivery-menu" buttonClassName="flex min-h-9 min-w-9 shrink-0 items-center justify-center rounded-lg text-[color:var(--cp-muted)]" />
    </li>
  )
}
