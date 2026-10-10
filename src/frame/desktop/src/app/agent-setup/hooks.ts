/* ── Agent setup – data hooks and draft storage ── */

import { useEffect, useRef, useState } from 'react'
import {
  checkAgentName,
  fetchAgentTemplates,
  fetchUserDetail,
  type AgentNameCheck,
  type AgentTemplate,
  type UserDetail,
} from '../../api/user_mgr'
import { onOwnProfileChanged } from './events'
import {
  agentSetupDraftKey,
  classifyAgentError,
  isValidAgentName,
  parseAgentSetupDraft,
  type AgentSetupDraft,
  type AgentSetupSource,
  type ClassifiedAgentError,
} from './model'

// ── Draft storage (per user and entry; never holds the bot token) ──

export function readStoredDraft(userId: string, source: AgentSetupSource): AgentSetupDraft | null {
  try {
    const raw = window.localStorage.getItem(agentSetupDraftKey(userId, source))
    return raw ? parseAgentSetupDraft(JSON.parse(raw), source) : null
  } catch {
    return null
  }
}

export function writeStoredDraft(userId: string, draft: AgentSetupDraft) {
  try {
    window.localStorage.setItem(agentSetupDraftKey(userId, draft.source), JSON.stringify(draft))
  } catch {
    // Storage full or blocked: the draft still lives in the wizard's state.
  }
}

export function removeStoredDraft(userId: string, source: AgentSetupSource) {
  try {
    window.localStorage.removeItem(agentSetupDraftKey(userId, source))
  } catch {
    // Nothing to clean up when storage is unavailable.
  }
}

// ── Name check ──

export type NameCheckState =
  | { status: 'empty' }
  | { status: 'invalid' }
  | { status: 'checking' }
  | { status: 'checked'; result: AgentNameCheck }
  | { status: 'error'; error: ClassifiedAgentError }

const NAME_CHECK_DELAY_MS = 400

/**
 * Debounced `agent.check_name`; a name is only usable once its own check came back available.
 * `onChecked` sees every completed check (e.g. to follow a suggestion).
 */
export function useAgentNameCheck(name: string, enabled: boolean, onChecked?: (result: AgentNameCheck) => void) {
  const [attempt, setAttempt] = useState(0)
  const [entry, setEntry] = useState<{ name: string; attempt: number; result?: AgentNameCheck; error?: ClassifiedAgentError } | null>(null)
  const valid = isValidAgentName(name)
  const onCheckedRef = useRef(onChecked)
  useEffect(() => { onCheckedRef.current = onChecked })

  useEffect(() => {
    if (!enabled || !valid) return
    let cancelled = false
    const timer = window.setTimeout(() => {
      void checkAgentName(name).then(({ data, error }) => {
        if (cancelled) return
        setEntry(data
          ? { name, attempt, result: data }
          : { name, attempt, error: classifyAgentError(error) })
        if (data) onCheckedRef.current?.(data)
      })
    }, NAME_CHECK_DELAY_MS)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [name, enabled, valid, attempt])

  let state: NameCheckState
  if (!name) state = { status: 'empty' }
  else if (!valid) state = { status: 'invalid' }
  else if (entry?.name !== name || entry.attempt !== attempt) state = { status: 'checking' }
  else if (entry.result) state = { status: 'checked', result: entry.result }
  else state = { status: 'error', error: entry.error ?? { kind: 'other', detail: '' } }

  return { state, retry: () => setAttempt((value) => value + 1) }
}

// ── Templates ──

export type TemplatesState =
  | { status: 'loading' }
  | { status: 'ready'; templates: AgentTemplate[] }
  | { status: 'error'; error: ClassifiedAgentError }

export function useAgentTemplates() {
  const [state, setState] = useState<TemplatesState>({ status: 'loading' })
  const [version, setVersion] = useState(0)

  useEffect(() => {
    let cancelled = false
    void fetchAgentTemplates().then(({ data, error }) => {
      if (cancelled) return
      setState(data
        ? { status: 'ready', templates: data.templates ?? [] }
        : { status: 'error', error: classifyAgentError(error) })
    })
    return () => { cancelled = true }
  }, [version])

  return {
    state,
    reload: () => {
      setState({ status: 'loading' })
      setVersion((value) => value + 1)
    },
  }
}

// ── The signed-in user's own profile (Owner identities) ──

export type OwnProfileState =
  | { status: 'loading' }
  | { status: 'ready'; detail: UserDetail }
  | { status: 'error'; error: ClassifiedAgentError }

export function useOwnProfile(userId: string) {
  const [state, setState] = useState<OwnProfileState>({ status: 'loading' })
  const [version, setVersion] = useState(0)

  useEffect(() => {
    let cancelled = false
    void fetchUserDetail({ userId }).then(({ data, error }) => {
      if (cancelled) return
      setState(data ? { status: 'ready', detail: data } : { status: 'error', error: classifyAgentError(error) })
    })
    return () => { cancelled = true }
  }, [userId, version])

  useEffect(() => onOwnProfileChanged(() => {
    setState({ status: 'loading' })
    setVersion((value) => value + 1)
  }), [])

  return {
    state,
    reload: () => {
      setState({ status: 'loading' })
      setVersion((value) => value + 1)
    },
  }
}
