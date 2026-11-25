/* A page tidied by itself ("this page only"), shown in home mode: a fallback page of the reflow bucket. For its
 * address, what the bucket has (`browser.reflow_state`, which starts no browser) decides what shows — the page that
 * was made, or that there is none yet. Making one is the Agent's, and always the owner's own request: a message in
 * the browser conversation. One controller per App window; which address it shows is the home's (site/controller.ts). */

import { getBrowserBackend, type BrowserAgent } from '../api/backend'
import { browserErrorCode } from '../api/types'
import { agentBrowserExtensions } from '../extensions'
import { pageOnlyMessage } from '../site/messages'
import type { AgentBrowserStore } from '../store/store'
import { ReflowError, libraryState, normalizeUrl, reflowAddress, type ReflowState } from './library'
import { getReflowSource } from './source'

export type Translate = (key: string, fallback?: string, variables?: Record<string, string | number>) => string

export type ReflowPhase =
  /** No address. */
  | 'blank'
  | 'loading'
  /** A page is shown. */
  | 'view'
  /** Nothing to show yet: the Agent was asked. */
  | 'making'
  /** Nothing to show and nobody is making it (`reason`). */
  | 'unmade'
  /** The address has no reflow (not an http(s) page). */
  | 'unsupported'
  | 'failed'

export interface ReflowNotice {
  id: number
  kind: 'agent_asked' | 'agent_unreachable'
}

export interface ReflowSnapshot {
  /** The address of the page, as the home gave it; empty while it shows none. */
  url: string
  /** The title the original page had when the owner asked for it; empty when unknown. */
  title: string
  /** The one line the owner added when asking for it; empty without one. */
  note: string
  phase: ReflowPhase
  state: ReflowState | null
  /** The page the frame shows (a path of protocol §5.3) and the count that loads it again. */
  view: string | null
  frameKey: number
  /** The Agent was asked about this address and has not delivered yet. */
  asked: boolean
  /** Why nothing is being made (`unmade`): nobody was asked yet, or the Agent could not be written to. */
  reason: 'not_made' | 'agent_unreachable' | null
  notice: ReflowNotice | null
}

/** What a tidied page tells the App (protocol §5.5). */
type PageMessage = { type?: unknown; url?: unknown }

/** What the page showed when the Agent was asked: the request is met once it shows something else. */
interface Asked { view: string | null; madeAt: number }

const AGENT_POLL_MS = 4000
const ASKED_KEY = 'buckyos.agent-browser.reflow.asked'

/** Requests to the Agent that are still open, by Agent and normalized address; kept for the browser tab so a reload does not ask again. */
const asked = new Map<string, Asked>((() => {
  try { return JSON.parse(window.sessionStorage.getItem(ASKED_KEY) ?? '[]') as Array<[string, Asked]> } catch { return [] }
})())

function saveAsked() {
  try { window.sessionStorage.setItem(ASKED_KEY, JSON.stringify([...asked])) } catch { /* no storage */ }
}

const askedKey = (agentId: string, url: string) => `${agentId}\n${url}`

/** What the bucket has for an address: the Agent's service says, and when it does not answer the bucket itself is read, so a page that was made shows without the Agent. */
export async function readReflowState(agent: BrowserAgent, url: string): Promise<ReflowState> {
  try {
    return await (await getBrowserBackend()).call<ReflowState>(agent, 'browser.reflow_state', { url })
  } catch (error) {
    if (browserErrorCode(error) === 'invalid_argument') throw error
    return libraryState(await (await getReflowSource()).library(), await reflowAddress(url))
  }
}

const BLANK: ReflowSnapshot = { url: '', title: '', note: '', phase: 'blank', state: null, view: null, frameKey: 0, asked: false, reason: null, notice: null }

export class ReflowController {
  private readonly store: AgentBrowserStore
  private snapshot: ReflowSnapshot = BLANK
  private readonly listeners = new Set<() => void>()
  private active = false
  private t: Translate = (key, fallback) => fallback ?? key
  private openLink: (url: string) => void = () => undefined
  private evaluating: Promise<void> = Promise.resolve()
  private timer = 0
  private run = 0
  private noticeId = 0
  private shown: { view: string; madeAt: number } | null = null

  constructor(store: AgentBrowserStore) {
    this.store = store
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  getSnapshot = () => this.snapshot

  private set(patch: Partial<ReflowSnapshot>) {
    this.snapshot = { ...this.snapshot, ...patch }
    for (const listener of [...this.listeners]) listener()
  }

  private notify(kind: ReflowNotice['kind']) {
    this.set({ notice: { id: ++this.noticeId, kind } })
  }

  dismissNotice() {
    if (this.snapshot.notice) this.set({ notice: null })
  }

  // ── The address ──

  /** The language the Agent is written to in when a page is asked for again. */
  setTranslate(t: Translate) {
    this.t = t
  }

  /** Where a link of the shown page goes: the home decides (a page of the site, another tidied page, the original site). */
  setOpenLink(open: (url: string) => void) {
    this.openLink = open
  }

  /** The panel is shown: what is waited for is looked at again from time to time. */
  activate() {
    this.active = true
    this.schedule()
  }

  deactivate() {
    this.active = false
    window.clearTimeout(this.timer)
  }

  /** The page to show; `title` is the original page's when the owner just came from it. */
  show(url: string, title = '') {
    this.set({ url, title, ...(url === this.snapshot.url ? {} : { note: '' }) })
    this.evaluating = this.evaluate()
  }

  retry() {
    this.evaluating = this.evaluate()
  }

  /** Whether the Agent was asked for the page of this address and has not delivered it yet. */
  waitsFor(url: string): boolean {
    const { agent } = this.store.getSnapshot()
    try {
      return !!agent && asked.has(askedKey(agent.agentId, normalizeUrl(url).url))
    } catch {
      return false
    }
  }

  // ── What the bucket has ──

  private async evaluate() {
    const run = ++this.run
    window.clearTimeout(this.timer)
    this.shown = null
    const { url } = this.snapshot
    const { agent } = this.store.getSnapshot()
    const cleared = { state: null, view: null, asked: false, reason: null, notice: null }
    if (!url || !agent) { this.set({ ...cleared, phase: 'blank' }); return }
    this.set({ ...cleared, phase: 'loading' })
    try {
      const state = await readReflowState(agent, url)
      if (run !== this.run) return
      this.apply(state)
      // Nothing is made by looking: without a page or a request to the Agent, the owner chooses.
      if (this.snapshot.phase === 'loading') this.set({ phase: 'unmade', reason: 'not_made' })
      else this.schedule()
    } catch (error) {
      if (run !== this.run) return
      this.set({ phase: error instanceof ReflowError || browserErrorCode(error) === 'invalid_argument' ? 'unsupported' : 'failed' })
    }
  }

  /** Takes a new state of the address: shows what it has (a page made again is loaded anew) and settles what was waited for. */
  private apply(state: ReflowState) {
    const before = this.snapshot
    const key = askedKey(this.store.getSnapshot().agent?.agentId ?? '', state.url)
    const view = state.view
    const madeAt = view ? state.fallback?.made_at ?? 0 : 0
    let frameKey = before.frameKey
    if (view) {
      if (this.shown?.view !== view || this.shown.madeAt !== madeAt) frameKey += 1
      this.shown = { view, madeAt }
      const was = asked.get(key)
      if (was && (was.view !== view || was.madeAt !== madeAt)) {
        asked.delete(key)
        saveAsked()
      }
    } else {
      this.shown = null
    }
    const waiting = asked.has(key)
    this.set({ state, view, frameKey, asked: waiting, phase: view ? 'view' : waiting ? 'making' : before.phase })
  }

  private schedule() {
    window.clearTimeout(this.timer)
    if (this.active && this.snapshot.asked) this.timer = window.setTimeout(() => void this.poll(), AGENT_POLL_MS)
  }

  private async poll() {
    const run = this.run
    const url = this.snapshot.state?.url
    const { agent } = this.store.getSnapshot()
    if (!url || !agent) return
    try {
      const state = await readReflowState(agent, url)
      if (run !== this.run) return
      this.apply(state)
    } catch {
      /* neither the service nor the bucket answered: asked again at the next turn */
    }
    if (run === this.run) this.schedule()
  }

  // ── The Agent ──

  private unreachable() {
    if (this.snapshot.view) this.notify('agent_unreachable')
    else this.set({ phase: 'unmade', reason: 'agent_unreachable' })
  }

  /** Writes to the Agent in the browser conversation about this address: once until the page changes, unless the owner asks `again`. */
  private async ask(text: string, again: boolean): Promise<'asked' | 'waiting' | 'unreachable'> {
    const { state } = this.snapshot
    const agent = this.store.getSnapshot().agent
    const send = agentBrowserExtensions.askAgent
    if (!state || !agent) return 'unreachable'
    const key = askedKey(agent.agentId, state.url)
    const run = this.run
    let outcome: 'asked' | 'waiting' = 'waiting'
    if (again || !asked.has(key)) {
      if (!send) return 'unreachable'
      asked.set(key, this.shown ?? { view: null, madeAt: 0 })
      try {
        const context = await this.store.whenConversation()
        if (!context) throw Error('the conversation is not available')
        // The owner went elsewhere while the conversation came up: nothing is asked for a page they left.
        if (run !== this.run) {
          asked.delete(key)
          return outcome
        }
        await send({ text }, context)
      } catch {
        asked.delete(key)
        return 'unreachable'
      }
      saveAsked()
      outcome = 'asked'
    }
    if (run !== this.run) return outcome
    this.set({ asked: true, reason: null, phase: this.snapshot.view ? 'view' : 'making' })
    this.schedule()
    return outcome
  }

  /** The owner asks the Agent to tidy the page that is not there, with the fixed-format message. */
  async askAgent() {
    const { state, title, note } = this.snapshot
    if (!state) return
    const run = this.run
    const outcome = await this.ask(pageOnlyMessage({ url: state.url, title, note }), true)
    if (run === this.run && outcome === 'unreachable') this.unreachable()
  }

  /** The owner chose "tidy this page only" for the page being shown, maybe with a line for the Agent: it is asked once it is known that there is none. */
  async request(note: string) {
    this.set({ note: note.trim() })
    await this.evaluating
    if (this.snapshot.phase === 'unmade' && this.snapshot.reason === 'not_made') await this.askAgent()
  }

  /** The owner wants the page made again from the site as it is now. */
  async redo() {
    const { state, view } = this.snapshot
    if (!state) return
    if (!view) { await this.askAgent(); return }
    const run = this.run
    const text = [
      this.t('agentBrowser.reflow.ask.redo', 'Please make the reflow page of this address again, from the site as it is now: {{url}}', { url: state.url }),
      this.t('agentBrowser.reflow.ask.redoDone', 'Keep it in my reflow library when it is done (reflow result put).'),
    ].join('\n')
    const outcome = await this.ask(text, false)
    if (run === this.run) this.notify(outcome === 'unreachable' ? 'agent_unreachable' : 'agent_asked')
  }

  // ── The page ──

  /** A message of the page in the frame; the frame has checked where it came from. */
  onPageMessage(data: unknown) {
    const message = data as PageMessage | null
    if (!message || typeof message !== 'object') return
    if (message.type === 'reflow.source') this.openSource()
    else if (message.type === 'reflow.open' && typeof message.url === 'string') this.open(message.url)
  }

  /** A link of the page: an http(s) address, a relative one against the original page. */
  private open(link: string) {
    let url: URL
    try {
      url = new URL(link, this.snapshot.state?.url)
    } catch {
      return
    }
    if (url.protocol === 'http:' || url.protocol === 'https:') this.openLink(url.href)
  }

  /** Back to the original page: the browser opens what this page was made from. */
  openSource() {
    const { state } = this.snapshot
    const url = state?.fallback?.source_url || state?.url || this.snapshot.url
    if (url) this.store.openInPage(url)
    else this.store.setMode('page')
  }
}

const controllers = new WeakMap<AgentBrowserStore, ReflowController>()

/** The tidied-page controller of an App window. */
export function reflowControllerOf(store: AgentBrowserStore): ReflowController {
  let controller = controllers.get(store)
  if (!controller) {
    controller = new ReflowController(store)
    controllers.set(store, controller)
  }
  return controller
}
