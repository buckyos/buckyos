/* Q1 (a): MessageHub's data layer for the conversation component's bound parts — `SessionConversation`,
 * `agentSessions` and the embedded conversation page. Loaded on demand from `components/conversation/backend.ts`;
 * everything here works as the signed-in viewer looking at their own messages. */

import { ndn, NOTIFY_INTENT, type DID, type NotifyMessageData } from 'buckyos'
import { fetchCurrentAccount } from '../../api/account'
import { fetchAgentList } from '../../api/user_mgr'
import type { ConversationBackend, ConversationSessionInfo } from '../../components/conversation/backendTypes'
import type { MessageObject } from '../../components/conversation/model/msgobj'
import { agentDisplayName } from '../agent-setup/model'
import { SessionSource } from './conversationSessions'
import { findSession } from './conversationSource'
import { listUiSessionState, updateUiSessionState } from './datamodel/sessionApi'
import { getNotifyWatch } from './notify'
import { getMessageHubStore, isMessageHubMock } from './store'
import type { Session } from './types'

const GOAL_NOTICES = 50

const info = (session: Session): ConversationSessionInfo => ({
  id: session.id,
  entityId: session.entityId,
  title: getMessageHubStore().title(getMessageHubStore().defaultContext(), session),
  lastActiveAt: session.lastActiveAt,
  lifecycle: session.lifecycle,
  native: session.binding.kind === 'native',
  memoryMode: session.memoryMode ?? 'normal',
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
    addPerson: (did, name) => {
      if (!store.findEntity(context(), did)) store.addPeer(context(), did, { name, type: 'person' })
    },
    async ensureDefaultSession(entityId) {
      await ensureReady()
      const session = await store.ensureDefaultSession(context(), entityId, { writable: true })
      return session ? info(session) : null
    },
    async createSession(entityId, title, options) {
      await ensureReady()
      return info(await store.create(context(), { entityId, title, connection: 'native', ...options }))
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
    async goalNotices(agentDid, goalId) {
      await ensureReady()
      const views = await getNotifyWatch().list({ agent_did: agentDid, goal_id: goalId, limit: GOAL_NOTICES })
      return views.map((view): MessageObject => {
        const data: NotifyMessageData = { version: 1, notify_id: view.notify_id, kind: view.kind, category: view.category, level: view.level, home: false, source: view.source, prompt: view.prompt, url: view.url, ...(view.task_id ? { task_id: view.task_id } : {}), ...(view.expires_at_ms ? { expires_at_ms: view.expires_at_ms } : {}) }
        return { from: agentDid, to: [context().ownerDid], kind: 'notify', created_at_ms: view.created_at_ms, content: { format: 'text/plain', content: [view.prompt.title, view.prompt.message].filter(Boolean).join('\n'), machine: { intent: NOTIFY_INTENT, data: data as unknown as Record<string, ndn.CanonValue> } }, ui_message_id: `notify:${view.notify_id}` }
      }).sort((a, b) => a.created_at_ms - b.created_at_ms)
    },
  }
}
