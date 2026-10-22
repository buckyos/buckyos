import { useEffect, useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { messageHubRouteContext } from './launch'
import { MessageHubView } from './MessageHubView'
import { getNotifyWatch } from './notify'
import { getMessageHubStore } from './store'
import { useI18n } from '../../i18n/provider'
import { useDocumentTitle } from '../../presentation/documentHead'
import { denialText } from '../../sso/denial'
import { getBootIdentity, isRestrictedIdentity } from '../../sso/identity'
import { useGuestText } from './guest/text'

interface NotifyLink {
  notifyId: string
  target?: { entityId: string | null; sessionId: string; messageId: string }
}

/**
 * `?notify=<id>`: the card of a notification. `notify.get` names its home
 * record (session and message), which then opens like an
 * `entityId / sessionId / messageId` link.
 */
function useNotifyLink(notifyId: string | null): { loading: boolean; target?: NotifyLink['target']; failed: boolean } {
  const [link, setLink] = useState<NotifyLink | null>(null)
  useEffect(() => {
    if (!notifyId) return
    let cancelled = false
    void (async () => {
      try {
        const store = getMessageHubStore()
        await store.initialize()
        const home = (await getNotifyWatch().load(notifyId)).view?.home
        const context = store.defaultContext()
        if (!cancelled) setLink(home ? { notifyId, target: { entityId: store.findEntity(context, home.from) ? home.from : null, sessionId: home.session_id, messageId: home.msg_id } } : { notifyId })
      } catch {
        if (!cancelled) setLink({ notifyId })
      }
    })()
    return () => { cancelled = true }
  }, [notifyId])
  if (!notifyId) return { loading: false, failed: false }
  if (link?.notifyId !== notifyId) return { loading: true, failed: false }
  return { loading: false, target: link.target, failed: !link.target }
}

export function MessageHubRoute() {
  const [searchParams, setSearchParams] = useSearchParams()
  const navigate = useNavigate()
  const { t } = useI18n()
  const guestText = useGuestText()
  useDocumentTitle(t('apps.messagehub'))
  // A Limited account or an External DID has no mailbox of its own: its Sessions open from their links (`/messagehub/guest`).
  const restricted = isRestrictedIdentity(getBootIdentity())
  const notifyId = restricted ? null : searchParams.get('notify')
  const notifyLink = useNotifyLink(notifyId)
  const entityId = notifyLink.target ? notifyLink.target.entityId : searchParams.get('entityId')
  const sessionId = notifyLink.target?.sessionId ?? searchParams.get('sessionId')
  const messageId = notifyLink.target?.messageId ?? searchParams.get('messageId')
  const exitObserver = () => setSearchParams(previous => {
    const next = new URLSearchParams(previous)
    next.delete('ownerDid'); next.delete('mode'); next.delete('entityId'); next.delete('sessionId'); next.delete('messageId')
    return next
  }, { replace: true })

  return (
    <main className="min-h-dvh bg-[color:var(--cp-bg)] p-0 md:p-5">
      <div
        className="mx-auto flex h-dvh w-full flex-col overflow-hidden md:h-[calc(100dvh-2.5rem)] md:max-w-[1600px] md:rounded-[28px] md:border md:shadow-[var(--cp-window-shadow)]"
        style={{
          borderColor: 'var(--cp-border)',
          background:
            'linear-gradient(180deg, color-mix(in srgb, var(--cp-surface) 96%, transparent), color-mix(in srgb, var(--cp-surface-2) 94%, transparent))',
          backdropFilter: 'blur(20px)',
        }}
      >
        {notifyLink.failed ? <p role="alert" className="shrink-0 border-b border-[color:var(--cp-border)] px-4 py-2 text-[13px] text-[color:var(--cp-danger)]" data-testid="notify-link-failed">{t('messagehub.notify.linkFailed')}</p> : null}
        <div className="min-h-0 flex-1">
          {restricted
            ? <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center" data-testid="messagehub-restricted"><p role="alert">{denialText(t, { reason: 'session_kind_not_allowed', adjust: { party: 'product' } })}</p><p className="text-sm text-[color:var(--cp-muted)]">{guestText('restrictedHub')}</p><a className="min-h-11 rounded-lg border border-[color:var(--cp-border)] px-3 py-2 text-sm" href="/">{guestText('home')}</a></div>
            : notifyLink.loading
            ? <div className="flex h-full items-center justify-center"><p role="status">{t('messagehub.notify.opening')}</p></div>
            : <MessageHubView key={notifyId ?? ''} initialEntityId={entityId} initialSessionId={sessionId} initialMessageId={messageId} linkedMessageDetails={!notifyLink.target} contextRequest={messageHubRouteContext(searchParams)} onHome={() => navigate('/')} onExitObserver={exitObserver} />}
        </div>
      </div>
    </main>
  )
}
