/* The one place the conversation component reaches into the apps (C8): the Preview App's windows, and the
 * MessageHub data layer the bound components (`SessionConversation`, `agentSessions`) work on. The data layer is
 * loaded on first use, the way `components/network/backend.ts` loads its backends. */

import { useEffect, useSyncExternalStore } from 'react'
import type { ConversationBackend } from './backendTypes'

export { openPreview } from '../../app/preview/launch'

let backend: ConversationBackend | null = null
let loading: Promise<ConversationBackend> | null = null
const listeners = new Set<() => void>()

export function loadConversationBackend(): Promise<ConversationBackend> {
  if (backend) return Promise.resolve(backend)
  loading ??= import('../../app/messagehub/conversationBackend').then((module) => {
    backend ??= module.createConversationBackend()
    loading = null
    listeners.forEach((listener) => listener())
    return backend
  })
  return loading
}

/** The data layer once it is loaded; null before (`loadConversationBackend` starts it). */
export function conversationBackend(): ConversationBackend | null {
  return backend
}

const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } }

/** The data layer, loading it on first use; null until it is there. */
export function useConversationBackend(): ConversationBackend | null {
  const value = useSyncExternalStore(subscribe, conversationBackend)
  useEffect(() => { if (!backend) void loadConversationBackend() }, [])
  return value
}
