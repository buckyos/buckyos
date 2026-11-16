/* The viewer's MessageHub session as `SessionConversation` binds to it (`ConversationBackend.SessionSource`). */

import { useEffect, type ReactNode } from 'react'
import type { BoundConversation } from '../../components/conversation/backendTypes'
import { findSession, useMessageHubConversationSource } from './conversationSource'
import { useMessageHubReady, useMessageHubRuntime, useMessageHubStore } from './store'
import type { MessageHubContext, Session } from './types'


function BoundSession({ context, session, children }: { context: MessageHubContext; session: Session; children: (state: BoundConversation) => ReactNode }) {
  const store = useMessageHubStore()
  useEffect(() => store.startSync(context, session.id), [store, context.ownerDid, context.viewerDid, context.mode, session.id]) // eslint-disable-line react-hooks/exhaustive-deps
  const source = useMessageHubConversationSource({ context, entityId: session.entityId, session })
  return children({ status: 'ready', source })
}

function OwnSession({ sessionId, children }: { sessionId: string; children: (state: BoundConversation) => ReactNode }) {
  const store = useMessageHubStore()
  useMessageHubRuntime()
  const context = store.defaultContext()
  useEffect(() => { void store.ensureOwner(context) }, [store, context.ownerDid, context.viewerDid, context.mode]) // eslint-disable-line react-hooks/exhaustive-deps
  const owner = store.ownerStatus(context)
  if (owner.phase === 'error' || owner.phase === 'denied') return children({ status: 'error', retry: () => { void store.ensureOwner(context, true) } })
  const session = owner.phase === 'ready' ? findSession(context, sessionId) : null
  if (!session) return children(owner.phase === 'ready' ? { status: 'missing' } : { status: 'loading' })
  return <BoundSession context={context} session={session}>{children}</BoundSession>
}

export function SessionSource({ sessionId, children }: { sessionId: string; children: (state: BoundConversation) => ReactNode }) {
  const { status, retry } = useMessageHubReady()
  if (status === 'loading') return children({ status: 'loading' })
  if (status === 'error') return children({ status: 'error', retry })
  return <OwnSession sessionId={sessionId}>{children}</OwnSession>
}
