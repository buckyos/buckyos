import {
  ArrowLeft,
  Bell,
  BellOff,
  Eraser,
  Info,
  Loader2,
  Menu,
  MoreVertical,
  Pencil,
} from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { notifyCategoryFromSessionId } from 'buckyos'
import { useI18n } from '../../i18n/provider'
import { ConversationSurface, type ConversationHeaderControls } from '../../components/conversation/ConversationSurface'
import type { ConversationHostActions } from '../../components/conversation/messageActions'
import { useNotifyFilteredReader } from './notify'
import { NOTIFY_FILTERS, type NotifyFilter } from './notify/model'
import type { Entity, Session, MessageHubContext } from './types'
import { useMessageHubRuntime, useMessageHubStore } from './store'
import { useMessageHubConversationSource } from './conversationSource'
import { RequestBanner } from './RelationPanel'
import { HubAvatar } from './HubAvatar'
import { HeaderMenu, type HeaderMenuItem } from './EntityListHeader'

interface ConversationViewProps extends ConversationHostActions {
  entity: Entity
  session: Session | null
  context: MessageHubContext
  /** The write confirmation of a tunnel session, and whether the session's binding is confirmed. */
  confirmation?: string
  confirmed?: boolean
  onBack: () => void
  onOpenSessionSidebar: () => void
  onOpenDetails: () => void
  title?: string
  onOpenSessionDetails?: () => void
  creationReason?: string
  /** The session's notifications are muted for the owner. */
  muted?: boolean
  /** The session menu's mute / unmute; without it the entry is not offered. */
  onToggleMute?: () => void
  /** The session menu's "Clear history" (the host confirms); without it the entry is not offered. */
  onClearHistory?: () => void
  /** Which session opens is still being resolved. */
  defaultSessionPending?: boolean
  defaultSessionError?: boolean
  /** Texts carried out as commands instead of being sent. */
  commands?: Record<string, () => void>
  onRetryDefaultSession?: () => void
  leadingPane?: ReactNode
  isSessionSidebarOpen?: boolean
  /** Unread messages of the entity's other sessions, shown on the session list button. */
  otherSessionsUnread?: number
  /** Extra content of the session panel, shown when it is expanded (host-defined). */
  sessionPanelDetails?: ReactNode
  /** ObjId of a message to bring into view (a deep link, or the message whose details are open). */
  focusMessageId?: string | null
  /**
   * The conversation is with one of the viewer's own Agents: its header opens
   * the Agent panel (`open`: the panel is showing; the Info button toggles
   * it), the viewer controls the Agent's Turns, and nothing is sent while
   * the Agent handles the message in front.
   */
  agentPanel?: { open: boolean; onToggle: () => void }
  /** Why nothing may be sent right now, in words (the conversation is being archived). */
  sendBlockedReason?: string
}

/** MessageHub's conversation: its header (entity, sessions, search, Agent panel, ⋮ menu) around the conversation component. */
export function ConversationView({
  entity,
  session,
  context,
  confirmation,
  confirmed,
  onBack,
  onOpenSessionSidebar,
  onOpenDetails,
  title = session?.title ?? '', onOpenSessionDetails, creationReason, muted = false, onToggleMute, onClearHistory, defaultSessionPending = false, defaultSessionError, onRetryDefaultSession, commands,
  leadingPane = null,
  isSessionSidebarOpen = false,
  otherSessionsUnread = 0,
  sessionPanelDetails,
  focusMessageId = null,
  agentPanel,
  sendBlockedReason,
  onOpenMessage,
  onForward,
  onOpenEntity,
  onOpenProfile,
}: ConversationViewProps) {
  const { t } = useI18n()
  const store = useMessageHubStore()
  useMessageHubRuntime()
  const source = useMessageHubConversationSource({ context, entityId: entity.id, session, confirmation, confirmed, creationReason })
  const ownerStatus = store.ownerStatus(context)
  const refreshing = ownerStatus.phase === 'ready' && ownerStatus.refreshing
  const requestCount = session?.requestCount ?? 0
  // A notification session (`sysnotify:*`) is not a conversation: its cards are filtered, never answered by typing.
  const notifyCategory = session ? notifyCategoryFromSessionId(session.id) : undefined
  const [notifyFilter, setNotifyFilter] = useState<NotifyFilter>('all')
  const notifyReader = useNotifyFilteredReader(source.reader, notifyCategory ? notifyFilter : 'all')
  const subtitle = session ? [...new Set([title, session.binding.kind === 'tunnel' ? session.binding.connectionName : ''])].filter(Boolean).join(' · ') : defaultSessionPending ? '' : t('messagehub.noSessions')
  const isOwner = context.mode === 'self' && context.ownerDid === context.viewerDid
  const mutedMark = muted ? <BellOff size={13} className="shrink-0 text-[color:var(--cp-muted)]" role="img" aria-label={t('messagehub.sessionMenu.muted')} data-testid="session-muted" /> : null
  const menuItems: HeaderMenuItem[] = session ? [
    { key: 'edit-info', label: t('messagehub.sessionMenu.editInfo'), icon: <Pencil size={16} />, onSelect: () => onOpenSessionDetails?.(), disabled: !onOpenSessionDetails, testId: 'session-menu-edit-info' },
    ...(onToggleMute ? [{ key: 'mute', label: t(muted ? 'messagehub.unmute' : 'messagehub.mute'), icon: muted ? <Bell size={16} /> : <BellOff size={16} />, onSelect: onToggleMute, disabled: !isOwner, testId: 'session-menu-mute' }] : []),
    ...(onClearHistory && !notifyCategory ? [{ key: 'clear-history', label: t('messagehub.sessionMenu.clearHistory'), icon: <Eraser size={16} />, onSelect: onClearHistory, disabled: !isOwner || source.reader.totalCount === 0, danger: true, testId: 'session-menu-clear-history' }] : []),
  ] : []

  const header = ({ searchButton }: ConversationHeaderControls) => (
        <div
          className="flex items-center gap-1 px-2 py-1.5 flex-shrink-0 md:gap-2 md:px-3"
          style={{
            borderBottom: '1px solid var(--cp-border)',
            background: 'var(--cp-surface)',
          }}
        >
          <button
            onClick={onBack}
            className="-ml-2 flex min-h-11 min-w-11 items-center justify-center rounded-lg md:hidden"
            style={{ color: 'var(--cp-accent)' }}
            type="button"
            aria-label={t('messagehub.backToList')}
            title={t('messagehub.backToList')}
          >
            <ArrowLeft size={20} />
          </button>

          <button onClick={onOpenSessionSidebar} aria-label={t('messagehub.sessions')} title={otherSessionsUnread > 0 ? t('messagehub.otherSessionsUnread', '{{count}} unread in other sessions', { count: otherSessionsUnread }) : t('messagehub.sessions')} className="relative flex min-h-11 min-w-11 items-center justify-center" style={{ color: isSessionSidebarOpen ? 'var(--cp-accent)' : 'var(--cp-muted)' }} type="button"><Menu size={18} />{otherSessionsUnread > 0 && <span data-testid="other-sessions-unread" className="absolute right-0.5 top-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-[color:var(--cp-accent)] px-1 text-[10px] font-semibold leading-none tabular-nums text-[color:var(--cp-surface-opaque)]">{otherSessionsUnread}</span>}</button>
          <div className="min-w-0 flex-1">
            {/* On touch layouts the whole title block is one entity target; session
                details open from the ⋮ menu. */}
            <button onClick={onOpenDetails} className="flex min-h-11 w-full min-w-0 flex-col justify-center text-left md:hidden" type="button" aria-label={`${t('messagehub.entityDetails')}: ${entity.name}`}>
              <span className="flex max-w-full items-center gap-1.5"><HubAvatar type={entity.type} name={entity.name} did={entity.id} avatar={entity.avatar} size={20} systemNotify={entity.systemNotify} /><span className="truncate text-base font-medium">{entity.name}</span>{mutedMark}</span>
              <span className="block max-w-full truncate text-xs text-[color:var(--cp-muted)]">{subtitle}</span>
            </button>
            <button onClick={onOpenDetails} className="hidden max-w-full items-center gap-1.5 text-left md:flex" type="button" aria-label={`${t('messagehub.entityDetails')}: ${entity.name}`} title={t('messagehub.entityDetails')}><HubAvatar type={entity.type} name={entity.name} did={entity.id} avatar={entity.avatar} size={22} systemNotify={entity.systemNotify} /><span className="truncate text-[15px] font-medium">{entity.name}</span>{mutedMark}</button>
            <button onClick={agentPanel ? onOpenDetails : onOpenSessionDetails} disabled={!session} type="button" className="hidden max-w-full truncate text-xs text-[color:var(--cp-muted)] md:block" aria-label={t('messagehub.sessionDetails')} title={t('messagehub.sessionDetails')}>{subtitle}</button>
            <div role="status" data-testid="session-runtime" className="truncate text-xs text-[color:var(--cp-accent)]">{source.runtime.map(state => `${state.memberDid === context.ownerDid ? t('messagehub.you') : session?.members[state.memberDid]?.nickname || entity.name} · ${t(`messagehub.runtime.${state.status}`)}${state.statusLine ? ` · ${state.statusLine}` : ''}`).join(' · ')}</div>
          </div>
          {refreshing ? <span role="status" className="flex min-w-0 max-w-28 items-center gap-1.5 text-xs text-[color:var(--cp-muted)]" data-testid="conversation-refresh-status"><Loader2 size={14} className="shrink-0 animate-spin" aria-hidden /><span className="truncate">{t('states.loading')}</span></span> : null}
          {searchButton}
          {agentPanel ? <button type="button" onClick={agentPanel.onToggle} aria-pressed={agentPanel.open} title={t('messagehub.agentPanel.toggle')} aria-label={t('messagehub.agentPanel.toggle')} className="hidden min-h-11 min-w-11 items-center justify-center rounded-lg md:flex" style={{ color: agentPanel.open ? 'var(--cp-accent)' : 'var(--cp-muted)' }} data-testid="agent-panel-toggle"><Info size={18} /></button> : null}
          <HeaderMenu label={t('messagehub.sessionMenu.label')} icon={<MoreVertical size={18} />} items={menuItems} testId="session-menu" buttonClassName="flex min-h-11 min-w-11 items-center justify-center rounded-lg text-[color:var(--cp-muted)] disabled:opacity-40" />
        </div>
  )

  // On a touch layout a swipe to the left over the history opens the Agent panel (a second way in beside the title).
  return (
    <ConversationSurface
      source={source}
      features={{ search: !notifyCategory }}
      header={header}
      banner={<>
        {session ? <RequestBanner entity={entity} context={context} requestCount={requestCount} /> : null}
        {notifyCategory ? <div role="group" aria-label={t('messagehub.notify.filter.label')} className="flex shrink-0 items-center gap-1.5 overflow-x-auto border-b border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 py-1.5" data-testid="notify-filter">
          {NOTIFY_FILTERS.map(value => <button key={value} type="button" className="mh-notify-filter" aria-pressed={notifyFilter === value} onClick={() => setNotifyFilter(value)} data-filter={value}>{t(`messagehub.notify.filter.${value}`)}</button>)}
        </div> : null}
        {defaultSessionError && <div role="alert" className="flex shrink-0 items-center gap-2 px-3 py-2 text-xs text-[color:var(--cp-danger)]"><span>{t('messagehub.operationFailed')}</span><button type="button" className="min-h-8 rounded-lg border border-[color:var(--cp-border)] px-2" onClick={onRetryDefaultSession}>{t('messagehub.retry')}</button></div>}
      </>}
      leadingPane={leadingPane}
      panelDetails={sessionPanelDetails}
      historyReader={notifyCategory ? notifyReader : undefined}
      composer={!notifyCategory && !defaultSessionPending}
      commands={commands}
      emptyLabel={!session ? t(defaultSessionPending ? 'messagehub.loadingHistory' : 'messagehub.noSessions') : notifyCategory ? t(`messagehub.notify.empty.${notifyFilter}`) : undefined}
      sendBlockedReason={sendBlockedReason}
      focusMessageId={focusMessageId}
      onSwipeLeft={agentPanel && !agentPanel.open ? onOpenDetails : undefined}
      onOpenMessage={onOpenMessage}
      onForward={onForward}
      onOpenEntity={onOpenEntity}
      onOpenProfile={onOpenProfile}
    />
  )
}

