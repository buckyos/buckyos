import { useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import { z } from 'zod'
import { buckyos, type DID } from 'buckyos'
import { ConversationEmbedHost, originInZone, type ConversationEmbedErrorCode, type ConversationEmbedHostInit, type ConversationEmbedSend, type ConversationEmbedTurnMode } from 'buckyos/app-frame'
import { fetchAppList } from '../../../api/app_mgr'
import { I18nProvider, useI18n } from '../../../i18n/provider'
import { matchSupportedLocale, type SupportedLocale } from '../../../models/ui'
import { isMockRuntime } from '../../../runtime'
import { PrototypeThemeProvider } from '../../../theme/provider'
import { LoadingOrb } from '../../LoadingOrb'
import { loadConversationBackend } from '../backend'
import type { ConversationAttachmentRef, ConversationBackend } from '../backendTypes'
import { createSourcedSession, isSourced, loadSources, sourcedSessions } from '../agentSessions'
import { InMemoryConversationMessageReader } from '../history/data-source'
import { ConversationSurface } from '../ConversationSurface'
import type { OutgoingPayload } from '../model/outgoing'
import { SessionConversation } from '../SessionConversation'
import type { ConversationSource } from '../source'
import '../conversation.css'

function acceptEmbedder(origin: string): boolean {
  if (isMockRuntime()) return /^https?:\/\/([a-z0-9-]+\.)*(localhost|127\.0\.0\.1)(:\d+)?$/.test(origin)
  const zone = buckyos.getZoneHostName()
  return !!zone && originInZone(origin, zone)
}

function toLocale(locale: string): SupportedLocale {
  return matchSupportedLocale(locale) ?? 'en'
}

const didPattern = /^did:[a-z0-9]+:\S+$/i
/** The embedding App is untrusted input: what it asks for goes through a schema like every other caller's. */
const targetSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('agent'), agentDid: z.string().max(512).regex(didPattern).optional() }).strict(),
  z.object({ kind: z.literal('session'), sessionId: z.string().min(1).max(512) }).strict(),
])

/**
 * §2.3 rule 1: the App is the one whose web host the embedder's origin is (`<web host>.<zone>`), never what the App
 * says it is. The mock runtime has no App list: the first label of a `*.localhost` host stands for it.
 */
async function appIdOfOrigin(origin: string): Promise<string | null> {
  const host = new URL(origin).hostname.toLowerCase()
  if (isMockRuntime()) return /^([a-z0-9-]+)\.localhost$/.exec(host)?.[1] ?? null
  const zone = buckyos.getZoneHostName()?.toLowerCase()
  if (!zone || !host.endsWith(`.${zone}`)) return null
  const label = host.slice(0, -(zone.length + 1))
  const { data } = await fetchAppList()
  return data?.apps.find(app => app.web_hosts.some(web => web.trim().toLowerCase() === label))?.app_id ?? null
}

type Resolved = { kind: 'session'; sessionId: string } | { kind: 'new'; agent: { did: DID; name: string } }
type Phase =
  | { phase: 'waiting' }
  | { phase: 'loading' }
  | { phase: 'error'; code: ConversationEmbedErrorCode | 'unavailable' }
  | { phase: 'ready'; appId: string; target: Resolved }

// One embed per document: the App creates the page for one conversation.
let view: { init: ConversationEmbedHostInit | null; theme: 'light' | 'dark'; locale: string; state: Phase } = { init: null, theme: 'light', locale: 'en', state: { phase: 'waiting' } }
const listeners = new Set<() => void>()
const publish = (next: Partial<typeof view>) => { view = { ...view, ...next }; listeners.forEach(fn => fn()) }
const subscribe = (fn: () => void) => { listeners.add(fn); return () => { listeners.delete(fn) } }

/** Messages the App sent before the conversation was there, and the ones it sends while a send is under way: sent in order. */
let outbox: Promise<void> = Promise.resolve()
const pendingSends: ConversationEmbedSend[] = []

function fail(code: ConversationEmbedErrorCode | 'unavailable') {
  publish({ state: { phase: 'error', code } })
  host.state({ status: 'error', error: code })
  if (code !== 'unavailable') host.error(code)
}

async function resolve(init: ConversationEmbedHostInit, parentOrigin: string) {
  const target = targetSchema.safeParse(init.target)
  if (!target.success) { fail('invalid_params'); return }
  publish({ state: { phase: 'loading' } })
  host.state({ status: 'loading' })
  let backend: ConversationBackend
  let appId: string | null
  try {
    backend = await loadConversationBackend()
    await backend.ensureReady()
    appId = await appIdOfOrigin(parentOrigin)
  } catch { fail('unavailable'); return }
  if (!appId) { fail('denied'); return }
  let resolved: Resolved
  try {
    if (target.data.kind === 'session') {
      // Q2 (a): an App opens only the conversations it started.
      if (!backend.session(target.data.sessionId)) { fail('not_found'); return }
      await loadSources([target.data.sessionId])
      if (!isSourced(target.data.sessionId, appId)) { fail('denied'); return }
      resolved = { kind: 'session', sessionId: target.data.sessionId }
    } else {
      const agent = target.data.agentDid ? { did: target.data.agentDid, name: '' } : await backend.defaultAgent()
      if (!agent) { fail('not_found'); return }
      const [latest] = await sourcedSessions(agent.did, appId)
      resolved = latest ? { kind: 'session', sessionId: latest.id } : { kind: 'new', agent }
    }
  } catch { fail('unavailable'); return }
  publish({ state: { phase: 'ready', appId, target: resolved } })
  host.state({ status: 'ready', ...(resolved.kind === 'session' ? { sessionId: resolved.sessionId } : {}) })
  for (const message of pendingSends.splice(0)) enqueueSend(message)
}

/** The conversation's session; the first message of an App to the Agent starts it, marked with the App's id. */
async function conversationSession(): Promise<string> {
  const state = view.state
  if (state.phase !== 'ready') throw Error('not ready')
  if (state.target.kind === 'session') return state.target.sessionId
  const { agent } = state.target
  const session = await createSourcedSession(agent.did, state.appId, '', agent.name || undefined)
  publish({ state: { ...state, target: { kind: 'session', sessionId: session.id } } })
  host.state({ status: 'ready', sessionId: session.id })
  return session.id
}

/** Q3: a message of the App in the user's name. Zone objects go as attachments; paths, which name no object, as lines of text. */
function enqueueSend(message: ConversationEmbedSend) {
  if (view.state.phase !== 'ready') { pendingSends.push(message); return }
  const objects: ConversationAttachmentRef[] = []
  const paths: string[] = []
  for (const ref of message.attachments ?? []) {
    if (ref.kind === 'object-id') objects.push({ objId: ref.objectId })
    else paths.push(ref.path)
  }
  const text = [message.text.trim(), paths.join('\n')].filter(Boolean).join('\n\n')
  if (!text && objects.length === 0) return
  outbox = outbox.then(async () => {
    const backend = await loadConversationBackend()
    await backend.send(await conversationSession(), { text, attachments: objects })
  }).catch(() => { host.error('denied') })
}

const host = new ConversationEmbedHost({
  accept: acceptEmbedder,
  onInit: init => {
    publish({ init, theme: init.theme, locale: init.locale })
    if (host.parentOrigin) void resolve(init, host.parentOrigin)
  },
  onUpdate: patch => publish(patch),
  onSend: message => enqueueSend(message),
})

const EMPTY_READER = InMemoryConversationMessageReader.empty()

/** Before the App's first message there is no conversation yet: the composer starts it. */
function NewConversation({ agent, placeholder, onTurnMode }: { agent: { did: DID; name: string }; placeholder?: string; onTurnMode: (mode: ConversationEmbedTurnMode) => void }) {
  const { t } = useI18n()
  const source = useMemo<ConversationSource>(() => ({
    sessionId: '',
    selfDid: '',
    reader: EMPTY_READER,
    historyStatus: 'ready',
    hasOlder: false,
    loadOlder: () => Promise.resolve(false),
    markRead: () => undefined,
    access: { mode: 'read_write', canManage: false, canEnableWrite: false, canEditPresentation: false, canEditSharedState: false, canEditOwnMemberState: false },
    canAct: false,
    send: async (payload: OutgoingPayload) => {
      const sessionId = await conversationSession()
      const backend = await loadConversationBackend()
      await backend.post(sessionId, payload)
    },
    displayName: () => agent.name,
    runtime: [],
    preferences: { showActions: true, pinned: null },
    peerMarkdown: true,
  }), [agent.name])
  useEffect(() => onTurnMode('idle'), [onTurnMode])
  return <ConversationSurface source={source} features={{ search: false }} placeholder={placeholder} emptyLabel={t('messagehub.startConversation')} />
}

function EmbeddedConversation({ state, placeholder }: { state: Extract<Phase, { phase: 'ready' }>; placeholder?: string }) {
  const [turn, setTurn] = useState<ConversationEmbedTurnMode | null>(null)
  useEffect(() => { if (turn) host.turn(turn) }, [turn])
  const height = useVisualViewportHeight()
  return (
    <div className="flex w-full flex-col" style={{ height: height ? `${height}px` : '100dvh', background: 'var(--cp-message-canvas)' }} data-testid="conversation-embed">
      {state.target.kind === 'session'
        ? <SessionConversation key={state.target.sessionId} sessionId={state.target.sessionId} testId="conversation-embed-session" placeholder={placeholder} onTurnMode={setTurn} />
        : <NewConversation agent={state.target.agent} placeholder={placeholder} onTurnMode={setTurn} />}
    </div>
  )
}

/** E3: in a frame on a phone the soft keyboard shrinks the visual viewport; the composer stays above it. */
const subscribeViewport = (listener: () => void) => {
  window.visualViewport?.addEventListener('resize', listener)
  return () => window.visualViewport?.removeEventListener('resize', listener)
}
const viewportHeight = () => window.visualViewport ? Math.round(window.visualViewport.height) : null
function useVisualViewportHeight(): number | null {
  return useSyncExternalStore(subscribeViewport, viewportHeight)
}

function Status({ state }: { state: Phase }) {
  const { t } = useI18n()
  if (state.phase === 'error') {
    const text = state.code === 'denied' ? t('messagehub.reason.permission_denied') : state.code === 'not_found' ? t('messagehub.noSessions') : t('messagehub.loadFailed')
    return <div className="mh-session-conversation-status h-[100dvh]" role="alert" data-testid="conversation-embed-error" data-code={state.code}><p>{text}</p></div>
  }
  return <div className="mh-session-conversation-status h-[100dvh]" role="status" data-testid="conversation-embed-loading"><LoadingOrb size={32} /><p>{t('messagehub.loadingHistory')}</p></div>
}

/**
 * The system conversation component alone, for an App to show a conversation with an Agent inside its own layout
 * (`buckyos.conversation-embed`): no Desktop shell. It works with this origin's own session; the App only learns
 * whether the conversation is there and what the Agent is doing, never what was said.
 */
export function ConversationEmbedApp() {
  const { init, theme, locale, state } = useSyncExternalStore(subscribe, () => view)
  if (!host.parentOrigin) return <p role="alert" className="p-6 text-sm">This page only shows a conversation inside an App.</p>
  if (!init) return null
  return (
    <PrototypeThemeProvider mode={theme}>
      <I18nProvider locale={toLocale(locale)}>
        {state.phase === 'ready' ? <EmbeddedConversation state={state} placeholder={init.placeholder} /> : <Status state={state} />}
      </I18nProvider>
    </PrototypeThemeProvider>
  )
}
