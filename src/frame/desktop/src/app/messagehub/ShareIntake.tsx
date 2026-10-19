/**
 * A share handed over from the system share dialog (`sysdlg/share`): pick who
 * gets it, send it as a data_obj attachment (or the link text, for folders
 * and large files), and report the outcome to nfs-server.
 */

import { useEffect, useRef } from 'react'
import { useWindowDialog } from '../../desktop/windows/dialogs'
import { useI18n } from '../../i18n/provider'
import { reportShareDelivery } from '../../api/shares'
import { ForwardMessageForm } from './ForwardDialog'
import type { SharedLaunch } from './launch'
import { randomMsgNonce, type MessageObject } from './protocol/msgobj'
import type { MessageHubContext } from './types'

function shareMessage(context: MessageHubContext, share: SharedLaunch): MessageObject {
  const refs = share.objId
    ? [{ role: 'input' as const, target: { type: 'data_obj' as const, obj_id: share.objId, uri_hint: `cyfs://${share.objId}` }, label: share.name }]
    : []
  const text = share.objId ? '' : [share.title ?? share.name, share.url].filter(Boolean).join('\n')
  return {
    from: context.viewerDid,
    to: [],
    kind: 'chat',
    created_at_ms: Date.now(),
    nonce: randomMsgNonce(),
    content: { format: 'text/plain', content: text, ...(refs.length ? { refs } : {}) },
  }
}

export function ShareIntake({ context, share, confirmationFor }: { context: MessageHubContext; share: SharedLaunch; confirmationFor: (sessionId: string) => string | undefined }) {
  const { t } = useI18n()
  const dialog = useWindowDialog()
  const opened = useRef(false)
  useEffect(() => {
    if (opened.current) return
    opened.current = true
    const message = shareMessage(context, share)
    let settled = false
    void dialog
      .open({
        title: t('share.handoff.messagehub.banner', 'Choose who to send {{name}} to', { name: share.name }),
        size: 'sm',
        dismissible: false,
        renderBody: (controls) => (
          <ForwardMessageForm
            context={context}
            message={message}
            confirmationFor={confirmationFor}
            onCancel={() => {
              settled = true
              reportShareDelivery(share.shareId, 'canceled')
              controls.close()
            }}
            onDone={(entity, sessionId) => {
              settled = true
              reportShareDelivery(share.shareId, 'confirmed', { businessRef: { entity_id: entity.id, session_id: sessionId } })
              controls.close()
            }}
          />
        ),
      })
      .then(() => {
        if (!settled) reportShareDelivery(share.shareId, 'unknown')
      })
  }, [context, share, confirmationFor, dialog, t])
  return null
}
