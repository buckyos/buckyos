import { useCallback, useEffect, useMemo, useState } from 'react'

import {
  fetchChatBootstrap,
  fetchChatContacts,
  fetchChatMessages,
  sendChatMessage,
  startChatStream,
} from '@/api'

import Icon from '../../icons'

const formatTimestamp = (value: number) => {
  if (!value) return 'Unknown time'
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(value))
}

const formatRelativeFreshness = (value: number) => {
  if (!value) return 'No recent activity'
  const diffMs = Date.now() - value
  const diffMinutes = Math.max(1, Math.round(diffMs / 60000))
  if (diffMinutes < 60) return `${diffMinutes}m ago`
  const diffHours = Math.round(diffMinutes / 60)
  if (diffHours < 24) return `${diffHours}h ago`
  const diffDays = Math.round(diffHours / 24)
  return `${diffDays}d ago`
}

const readErrorMessage = (error: unknown, fallback: string) => {
  if (error instanceof Error && error.message.trim()) return error.message
  if (typeof error === 'string' && error.trim()) return error
  return fallback
}

const bindingsSummary = (contact: ChatContact) => {
  if (!contact.bindings.length) return 'No tunnel bindings'
  return contact.bindings
    .slice(0, 2)
    .map((binding) => `${binding.platform}:${binding.display_id || binding.account_id}`)
    .join(' · ')
}

const streamLabels: Record<'idle' | 'connecting' | 'reconnecting' | 'live' | 'offline', string> = {
  idle: 'Idle',
  connecting: 'Connecting',
  reconnecting: 'Reconnecting',
  live: 'Live stream',
  offline: 'Stream offline',
}

const streamPillStyles: Record<'idle' | 'connecting' | 'reconnecting' | 'live' | 'offline', string> = {
  idle: 'border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]',
  connecting: 'border-sky-200 bg-sky-50 text-sky-700',
  reconnecting: 'border-amber-200 bg-amber-50 text-amber-700',
  live: 'border-emerald-200 bg-emerald-50 text-emerald-700',
  offline: 'border-rose-200 bg-rose-50 text-rose-700',
}

const upsertMessage = (items: ChatMessage[], next: ChatMessage) => {
  const index = items.findIndex((item) => item.record_id === next.record_id)
  if (index === -1) {
    return [...items, next]
  }

  const updated = [...items]
  updated[index] = next
  return updated
}

const ChatWindow = () => {
  const [bootstrap, setBootstrap] = useState<ChatBootstrapResponse | null>(null)
  const [contacts, setContacts] = useState<ChatContact[]>([])
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [contactsLoading, setContactsLoading] = useState(true)
  const [messagesLoading, setMessagesLoading] = useState(false)
  const [sending, setSending] = useState(false)
  const [pageError, setPageError] = useState<string | null>(null)
  const [messageError, setMessageError] = useState<string | null>(null)
  const [streamError, setStreamError] = useState<string | null>(null)
  const [streamStatus, setStreamStatus] = useState<
    'idle' | 'connecting' | 'reconnecting' | 'live' | 'offline'
  >('idle')
  const [streamActivityAt, setStreamActivityAt] = useState<number | null>(null)
  const [selectedPeerDid, setSelectedPeerDid] = useState('')
  const [targetInput, setTargetInput] = useState('')
  const [contactSearch, setContactSearch] = useState('')
  const [draft, setDraft] = useState('')
  const [threadId, setThreadId] = useState('')
  const [reloadKey, setReloadKey] = useState(0)

  const loadShell = useCallback(async () => {
    setContactsLoading(true)
    const [bootstrapResult, contactsResult] = await Promise.all([
      fetchChatBootstrap(),
      fetchChatContacts({ limit: 100 }),
    ])

    setBootstrap(bootstrapResult.data)
    setContacts(contactsResult.data?.items ?? [])

    const bootstrapError = bootstrapResult.error
    const contactsError = contactsResult.error
    if (bootstrapError || contactsError) {
      setPageError(
        readErrorMessage(
          bootstrapError ?? contactsError,
          'Unable to load chat through control panel wrapper.',
        ),
      )
    } else {
      setPageError(null)
    }

    setContactsLoading(false)
  }, [])

  const loadMessages = useCallback(async (peerDid: string, options: { quiet?: boolean } = {}) => {
    const normalized = peerDid.trim()
    if (!normalized) {
      setMessages([])
      setMessageError(null)
      return
    }

    if (!options.quiet) {
      setMessagesLoading(true)
    }

    const { data, error } = await fetchChatMessages(normalized, 60)
    setMessages(data?.items ?? [])
    setMessageError(error ? readErrorMessage(error, 'Unable to load messages.') : null)

    if (!options.quiet) {
      setMessagesLoading(false)
    }
  }, [])

  useEffect(() => {
    void loadShell()
  }, [loadShell, reloadKey])

  useEffect(() => {
    if (selectedPeerDid || !contacts.length) {
      return
    }

    setSelectedPeerDid(contacts[0].did)
    setTargetInput(contacts[0].did)
  }, [contacts, selectedPeerDid])

  useEffect(() => {
    void loadMessages(selectedPeerDid)
  }, [loadMessages, selectedPeerDid])

  useEffect(() => {
    const peerDid = selectedPeerDid.trim()
    if (!peerDid) {
      setStreamStatus('idle')
      setStreamError(null)
      setStreamActivityAt(null)
      return
    }

    let cancelled = false
    let reconnectTimer: number | null = null
    let stopStream: (() => void) | null = null
    let attempt = 0

    const clearReconnect = () => {
      if (reconnectTimer !== null) {
        window.clearTimeout(reconnectTimer)
        reconnectTimer = null
      }
    }

    const scheduleReconnect = () => {
      clearReconnect()
      reconnectTimer = window.setTimeout(() => {
        connect()
      }, 1800)
    }

    const connect = () => {
      if (cancelled) return

      setStreamStatus(attempt === 0 ? 'connecting' : 'reconnecting')
      setStreamError(null)

      stopStream = startChatStream({
        peerDid,
        onEvent: (event) => {
          if (cancelled) return

          if (event.type === 'ack') {
            setStreamStatus('live')
            setStreamActivityAt(event.at_ms)
            void loadMessages(peerDid, { quiet: true })
            return
          }

          if (event.type === 'keepalive') {
            setStreamStatus('live')
            setStreamActivityAt(event.at_ms)
            return
          }

          if (event.type === 'message') {
            setStreamStatus('live')
            setStreamActivityAt(event.at_ms)
            setMessages((current) => upsertMessage(current, event.message))
            return
          }

          if (event.type === 'resync') {
            setStreamActivityAt(event.at_ms)
            void loadMessages(peerDid, { quiet: true })
            return
          }

          if (event.type === 'error') {
            setStreamStatus('offline')
            setStreamActivityAt(event.at_ms)
            setStreamError(event.message)
          }
        },
        onError: (error) => {
          if (cancelled) return
          setStreamStatus('offline')
          setStreamError(readErrorMessage(error, 'Live chat stream disconnected.'))
        },
        onDone: (reason) => {
          if (cancelled || reason === 'aborted' || reason === 'fatal') {
            return
          }

          setStreamStatus('offline')
          scheduleReconnect()
        },
      })

      attempt += 1
    }

    connect()

    return () => {
      cancelled = true
      clearReconnect()
      stopStream?.()
    }
  }, [loadMessages, selectedPeerDid])

  const visibleContacts = useMemo(() => {
    const keyword = contactSearch.trim().toLowerCase()
    if (!keyword) return contacts

    return contacts.filter((contact) => {
      const haystack = [contact.name, contact.did, contact.note ?? '', bindingsSummary(contact)]
        .join(' ')
        .toLowerCase()
      return haystack.includes(keyword)
    })
  }, [contactSearch, contacts])

  const activeContact = useMemo(
    () => contacts.find((contact) => contact.did === selectedPeerDid) ?? null,
    [contacts, selectedPeerDid],
  )

  const orderedMessages = useMemo(
    () => [...messages].sort((left, right) => left.created_at_ms - right.created_at_ms),
    [messages],
  )

  const canSend = bootstrap?.capabilities.message_send ?? false

  const handleRefresh = async () => {
    setStreamError(null)
    setReloadKey((value) => value + 1)
    if (selectedPeerDid.trim()) {
      await loadMessages(selectedPeerDid)
    }
  }

  const handleOpenPeer = (peerDid: string) => {
    const normalized = peerDid.trim()
    if (!normalized) return
    setSelectedPeerDid(normalized)
    setTargetInput(normalized)
  }

  const handleSend = async () => {
    const targetDid = selectedPeerDid.trim()
    const content = draft.trim()
    const normalizedThreadId = threadId.trim()
    if (!targetDid || !content) return

    setSending(true)
    setStreamError(null)
    const { error } = await sendChatMessage(
      targetDid,
      content,
      normalizedThreadId || undefined,
    )
    if (error) {
      setMessageError(readErrorMessage(error, 'Unable to send message.'))
      setSending(false)
      return
    }

    setDraft('')
    await loadMessages(targetDid, { quiet: true })
    setSending(false)
  }

  return (
    <div className="flex h-full min-h-0 flex-col gap-4 p-4">
      <section className="cp-panel px-6 py-5">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <div className="flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
              <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
                <Icon name="message" className="size-4" />
              </span>
              <div>
                <h2 className="text-xl font-semibold text-[var(--cp-ink)]">Bucky Chat</h2>
              </div>
            </div>
          </div>

          <div className="flex flex-wrap gap-2">
            <span className={`cp-pill border ${streamPillStyles[streamStatus]}`}>
              {streamLabels[streamStatus]}
            </span>
            <button
              type="button"
              onClick={handleRefresh}
              className="inline-flex min-h-11 items-center gap-2 rounded-full bg-[var(--cp-primary)] px-5 py-2 text-sm font-semibold text-white shadow transition hover:bg-[var(--cp-primary-strong)]"
            >
              Refresh
            </button>
          </div>
        </div>

        <div className="mt-5 grid gap-3 lg:grid-cols-3">
          <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
            <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Owner Scope</p>
            <p className="mt-2 truncate text-sm font-semibold text-[var(--cp-ink)]">
              {bootstrap?.scope.owner_did ?? 'Loading...'}
            </p>
          </div>
          <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
            <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Contacts</p>
            <p className="mt-2 text-sm font-semibold text-[var(--cp-ink)]">{contacts.length}</p>
          </div>
          <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
            <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Live Stream</p>
            <p className="mt-2 text-sm font-semibold text-[var(--cp-ink)]">
              {streamLabels[streamStatus]}
            </p>
            <p className="mt-1 text-xs text-[var(--cp-muted)]">
              {streamActivityAt
                ? `Last event ${formatRelativeFreshness(streamActivityAt)}`
                : 'Waiting for first event'}
            </p>
          </div>
        </div>

        {pageError ? (
          <div className="mt-4 rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-700">
            {pageError}
          </div>
        ) : null}

        {streamError ? (
          <div className="mt-4 rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-700">
            {streamError}
          </div>
        ) : null}
      </section>

      <div className="grid min-h-0 flex-1 gap-4 xl:grid-cols-[300px_minmax(0,1fr)]">
        <aside className="cp-panel flex min-h-0 flex-col p-5">
          <div className="flex items-center justify-between gap-3">
            <div>
              <h3 className="text-lg font-semibold text-[var(--cp-ink)]">Targets</h3>
              <p className="mt-1 text-sm text-[var(--cp-muted)]">
                Open a contact thread or enter any DID manually.
              </p>
            </div>
            <span className="cp-pill border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">
              {visibleContacts.length}
            </span>
          </div>

          <div className="mt-4 space-y-3">
            <input
              value={contactSearch}
              onChange={(event) => setContactSearch(event.target.value)}
              placeholder="Search contacts"
              className="w-full rounded-full border border-[var(--cp-border)] bg-white px-4 py-2 text-sm text-[var(--cp-ink)] placeholder:text-[var(--cp-muted)] focus:outline-none focus:ring-2 focus:ring-[var(--cp-primary-soft)]"
            />

            <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] p-3">
              <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
                Direct DID
              </p>
              <div className="mt-2 flex gap-2">
                <input
                  value={targetInput}
                  onChange={(event) => setTargetInput(event.target.value)}
                  placeholder="did:bns:target"
                  className="min-w-0 flex-1 rounded-xl border border-[var(--cp-border)] bg-white px-3 py-2 text-sm text-[var(--cp-ink)] placeholder:text-[var(--cp-muted)] focus:outline-none focus:ring-2 focus:ring-[var(--cp-primary-soft)]"
                />
                <button
                  type="button"
                  onClick={() => handleOpenPeer(targetInput)}
                  className="inline-flex min-h-11 items-center rounded-xl bg-[var(--cp-primary)] px-4 text-sm font-semibold text-white transition hover:bg-[var(--cp-primary-strong)]"
                >
                  Open
                </button>
              </div>
            </div>
          </div>

          <div className="mt-4 min-h-0 flex-1 space-y-2 overflow-y-auto pr-1">
            {contactsLoading ? (
              Array.from({ length: 5 }).map((_, index) => (
                <div
                  key={`chat-contact-skeleton-${index}`}
                  className="animate-pulse rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-4"
                >
                  <div className="h-3 w-24 rounded-full bg-white" />
                  <div className="mt-3 h-3 w-full rounded-full bg-white" />
                </div>
              ))
            ) : visibleContacts.length ? (
              visibleContacts.map((contact) => {
                const active = contact.did === selectedPeerDid
                return (
                  <button
                    key={contact.did}
                    type="button"
                    onClick={() => handleOpenPeer(contact.did)}
                    className={`w-full rounded-2xl border px-4 py-3 text-left transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--cp-primary-soft)] ${
                      active
                        ? 'border-transparent bg-[var(--cp-primary)] text-white shadow'
                        : 'border-[var(--cp-border)] bg-[var(--cp-surface-muted)] text-[var(--cp-ink)] hover:border-[var(--cp-primary-soft)] hover:bg-white'
                    }`}
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <p className="truncate text-sm font-semibold">{contact.name || contact.did}</p>
                        <p className={`mt-1 truncate text-xs ${active ? 'text-white/75' : 'text-[var(--cp-muted)]'}`}>
                          {contact.did}
                        </p>
                      </div>
                      <span className={`cp-pill ${active ? 'bg-white/15 text-white' : 'bg-white text-[var(--cp-muted)]'}`}>
                        {contact.access_level}
                      </span>
                    </div>
                    <p className={`mt-2 text-xs ${active ? 'text-white/80' : 'text-[var(--cp-muted)]'}`}>
                      {bindingsSummary(contact)}
                    </p>
                    <p className={`mt-2 text-[11px] ${active ? 'text-white/70' : 'text-[var(--cp-muted)]'}`}>
                      Updated {formatRelativeFreshness(contact.updated_at)}
                    </p>
                  </button>
                )
              })
            ) : (
              <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-6 text-center text-sm text-[var(--cp-muted)]">
                No contacts in this scope yet. Enter a DID above to inspect or send a direct
                message.
              </div>
            )}
          </div>
        </aside>

        <section className="cp-panel flex min-h-0 flex-col overflow-hidden">
          <div className="border-b border-[var(--cp-border)] px-6 py-5">
            <div className="flex flex-wrap items-start justify-between gap-4">
              <div>
                <div className="flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
                  <span className="inline-flex size-10 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
                    <Icon name="message" className="size-4" />
                  </span>
                  <div>
                    <h3 className="text-xl font-semibold text-[var(--cp-ink)]">
                      {activeContact?.name ?? (selectedPeerDid || 'Select a target')}
                    </h3>
                    <p className="mt-1 text-sm text-[var(--cp-muted)]">
                      {selectedPeerDid || 'Choose a contact or open a DID thread from the sidebar.'}
                    </p>
                  </div>
                </div>
              </div>

              <div className="grid gap-2 sm:grid-cols-2">
                <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
                  <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Peer</p>
                  <p className="mt-2 text-sm font-semibold text-[var(--cp-ink)]">
                    {selectedPeerDid || '-'}
                  </p>
                </div>
                <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
                  <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Messages</p>
                  <p className="mt-2 text-sm font-semibold text-[var(--cp-ink)]">{messages.length}</p>
                </div>
              </div>
            </div>

            <div className="mt-4 grid gap-3 lg:grid-cols-[minmax(0,1fr)_220px]">
              <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] p-3">
                <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">
                  Thread ID (optional)
                </p>
                <input
                  value={threadId}
                  onChange={(event) => setThreadId(event.target.value)}
                  placeholder="Attach this chat to a future agent/control thread"
                  className="mt-2 w-full rounded-xl border border-[var(--cp-border)] bg-white px-3 py-2 text-sm text-[var(--cp-ink)] placeholder:text-[var(--cp-muted)] focus:outline-none focus:ring-2 focus:ring-[var(--cp-primary-soft)]"
                />
              </div>
              <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3 text-sm text-[var(--cp-muted)]">
                <p className="font-semibold text-[var(--cp-ink)]">Wrapper mode</p>
                <p className="mt-2">
                  Uses `chat.*` plus streamed NDJSON from control panel, not direct browser
                  calls to `msg-center` and not WebSocket.
                </p>
              </div>
            </div>

            {messageError ? (
              <div className="mt-4 rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-700">
                {messageError}
              </div>
            ) : null}
          </div>

          <div className="flex min-h-0 flex-1 flex-col bg-[var(--cp-surface-muted)]/60">
            <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
              {!selectedPeerDid ? (
                <div className="flex h-full items-center justify-center rounded-3xl border border-dashed border-[var(--cp-border)] bg-white/60 px-6 text-center text-sm text-[var(--cp-muted)]">
                  <div>
                    <p className="text-base font-semibold text-[var(--cp-ink)]">No active target</p>
                    <p className="mt-2">
                      Select a contact on the left or enter a DID manually to start using the
                      control-panel chat wrapper.
                    </p>
                  </div>
                </div>
              ) : messagesLoading ? (
                <div className="space-y-3">
                  {Array.from({ length: 5 }).map((_, index) => (
                    <div
                      key={`chat-message-skeleton-${index}`}
                      className="animate-pulse rounded-3xl border border-[var(--cp-border)] bg-white px-4 py-4"
                    >
                      <div className="h-3 w-20 rounded-full bg-[var(--cp-surface-muted)]" />
                      <div className="mt-3 h-3 w-2/3 rounded-full bg-[var(--cp-surface-muted)]" />
                      <div className="mt-2 h-3 w-1/2 rounded-full bg-[var(--cp-surface-muted)]" />
                    </div>
                  ))}
                </div>
              ) : orderedMessages.length ? (
                <div className="space-y-3">
                  {orderedMessages.map((message) => {
                    const outbound = message.direction === 'outbound'
                    return (
                      <div
                        key={message.record_id}
                        className={`flex ${outbound ? 'justify-end' : 'justify-start'}`}
                      >
                        <div
                          className={`max-w-2xl rounded-3xl px-4 py-3 shadow-sm ${
                            outbound
                              ? 'bg-[var(--cp-primary)] text-white'
                              : 'border border-[var(--cp-border)] bg-white text-[var(--cp-ink)]'
                          }`}
                        >
                          <div className="flex flex-wrap items-center gap-2 text-[11px] font-semibold uppercase tracking-wide">
                            <span>{outbound ? 'Outbound' : 'Inbound'}</span>
                            <span className={outbound ? 'text-white/60' : 'text-[var(--cp-muted)]'}>
                              {formatTimestamp(message.created_at_ms)}
                            </span>
                            {message.thread_id ? (
                              <span className={outbound ? 'text-white/70' : 'text-[var(--cp-muted)]'}>
                                thread {message.thread_id}
                              </span>
                            ) : null}
                          </div>
                          <p className={`mt-2 whitespace-pre-wrap text-sm leading-6 ${outbound ? 'text-white' : 'text-[var(--cp-ink)]'}`}>
                            {message.content || '(empty message body)'}
                          </p>
                        </div>
                      </div>
                    )
                  })}
                </div>
              ) : (
                <div className="flex h-full items-center justify-center rounded-3xl border border-dashed border-[var(--cp-border)] bg-white/60 px-6 text-center text-sm text-[var(--cp-muted)]">
                  <div>
                    <p className="text-base font-semibold text-[var(--cp-ink)]">No recent messages</p>
                    <p className="mt-2">
                      This minimal wrapper currently scans recent inbox and outbox messages for the
                      selected peer.
                    </p>
                  </div>
                </div>
              )}
            </div>

            <div className="border-t border-[var(--cp-border)] bg-white px-6 py-5">
              <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto]">
                <textarea
                  value={draft}
                  onChange={(event) => setDraft(event.target.value)}
                  placeholder={
                    !canSend
                      ? 'Messaging is unavailable for this account.'
                      : selectedPeerDid
                        ? 'Send a text message through msg-center...'
                        : 'Select a target to send'
                  }
                  rows={4}
                  disabled={!selectedPeerDid || sending || !canSend}
                  className="min-h-[124px] rounded-3xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3 text-sm text-[var(--cp-ink)] placeholder:text-[var(--cp-muted)] focus:outline-none focus:ring-2 focus:ring-[var(--cp-primary-soft)] disabled:cursor-not-allowed disabled:opacity-60"
                />

                <div className="flex flex-col justify-between gap-3 lg:w-44">
                  <button
                  type="button"
                  onClick={handleSend}
                  disabled={!selectedPeerDid || !draft.trim() || sending || !canSend}
                  className="inline-flex min-h-11 items-center justify-center gap-2 rounded-full bg-[var(--cp-primary)] px-5 py-2 text-sm font-semibold text-white shadow transition hover:bg-[var(--cp-primary-strong)] disabled:cursor-not-allowed disabled:opacity-60"
                >
                    <Icon name="message" className="size-4" />
                    {sending ? 'Sending...' : 'Send'}
                  </button>

                  <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3 text-xs text-[var(--cp-muted)]">
                    <p className="font-semibold uppercase tracking-wide text-[var(--cp-ink)]">
                      Current Notes
                    </p>
                    <div className="mt-2 space-y-1.5">
                      {(bootstrap?.notes ?? []).slice(0, 2).map((note) => (
                        <p key={note}>{note}</p>
                      ))}
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </section>
      </div>
    </div>
  )
}

export default ChatWindow
