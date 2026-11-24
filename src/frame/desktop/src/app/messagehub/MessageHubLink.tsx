/* ── MessageHub opened on an address (`/messagehub?…`): in a page of its own or in a window ── */

import { useEffect, useMemo, useState } from 'react'
import { messageHubLinkShare, messageHubRouteContext } from './launch'
import { MessageHubView } from './MessageHubView'
import { getNotifyWatch } from './notify'
import { getMessageHubStore } from './store'
import { useI18n } from '../../i18n/provider'

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

export function MessageHubLink({ search, windowId, onHome, onExitObserver }: { search: URLSearchParams; windowId?: string; onHome?: () => void; onExitObserver?: () => void }) {
  const { t } = useI18n()
  const notifyId = search.get('notify')
  const notifyLink = useNotifyLink(notifyId)
  const share = search.get('share')
  const initialShare = useMemo(() => messageHubLinkShare(new URLSearchParams(share ? { share } : {})), [share])
  const { target } = notifyLink
  return (
    <>
      {notifyLink.failed ? <p role="alert" className="shrink-0 border-b border-[color:var(--cp-border)] px-4 py-2 text-[13px] text-[color:var(--cp-danger)]" data-testid="notify-link-failed">{t('messagehub.notify.linkFailed')}</p> : null}
      <div className="min-h-0 flex-1">
        {notifyLink.loading
          ? <div className="flex h-full items-center justify-center"><p role="status">{t('messagehub.notify.opening')}</p></div>
          : <MessageHubView key={notifyId ?? ''} initialEntityId={target ? target.entityId : search.get('entityId')} initialSessionId={target?.sessionId ?? search.get('sessionId')} initialMessageId={target?.messageId ?? search.get('messageId')} linkedMessageDetails={!target} initialShare={initialShare} contextRequest={messageHubRouteContext(search)} windowId={windowId} onHome={onHome} onExitObserver={onExitObserver} />}
      </div>
    </>
  )
}
