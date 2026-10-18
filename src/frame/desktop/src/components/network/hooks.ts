/* ── React bindings of the network cache ── */

import { useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import { networkCache, type ResourceData, type ResourceKey, type ResourceState } from './cache'
import { messageRequests, pendingInvitations, requestItems } from './relation'
import type { LookupKind, LookupResult, RequestItem } from './types'

export function useNetworkResource<K extends ResourceKey>(key: K): ResourceState<ResourceData[K]> {
  const state = useSyncExternalStore(networkCache.subscribe, () => networkCache.getState(key))
  useEffect(() => { networkCache.ensure(key) }, [key])
  return state
}

export function useOwnerDid(): string | null {
  const [did, setDid] = useState<string | null>(null)
  useEffect(() => {
    let cancelled = false
    void networkCache.ownerDid().then(value => { if (!cancelled) setDid(value) }).catch(() => undefined)
    return () => { cancelled = true }
  }, [])
  return did
}

export interface LookupState {
  status: 'idle' | 'loading' | 'ready' | 'error'
  result?: LookupResult
  error?: unknown
  /** The input the shown result answers. */
  input: string
  retry: () => void
}

/** `contact.lookup` for the typed input, debounced; re-asks after any relationship change. */
export function useLookup(input: string, options: { kinds?: LookupKind[]; limit?: number; enabled?: boolean; debounceMs?: number } = {}): LookupState {
  const { enabled = true, debounceMs = 250, limit } = options
  const kindsKey = (options.kinds ?? []).join(',')
  const epoch = useSyncExternalStore(networkCache.subscribe, networkCache.getEpoch)
  const [attempt, setAttempt] = useState(0)
  const [state, setState] = useState<Omit<LookupState, 'retry'>>({ status: 'idle', input: '' })
  const trimmed = input.trim()

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    const kinds = kindsKey ? kindsKey.split(',') as LookupKind[] : undefined
    const timer = window.setTimeout(() => {
      setState(previous => previous.input === trimmed && previous.status === 'ready' ? previous : { ...previous, status: 'loading' })
      networkCache.lookup(trimmed, { kinds, limit })
        .then((result) => { if (!cancelled) setState({ status: 'ready', result, input: trimmed }) })
        .catch((error) => { if (!cancelled) setState({ status: 'error', error, input: trimmed }) })
    }, debounceMs)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [trimmed, kindsKey, limit, enabled, debounceMs, epoch, attempt])

  return useMemo(() => ({ ...state, status: enabled ? state.status : 'idle', retry: () => setAttempt(value => value + 1) }), [state, enabled])
}

export interface RequestsState {
  status: 'loading' | 'ready' | 'error' | 'partial'
  items: RequestItem[]
  errors: unknown[]
  retry: () => void
}

/** The four Requests item types (§3.6) from the cached lists; `partial` when some lists failed. */
export function useRequests(): RequestsState {
  const contacts = useNetworkResource('contacts')
  const friendRequests = useNetworkResource('friendRequests')
  const requestSessions = useNetworkResource('requestSessions')
  const groups = useNetworkResource('groups')
  const invitations = useNetworkResource('invitations')
  const joinRequests = useNetworkResource('joinRequests')
  return useMemo(() => {
    const all = [contacts, friendRequests, requestSessions, groups, invitations, joinRequests]
    const errors = all.filter(state => state.status === 'error').map(state => state.error)
    const loading = all.some(state => state.status === 'loading' && state.data === undefined)
    const items = requestItems({
      friendRequests: friendRequests.data ?? [],
      messageRequests: messageRequests(requestSessions.data ?? [], contacts.data ?? [], friendRequests.data ?? []),
      invitations: pendingInvitations(invitations.data ?? [], groups.data ?? []),
      joinRequests: joinRequests.data ?? [],
    })
    const retry = () => {
      (['contacts', 'friendRequests', 'requestSessions', 'groups', 'invitations', 'joinRequests'] as const).forEach(key => networkCache.refresh(key))
    }
    const status: RequestsState['status'] = loading ? 'loading' : errors.length === all.length ? 'error' : errors.length > 0 ? 'partial' : 'ready'
    return { status, items, errors, retry }
  }, [contacts, friendRequests, requestSessions, groups, invitations, joinRequests])
}
