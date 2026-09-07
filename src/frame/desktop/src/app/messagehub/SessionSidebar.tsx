import { Archive, MessageSquare, Send, SquarePen, Trash2, X } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { useMessageHubClock } from './store'
import { relativeActivity } from './sessionModel'
import type { Session } from './types'

interface SessionSidebarProps {
  sessions: Session[]
  activeSessionId: string | null
  onSelectSession: (id: string) => void
  onClose: () => void
  onCreate?: () => void
  onManage?: (session: Session) => void
  onToggleArchived?: () => void
  archived?: boolean
  archivedCount?: number
  canManage?: boolean
  creationReason?: string
  titleFor?: (session: Session) => string
  statusFor?: (session: Session) => string
  showHeader?: boolean
}

export function SessionSidebar({ sessions, activeSessionId, onSelectSession, onClose, onCreate, onManage, onToggleArchived, archived = false, archivedCount = 0, canManage = false, creationReason, titleFor = session => session.title, statusFor, showHeader = true }: SessionSidebarProps) {
  const { t } = useI18n()
  const now = useMessageHubClock()
  return <div className="flex h-full flex-col bg-[color:var(--cp-surface)]" data-testid="session-sidebar">
    {showHeader && <div className="flex items-center justify-between px-4 py-2">
      <h2 className="text-sm font-semibold">{t('messagehub.sessions')}</h2>
      <button type="button" className="min-h-11 min-w-11" aria-label={t('messagehub.close')} onClick={onClose}><X size={18} /></button>
    </div>}
    {onCreate && <div className="flex items-center gap-1 px-2 py-2 border-b border-[color:var(--cp-border)]">
      <button type="button" onClick={onToggleArchived} aria-pressed={archived} className="flex min-h-11 flex-1 items-center gap-1 text-xs"><Archive size={15} />{archived ? t('messagehub.activeSessions') : `${t('messagehub.archived')} (${archivedCount})`}</button>
      <button type="button" onClick={onCreate} disabled={!!creationReason} title={creationReason ? t(`messagehub.reason.${creationReason}`) : t('messagehub.newSession')} aria-label={t('messagehub.newSession')} className="min-h-11 min-w-11 disabled:opacity-40"><SquarePen size={18} /></button>
    </div>}
    <div className="flex-1 overflow-y-auto px-2 py-2 shell-scrollbar">
      {sessions.length === 0 && <p className="p-3 text-sm text-[color:var(--cp-muted)]">{t('messagehub.noSessions')}</p>}
      {sessions.map(session => <div key={session.id} data-testid="session-row" data-session-id={session.id} className="group mb-1 flex items-center rounded-lg" style={{ background: session.id === activeSessionId ? 'color-mix(in srgb, var(--cp-accent) 10%, transparent)' : undefined }}>
        <button type="button" onClick={() => onSelectSession(session.id)} aria-current={session.id === activeSessionId ? 'true' : undefined} className="flex min-h-11 min-w-0 flex-1 items-center gap-2 p-2 text-left">
          <span role="img" aria-label={session.source ?? 'BuckyOS'} className="shrink-0 text-[color:var(--cp-accent)]">{session.binding.kind === 'tunnel' ? <Send size={14} /> : <MessageSquare size={14} />}</span>
          <span className="min-w-0 flex-1"><span className={`block truncate text-sm ${session.id === activeSessionId ? 'font-semibold' : 'font-medium'}`}>{titleFor(session)}{statusFor?.(session) && <span role="img" aria-label={statusFor(session)} className="ml-1 text-[color:var(--cp-accent)]">•••</span>}</span>{session.binding.kind === 'tunnel' && <span className="block truncate text-[10px] text-[color:var(--cp-muted)]">{session.binding.connectionName}</span>}</span>
          {(session.requestCount ?? 0) > 0 && <span className="rounded-full bg-[color:color-mix(in_srgb,var(--cp-warning)_18%,transparent)] px-1.5 text-[10px] text-[color:var(--cp-warning)]" title={t('messagehub.requests')}>{t('messagehub.requestShort')}</span>}
          {session.unreadCount > 0 && <span className="text-xs">{session.unreadCount}</span>}
        </button>
        <time className="w-8 shrink-0 text-right text-[11px] text-[color:var(--cp-muted)]" title={session.lastActiveAt ? new Date(session.lastActiveAt).toLocaleString() : undefined}>{relativeActivity(session.lastActiveAt, now, t('messagehub.now'))}</time>
        <button type="button" disabled={!canManage} aria-label={`${t('messagehub.manageSession')}: ${titleFor(session)}`} onClick={() => onManage?.(session)} className="flex min-h-11 w-11 shrink-0 items-center justify-center opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 [@media(hover:none)]:opacity-100 max-md:opacity-100 disabled:invisible"><Trash2 size={14} /></button>
      </div>)}
    </div>
  </div>
}
