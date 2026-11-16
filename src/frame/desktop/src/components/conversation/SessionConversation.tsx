import { useMediaQuery } from '@mui/material'
import type { ReactNode } from 'react'
import { LoadingOrb } from '../LoadingOrb'
import { useOptionalWindowDialog, WindowDialogProvider } from '../../desktop/windows/dialogs'
import { useI18n } from '../../i18n/provider'
import { useConversationBackend } from './backend'
import { ConversationSurface, type ConversationSurfaceProps } from './ConversationSurface'
import { useMessageMediaHost } from './media/context'
import { MessageMediaHost } from './media/MessageMediaHost'
import './conversation.css'

export interface SessionConversationProps extends Omit<ConversationSurfaceProps, 'source' | 'historyReader' | 'composer' | 'banner' | 'leadingPane' | 'panelDetails'> {
  /** A session of the signed-in viewer's own messages. */
  sessionId: string
  testId?: string
}

function Status({ testId, text, loading, action }: { testId: string; text: string; loading?: boolean; action?: { label: string; onClick: () => void } }) {
  return (
    <div className="mh-session-conversation-status" role={loading ? 'status' : 'alert'} data-testid={testId}>
      {loading ? <LoadingOrb size={32} /> : null}
      <p>{text}</p>
      {action ? <button type="button" className="mh-turn-action" onClick={action.onClick} data-testid={`${testId}-retry`}>{action.label}</button> : null}
    </div>
  )
}

function Bound({ sessionId, testId = 'session-conversation', ...surface }: SessionConversationProps) {
  const { t } = useI18n()
  const backend = useConversationBackend()
  if (!backend) return <Status testId="session-conversation-loading" loading text={t('messagehub.loadingHistory')} />
  return (
    <backend.SessionSource sessionId={sessionId}>
      {state => state.status === 'ready' ? (
        <div className="mh-session-conversation" data-testid={testId} data-session={sessionId}>
          <ConversationSurface {...surface} source={state.source} />
        </div>
      ) : state.status === 'loading' ? <Status testId="session-conversation-loading" loading text={t('messagehub.loadingHistory')} />
        : state.status === 'missing' ? <Status testId="session-conversation-missing" text={t('messagehub.noSessions')} />
        : <Status testId="session-conversation-error" text={t('messagehub.historyFailed')} action={{ label: t('messagehub.retry'), onClick: state.retry }} />}
    </backend.SessionSource>
  )
}

/**
 * A conversation of the signed-in viewer, bound to MessageHub's data: give it a session id and it shows that
 * conversation (§2.2). It brings its dialog and media hosts along unless the page already has them.
 */
export function SessionConversation(props: SessionConversationProps) {
  const dialogs = useOptionalWindowDialog()
  const media = useMessageMediaHost()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  let content: ReactNode = <Bound {...props} />
  if (!media) content = <MessageMediaHost>{content}</MessageMediaHost>
  if (!dialogs) content = <WindowDialogProvider surface={isDesktop ? 'desktop' : 'mobile'} permissions={{ fullscreen: false }}>{content}</WindowDialogProvider>
  return content
}
