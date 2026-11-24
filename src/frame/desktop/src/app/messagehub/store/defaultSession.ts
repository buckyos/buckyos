import { creationReason, viewerSessionKey } from '../sessionModel'
import type { MessageHubContext, Session } from '../types'
import type { DefaultSessionOptions, MessageHubStore } from './types'

type DefaultSessionStore = Pick<MessageHubStore, 'ensureOwner' | 'canView' | 'ownerStatus' | 'agentDefaultSession' | 'defaultSession' | 'hasMoreEntities' | 'loadMoreEntities' | 'findEntity' | 'connections' | 'policy' | 'create' | 'manage' | 'sessions'>
const pending = new WeakMap<DefaultSessionStore, Map<string, Promise<Session | null>>>()

export function ensureDefaultSession(store: DefaultSessionStore, context: MessageHubContext, entityId: string, options: DefaultSessionOptions = {}): Promise<Session | null> {
  let requests = pending.get(store)
  if (!requests) { requests = new Map(); pending.set(store, requests) }
  const key = `${viewerSessionKey(context, entityId)}${options.writable ? ' writable' : ''}`
  const existing = requests.get(key)
  if (existing) return existing
  const request = resolveDefaultSession(store, context, entityId, options).finally(() => { requests.delete(key) })
  requests.set(key, request)
  return request
}

async function resolveDefaultSession(store: DefaultSessionStore, context: MessageHubContext, entityId: string, options: DefaultSessionOptions): Promise<Session | null> {
  await store.ensureOwner(context)
  if (!store.canView(context) || store.ownerStatus(context).phase !== 'ready') throw new Error('permission_denied')
  const stored = await store.agentDefaultSession(context, entityId, options)
  if (stored) {
    // Entering an archived default brings it back.
    if (stored.lifecycle !== 'archived') return stored
    await store.manage(context, stored.id, 'restore')
    return store.sessions(context, entityId).find(session => session.id === stored.id) ?? stored
  }
  let session = store.defaultSession(context, entityId)
  while (!session && store.hasMoreEntities(context)) {
    await store.loadMoreEntities(context)
    if (!store.canView(context) || store.ownerStatus(context).phase !== 'ready') throw new Error('permission_denied')
    session = store.defaultSession(context, entityId)
  }
  if (session) return session
  const entity = store.findEntity(context, entityId)
  if (!entity || entity.type === 'group') return null
  const choices = store.connections(context, entityId)
    .filter(choice => entity.type !== 'agent' || choice.binding.kind === 'native')
    .filter(choice => !creationReason(context, entity, store.policy(context, entityId), choice.binding))
  const connection = choices.find(choice => choice.binding.kind === 'native') ?? choices[0]
  return connection ? store.create(context, { entityId, title: '', connection: connection.id }) : null
}
