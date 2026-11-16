/* Q1 (a): MessageHub's data layer for the conversation component's bound parts — `SessionConversation`,
 * `agentSessions` and the embedded conversation page. Loaded on demand from `components/conversation/backend.ts`;
 * everything here works as the signed-in viewer looking at their own messages. */

import { ndn, type DID } from 'buckyos'
import { fetchCurrentAccount } from '../../api/account'
import { fetchAgentList } from '../../api/user_mgr'
import type { ConversationBackend, ConversationSessionInfo } from '../../components/conversation/backendTypes'
import type { MessageObject } from '../../components/conversation/model/msgobj'
import { agentDisplayName } from '../agent-setup/model'
import { SessionSource } from './conversationSessions'
import { findSession } from './conversationSource'
import { listUiSessionState, updateUiSessionState } from './datamodel/sessionApi'
import { getMessageHubStore, isMessageHubMock } from './store'
import type { Session } from './types'

const info = (session: Session): ConversationSessionInfo => ({
  id: session.id,
  entityId: session.entityId,
  title: getMessageHubStore().title(getMessageHubStore().defaultContext(), session),
  lastActiveAt: session.lastActiveAt,
  lifecycle: session.lifecycle,
  native: session.binding.kind === 'native',
})

export function createConversationBackend(): ConversationBackend {
  const store = getMessageHubStore()
  let ready: Promise<void> | null = null
  const context = () => store.defaultContext()
  const ensureReady = () => {
    ready ??= store.initialize().then(() => store.ensureOwner(context())).catch((error: unknown) => { ready = null; throw error })
    return ready
  }
  return {
    isMock: isMessageHubMock(),
    SessionSource,
    ensureReady,
    viewerDid: () => {
      try { return store.defaultContext().ownerDid } catch { return '' }
    },
    async defaultAgent() {
      await ensureReady()
      if (store.isMock) {
        const agent = store.entities(context()).find(entity => entity.type === 'agent' && store.isOwnAgent(context(), entity.id))
        return agent ? { did: agent.id, name: agent.name } : null
      }
      const [account, list] = await Promise.all([fetchCurrentAccount(), fetchAgentList()])
      if (list.error || !list.data) throw list.error ?? Error('Agent directory unavailable')
      const agent = list.data.agents.find(entry => entry.owner_user_id === account?.user_id && entry.install.state === 'ready')
      return agent ? { did: agent.agent_did, name: agentDisplayName(agent) } : null
    },
    sessions: (entityId) => store.sessions(context(), entityId, 'active').map(info).sort((a, b) => b.lastActiveAt - a.lastActiveAt),
    session: (sessionId) => {
      const session = findSession(context(), sessionId)
      return session ? info(session) : null
    },
    addAgent: (agentDid, name) => {
      if (!store.findEntity(context(), agentDid)) store.addPeer(context(), agentDid, { name, type: 'agent' })
    },
    async ensureDefaultSession(entityId) {
      await ensureReady()
      const session = await store.ensureDefaultSession(context(), entityId)
      return session ? info(session) : null
    },
    async createSession(entityId, title) {
      await ensureReady()
      return info(await store.create(context(), { entityId, title, connection: 'native' }))
    },
    deleteSession: (sessionId) => store.manage(context(), sessionId, 'delete'),
    async send(sessionId, { text, attachments = [] }) {
      await ensureReady()
      if (attachments.length === 0) return store.send(context(), sessionId, { content: text, attachments: [] }, undefined)
      // Zone objects travel as `data_obj` references, the way a share does (ShareIntake).
      const message: MessageObject = {
        from: context().viewerDid,
        to: [],
        kind: 'chat',
        created_at_ms: Date.now(),
        nonce: ndn.randomMsgNonce(),
        content: { format: 'text/plain', content: text, refs: attachments.map(item => ({ role: 'input' as const, target: { type: 'data_obj' as const, obj_id: item.objId, uri_hint: `cyfs://${item.objId}` }, ...(item.label ? { label: item.label } : {}) })) },
      }
      return store.forward(context(), sessionId, message, undefined)
    },
    async post(sessionId, payload) {
      await ensureReady()
      return store.send(context(), sessionId, payload, undefined)
    },
    async listUiState(sessionId) {
      return (await listUiSessionState(sessionId, context().ownerDid)).map(entry => ({ key: entry.key, value: entry.value }))
    },
    updateUiState: async (sessionId, key, value) => { await updateUiSessionState(sessionId, key, value, context().ownerDid as DID) },
  }
}
