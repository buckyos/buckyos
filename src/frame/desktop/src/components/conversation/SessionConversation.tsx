import { useMediaQuery } from '@mui/material'
import type { ReactNode } from 'react'
import { LoadingOrb } from '../LoadingOrb'
import { useOptionalWindowDialog, WindowDialogProvider } from '../../desktop/windows/dialogs'
import { useI18n } from '../../i18n/provider'
import { useConversationBackend } from './backend'
import { ConversationSurface, type ConversationSurfaceProps } from './ConversationSurface'
import { useMessageMediaHost } from './media/context'
import { MessageMediaHost } from './media/MessageMediaHost'
import type { OutgoingPayload } from './model/outgoing'
import type { ConversationSource } from './source'
import './conversation.css'

export interface SessionConversationProps extends Omit<ConversationSurfaceProps, 'source' | 'historyReader' | 'composer' | 'banner' | 'leadingPane' | 'panelIcon' | 'panelDetails'> {
  /** A session of the signed-in viewer's own messages. */
  sessionId: string
  testId?: string
  /** What a message the composer sends (and its pending bubble) carries in the end: the host adds the place the
   * conversation was opened from. Called once per payload with a `localId` for the bubble and again for the send. */
  outgoing?: (payload: OutgoingPayload) => OutgoingPayload
}

const wrapped = new WeakMap<ConversationSource, { outgoing: (payload: OutgoingPayload) => OutgoingPayload; source: ConversationSource }>()

/** The source with the host's `outgoing`; the same object while the source and `outgoing` stay the same. */
function withOutgoing(source: ConversationSource, outgoing: (payload: OutgoingPayload) => OutgoingPayload): ConversationSource {
  const known = wrapped.get(source)
  if (known?.outgoing === outgoing) return known.source
  const { stageOutgoing } = source
  const next = { ...source, send: (payload: OutgoingPayload) => source.send(outgoing(payload)), stageOutgoing: stageOutgoing ? (queued: OutgoingPayload[]) => stageOutgoing(queued.map(outgoing)) : undefined }
  wrapped.set(source, { outgoing, source: next })
  return next
}

/** What stands in the place of a conversation that is not there. */
export function ConversationStatus({ testId, text, loading, action }: { testId: string; text: string; loading?: boolean; action?: { label: string; onClick: () => void } }) {
  return (
    <div className="mh-session-conversation-status" role={loading ? 'status' : 'alert'} data-testid={testId}>
      {loading ? <LoadingOrb size={32} /> : null}
      <p>{text}</p>
      {action ? <button type="button" className="mh-turn-action" onClick={action.onClick} data-testid={`${testId}-retry`}>{action.label}</button> : null}
    </div>
  )
}

function Bound({ sessionId, testId = 'session-conversation', outgoing, ...surface }: SessionConversationProps) {
  const { t } = useI18n()
  const backend = useConversationBackend()
  if (!backend) return <ConversationStatus testId="session-conversation-loading" loading text={t('messagehub.loadingHistory')} />
  return (
    <backend.SessionSource sessionId={sessionId}>
      {state => state.status === 'ready' ? (
        <div className="mh-session-conversation" data-testid={testId} data-session={sessionId}>
          <ConversationSurface {...surface} source={outgoing ? withOutgoing(state.source, outgoing) : state.source} />
        </div>
      ) : state.status === 'loading' ? <ConversationStatus testId="session-conversation-loading" loading text={t('messagehub.loadingHistory')} />
        : state.status === 'missing' ? <ConversationStatus testId="session-conversation-missing" text={t('messagehub.noSessions')} />
        : <ConversationStatus testId="session-conversation-error" text={t('messagehub.historyFailed')} action={{ label: t('messagehub.retry'), onClick: state.retry }} />}
    </backend.SessionSource>
  )
}

/** The dialog and media hosts a conversation works with, unless the page already has them. */
export function ConversationHosts({ children }: { children: ReactNode }) {
  const dialogs = useOptionalWindowDialog()
  const media = useMessageMediaHost()
  const isDesktop = useMediaQuery('(min-width: 769px)')
  let content: ReactNode = children
  if (!media) content = <MessageMediaHost>{content}</MessageMediaHost>
  if (!dialogs) content = <WindowDialogProvider surface={isDesktop ? 'desktop' : 'mobile'} permissions={{ fullscreen: false }}>{content}</WindowDialogProvider>
  return content
}

/**
 * A conversation of the signed-in viewer, bound to MessageHub's data: give it a session id and it shows that
 * conversation (§2.2). It brings its dialog and media hosts along unless the page already has them.
 */
export function SessionConversation(props: SessionConversationProps) {
  return <ConversationHosts><Bound {...props} /></ConversationHosts>
}
