import { useNavigate, useSearchParams } from 'react-router-dom'
import { MessageHubLink } from './MessageHubLink'
import { useI18n } from '../../i18n/provider'
import { useDocumentTitle } from '../../presentation/documentHead'
import { denialText } from '../../sso/denial'
import { getBootIdentity, isRestrictedIdentity } from '../../sso/identity'
import { useStandalonePage } from '../standalone/context'
import { StandalonePage } from '../standalone/StandalonePage'
import { useGuestText } from './guest/text'

// A Limited account or an External DID has no mailbox of its own: its Sessions open from their links (`/messagehub/guest`).
function RestrictedMessageHub() {
  const { t } = useI18n()
  const guestText = useGuestText()
  useDocumentTitle(t('apps.messagehub'))
  return <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center" data-testid="messagehub-restricted"><p role="alert">{denialText(t, { reason: 'session_kind_not_allowed', adjust: { party: 'product' } })}</p><p className="text-sm text-[color:var(--cp-muted)]">{guestText('restrictedHub')}</p><a className="min-h-11 rounded-lg border border-[color:var(--cp-border)] px-3 py-2 text-sm" href="/">{guestText('home')}</a></div>
}

function MessageHubFrame({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-0 flex-1 p-0 md:p-5">
      <div
        className="mx-auto flex h-full w-full flex-col overflow-hidden md:max-w-[1600px] md:rounded-xl md:border md:shadow-[var(--cp-window-shadow)]"
        style={{ borderColor: 'var(--cp-border)', background: 'var(--cp-surface-opaque)' }}
      >
        {children}
      </div>
    </div>
  )
}

function MessageHubPage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const navigate = useNavigate()
  // In the BuckyOS client the page fills a window or a tab, and the way back to the desktop is the client's.
  const inBrowser = useStandalonePage()?.container === 'browser'
  const exitObserver = () => setSearchParams(previous => {
    const next = new URLSearchParams(previous)
    next.delete('ownerDid'); next.delete('mode'); next.delete('entityId'); next.delete('sessionId'); next.delete('messageId')
    return next
  }, { replace: true })
  const hub = <MessageHubLink search={searchParams} onHome={inBrowser ? () => navigate('/') : undefined} onExitObserver={exitObserver} />
  return inBrowser
    ? <MessageHubFrame>{hub}</MessageHubFrame>
    : <div className="flex min-h-0 flex-1 flex-col" style={{ background: 'var(--cp-surface-opaque)' }}>{hub}</div>
}

export function MessageHubRoute() {
  if (isRestrictedIdentity(getBootIdentity())) {
    return <main className="flex h-dvh flex-col bg-[color:var(--cp-bg)]"><MessageHubFrame><RestrictedMessageHub /></MessageHubFrame></main>
  }
  return <StandalonePage appId="messagehub" ownHeader routed>{() => <MessageHubPage />}</StandalonePage>
}
