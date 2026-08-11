import {
  Blocks,
  ListTodo,
  MessageSquare,
  Send,
  SquarePen,
  X,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useI18n } from '../../i18n/provider'
import type { Session } from './types'

interface SessionSidebarProps {
  sessions: Session[]
  activeSessionId: string | null
  onSelectSession: (id: string) => void
  onClose: () => void
  showHeader?: boolean
}

function getSessionLeadingVisual(session: Session): {
  icon: ReactNode
  color: string
} {
  if (session.source === 'telegram') {
    return {
      icon: <Send size={14} />,
      color: '#2498eb',
    }
  }

  if (session.source === 'linear' || session.type === 'task') {
    return {
      icon: <ListTodo size={14} />,
      color: 'var(--cp-warning)',
    }
  }

  if (session.type === 'workspace') {
    return {
      icon: <Blocks size={14} />,
      color: 'var(--cp-success)',
    }
  }

  return {
    icon: <MessageSquare size={14} />,
    color: 'var(--cp-accent)',
  }
}

export function SessionSidebar({
  sessions,
  activeSessionId,
  onSelectSession,
  onClose,
  showHeader = true,
}: SessionSidebarProps) {
  const { t } = useI18n()

  return (
    <div
      className="relative flex h-full flex-col"
      style={{ background: 'var(--cp-surface)' }}
    >
      {showHeader ? (
        <div
          className="flex items-center justify-between px-4 py-3 flex-shrink-0"
        >
          <h2
            className="text-sm font-semibold"
            style={{ color: 'var(--cp-text)' }}
          >
            {t('messagehub.sessions', 'Sessions')}
          </h2>
          <button
            onClick={onClose}
            className="p-1 rounded-lg"
            style={{ color: 'var(--cp-muted)' }}
            type="button"
          >
            <X size={18} />
          </button>
        </div>
      ) : null}

      {/* Session list */}
      <div className={`flex-1 overflow-y-auto px-2 pb-24 shell-scrollbar ${showHeader ? '' : 'pt-2'}`}>
        {sessions.map((session) => {
          const isActive = session.id === activeSessionId
          const leadingVisual = getSessionLeadingVisual(session)
          return (
            <button
              key={session.id}
              onClick={() => onSelectSession(session.id)}
              className="relative mb-0.5 w-full px-2 py-1.5 text-left transition-colors"
              style={{
                color: 'var(--cp-text)',
              }}
              type="button"
            >
              <div className="flex items-center gap-1.5 pr-3">
                <span
                  className="flex-shrink-0"
                  style={{
                    color: leadingVisual.color,
                  }}
                >
                  {leadingVisual.icon}
                </span>

                <span
                  className="min-w-0 flex-1 truncate text-sm"
                  style={{
                    color: 'var(--cp-text)',
                    fontWeight: isActive ? 600 : 500,
                  }}
                >
                  {session.title}
                  {session.unreadCount > 0 ? (
                    <span
                      className="ml-1"
                      style={{
                        color: isActive
                          ? 'color-mix(in srgb, var(--cp-text) 78%, transparent)'
                          : 'var(--cp-muted)',
                        fontWeight: 500,
                      }}
                    >
                      ({session.unreadCount})
                    </span>
                  ) : null}
                </span>
              </div>

              {isActive ? (
                <span
                  className="pointer-events-none absolute bottom-1.5 right-0 top-1.5 rounded-full"
                  style={{
                    width: 3,
                    background: 'var(--cp-accent)',
                    boxShadow: '0 0 0 3px color-mix(in srgb, var(--cp-accent) 12%, transparent)',
                  }}
                />
              ) : null}
            </button>
          )
        })}
      </div>

      <button
        className="absolute bottom-5 right-5 flex h-10 w-10 items-center justify-center rounded-full"
        style={{
          background: 'transparent',
          color: 'var(--cp-accent)',
          border: '1px solid color-mix(in srgb, var(--cp-border) 88%, var(--cp-accent))',
          boxShadow: '0 12px 28px color-mix(in srgb, var(--cp-shadow) 12%, transparent)',
        }}
        title={t('messagehub.newSession', 'New Session')}
        type="button"
      >
        <SquarePen size={17} />
      </button>
    </div>
  )
}
