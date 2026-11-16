/* The App's state: the user's Agents, the Agent and thing being looked at, the screen of that group, and the
 * requests the page raised outside itself. One screen channel at a time, for the current group; while there is
 * none (reflow mode, browser unavailable) the group's state comes from `browser.status`. */

import type { LocalFile, SelectedContent, ZoneContent } from 'buckyos/content'
import { getMessageHubStore, type MessageHubStore } from '../../messagehub/store'
import type { MessageHubContext } from '../../messagehub/types'
import { getBrowserBackend, type BrowserAgent, type BrowserBackend } from '../api/backend'
import { ScreenChannel, type ChannelFailure, type ChannelStatus } from '../api/channel'
import { browserErrorCode, DEFAULT_GROUP } from '../api/types'
import type { BrowserGroup, BrowserStatus, RequestResult, ScreenFocus, ScreenFrame, ScreenInput, ScreenRequest, ScreenState, ScreenUpstream } from '../api/types'
import { subscribeSources } from '../../../components/conversation/agentSessions'
import { subscribeDefaultBinding } from '../conversation/session'
import { agentBrowserExtensions, type ThingContext } from '../extensions'
import type { AgentBrowserLaunch, BrowserMode } from '../launch'
import { viewportFor, type Size } from '../screen/viewport'
import { toNavigableUrl } from '../shell/address'
import { buildThings, groupIdOfSession, type Thing } from './things'

export type Phase = 'loading' | 'error' | 'empty' | 'pick' | 'agent'
export type PendingAction = 'take_over' | 'hand_back' | 'hand_to_agent' | 'clear_profile' | 'create_thing'
export interface Notice { id: number; kind: 'agent_in_control' | 'offline' | 'failed' | 'profile_cleared'; code?: string }

export interface AgentBrowserSnapshot {
  phase: Phase
  agents: BrowserAgent[]
  agent: BrowserAgent | null
  /** `browser.status` of the Agent; null until the first answer. */
  browser: BrowserStatus | null
  /** The Agent's service did not answer `browser.status`. */
  browserError: boolean
  things: Thing[]
  /** MessageHub for the things' conversations: `unavailable` when it could not be reached (the default thing alone stays usable). */
  conversations: 'loading' | 'ready' | 'unavailable'
  /** The conversation of the current thing is pulled up over the page (original page and reflow modes). */
  chatOpen: boolean
  groupId: string
  mode: BrowserMode
  desktopSite: boolean
  screenStatus: ChannelStatus | 'idle'
  screenFailure: ChannelFailure | null
  screenAttempts: number
  state: ScreenState | null
  /** The current group as the screen last reported it, or `browser.status` while no screen is connected. */
  group: BrowserGroup | null
  /** The last `focus` of the screen: whether the page's focus is on something that takes text. */
  pageFocus: ScreenFocus | null
  /** The browser stopped under the connected screen and `browser.ensure` did not bring it back. */
  restartFailed: boolean
  requests: ScreenRequest[]
  notice: Notice | null
  pending: PendingAction | null
}

const AGENT_KEY = 'buckyos.agent-browser.agent'
const DESKTOP_SITE_KEY = 'buckyos.agent-browser.desktop-site'
const STATUS_POLL_MS = 5000
const VIEWPORT_DEBOUNCE_MS = 120
const NOTICE_MS = 2600
/** How long a stopped browser is given to come back by itself (the service restarts one that crashed) before the App starts it. */
const RESTART_MS = 2500

function remembered(key: string): string | null {
  try { return window.localStorage.getItem(key) } catch { return null }
}

function remember(key: string, value: string | null) {
  try {
    if (value === null) window.localStorage.removeItem(key)
    else window.localStorage.setItem(key, value)
  } catch { /* no storage */ }
}

export const browserUsable = (snapshot: AgentBrowserSnapshot): boolean => !!snapshot.browser && snapshot.browser.enabled && snapshot.browser.available
/** Whether the user's input reaches the page: not while the Agent is operating the group. */
export const canOperate = (snapshot: AgentBrowserSnapshot): boolean => snapshot.group?.controller !== 'agent'
export function currentThing(snapshot: Pick<AgentBrowserSnapshot, 'things' | 'groupId'>): Thing {
  return snapshot.things.find((thing) => thing.groupId === snapshot.groupId) ?? { groupId: snapshot.groupId, session: null, title: snapshot.groupId === DEFAULT_GROUP ? '' : snapshot.groupId, lastActiveAt: 0 }
}

export class AgentBrowserStore {
  private snapshot: AgentBrowserSnapshot = {
    phase: 'loading', agents: [], agent: null, browser: null, browserError: false, things: [], conversations: 'loading', chatOpen: false, groupId: DEFAULT_GROUP, mode: 'page',
    desktopSite: remembered(DESKTOP_SITE_KEY) === '1', screenStatus: 'idle', screenFailure: null, screenAttempts: 0, state: null,
    group: null, pageFocus: null, restartFailed: false, requests: [], notice: null, pending: null,
  }
  private readonly listeners = new Set<() => void>()
  private readonly frameListeners = new Set<(frame: ScreenFrame) => void>()
  private backend: BrowserBackend | null = null
  private channel: ScreenChannel | null = null
  private hub: MessageHubStore | null = null
  private hubContext: MessageHubContext | null = null
  private stopHub: (() => void) | null = null
  /** The groups `browser.status` last listed: a session that has one is a thing, whoever started it. */
  private browserGroups: ReadonlySet<string> = new Set()
  private screenSize: Size | null = null
  private stageSize: Size | null = null
  private pollTimer = 0
  private viewportTimer = 0
  private noticeTimer = 0
  private restartTimer = 0
  private noticeId = 0
  private pressedAt = -Infinity
  private epoch = 0
  private run = 0
  private disposed = false

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  getSnapshot = () => this.snapshot

  private update(patch: Partial<AgentBrowserSnapshot>) {
    if (this.disposed) return
    this.snapshot = { ...this.snapshot, ...patch }
    for (const listener of [...this.listeners]) listener()
  }

  // ── Agents ──

  async start(launch?: AgentBrowserLaunch) {
    const run = ++this.run
    this.disposed = false
    this.update({ phase: 'loading' })
    try {
      this.backend = await getBrowserBackend()
      const agents = await this.backend.agents()
      if (run !== this.run) return
      if (agents.length === 0) { this.update({ phase: 'empty', agents }); return }
      const wanted = launch?.agent ?? remembered(AGENT_KEY)
      const agent = agents.find((item) => item.agentId === wanted || item.agentDid === wanted) ?? (agents.length === 1 ? agents[0] : null)
      this.update({ agents, ...(launch?.mode ? { mode: launch.mode } : {}) })
      if (agent) this.enter(agent, launch?.group ? groupIdOfSession({ id: launch.group }, agent.agentDid, null) : DEFAULT_GROUP)
      else this.update({ phase: 'pick' })
    } catch {
      if (run === this.run) this.update({ phase: 'error' })
    }
  }

  retry() {
    void this.start()
  }

  selectAgent(agentId: string) {
    const agent = this.snapshot.agents.find((item) => item.agentId === agentId)
    if (agent) this.enter(agent, DEFAULT_GROUP)
  }

  /** Back to the list of Agents (only offered when there are several). */
  leaveAgent() {
    this.leave()
    remember(AGENT_KEY, null)
    this.update({ phase: 'pick', agent: null })
  }

  private leave() {
    this.epoch += 1
    this.closeChannel()
    window.clearInterval(this.pollTimer)
    this.pollTimer = 0
    this.stopHub?.()
    this.stopHub = null
    this.hubContext = null
    this.browserGroups = new Set()
    this.update({ browser: null, browserError: false, things: [], conversations: 'loading', chatOpen: false, state: null, group: null, pageFocus: null, restartFailed: false, requests: [], pending: null })
  }

  private enter(agent: BrowserAgent, groupId: string) {
    this.leave()
    remember(AGENT_KEY, agent.agentId)
    this.update({ phase: 'agent', agent, groupId, things: buildThings([], agent.agentDid, null, () => '', () => true) })
    void this.refreshStatus()
    void this.enterHub()
  }

  // ── Things ──

  private async enterHub() {
    const epoch = this.epoch
    const hub = getMessageHubStore()
    const retry = this.snapshot.conversations === 'unavailable'
    this.update({ conversations: 'loading' })
    try {
      await hub.initialize()
      const context = hub.defaultContext()
      await hub.ensureOwner(context, retry)
      if (this.disposed || epoch !== this.epoch) return
      if (hub.ownerStatus(context).phase !== 'ready') throw Error('conversations unavailable')
      this.hub = hub
      this.hubContext = context
      const unsubscribe = hub.subscribe(() => this.refreshThings())
      const stopSync = hub.startSync(context, null)
      const stopMarks = subscribeSources(() => this.refreshThings())
      const stopBinding = subscribeDefaultBinding((agentId) => { if (agentId === this.snapshot.agent?.agentId) void this.refreshStatus() })
      this.stopHub = () => { unsubscribe(); stopSync(); stopMarks(); stopBinding() }
      this.update({ conversations: 'ready' })
      this.refreshThings()
    } catch {
      // The default thing alone stays usable.
      if (!this.disposed && epoch === this.epoch) this.update({ conversations: 'unavailable' })
    }
  }

  retryConversations() {
    if (this.snapshot.conversations === 'unavailable') void this.enterHub()
  }

  thingContext(): ThingContext | null {
    const { agent } = this.snapshot
    return agent && this.hub && this.hubContext ? { agent, hub: this.hub, context: this.hubContext } : null
  }

  private refreshThings() {
    const context = this.thingContext()
    if (!context) return
    const { hub, agent } = context
    const listed = agentBrowserExtensions.listsThing
    const { groupId, browser } = this.snapshot
    const things = buildThings(
      hub.sessions(context.context, agent.agentDid, 'active'), agent.agentDid, browser?.default_session_id ?? null,
      (session) => hub.title(context.context, session),
      (session) => session.id === groupId || this.browserGroups.has(session.id) || (listed?.(session, context) ?? true),
    )
    const before = this.snapshot.things
    const same = before.length === things.length && things.every((thing, index) => {
      const old = before[index]
      return old.groupId === thing.groupId && old.title === thing.title && old.session?.id === thing.session?.id && old.lastActiveAt === thing.lastActiveAt && old.session?.unreadCount === thing.session?.unreadCount
    })
    if (!same) this.update({ things })
  }

  selectThing(groupId: string) {
    if (groupId === this.snapshot.groupId) return
    this.closeChannel()
    this.update({ groupId, state: null, group: null, requests: [] })
    this.refreshThings()
    this.syncChannel()
    void this.refreshStatus()
  }

  canCreateThing(): boolean {
    return !!agentBrowserExtensions.createThing && !!this.thingContext()
  }

  async createThing() {
    const context = this.thingContext()
    const create = agentBrowserExtensions.createThing
    if (!context || !create || this.snapshot.pending) return
    this.update({ pending: 'create_thing' })
    try {
      const session = await create(context)
      this.refreshThings()
      if (session) this.selectThing(groupIdOfSession(session, context.agent.agentDid, this.snapshot.browser?.default_session_id ?? null))
    } catch (error) {
      this.notify('failed', browserErrorCode(error) ?? undefined)
    } finally {
      this.update({ pending: null })
    }
  }

  // ── Browser status ──

  async refreshStatus() {
    const { agent, groupId } = this.snapshot
    const backend = this.backend
    if (!agent || !backend) return
    const epoch = this.epoch
    try {
      const browser = await backend.call<BrowserStatus>(agent, 'browser.status', { group_id: groupId })
      if (epoch !== this.epoch || groupId !== this.snapshot.groupId) return
      // The session the default group is bound to is the default thing, not one of its own.
      if (groupId !== DEFAULT_GROUP && groupId === browser.default_session_id) { this.selectThing(DEFAULT_GROUP); return }
      const group = browser.groups.find((item) => item.group_id === groupId) ?? null
      this.update({ browser, browserError: false, ...(this.channel?.status === 'open' ? {} : { group }) })
      this.refreshThings()
    } catch {
      if (epoch !== this.epoch) return
      this.update({ browserError: true })
    }
    this.syncChannel()
  }

  // ── Screen ──

  setMode(mode: BrowserMode) {
    if (mode === this.snapshot.mode) return
    this.update({ mode, ...(mode === 'chat' ? { chatOpen: false } : {}) })
    this.syncChannel()
  }

  /** Pulls the current thing's conversation up over the page, or puts it away; in conversation mode it is the stage already. */
  setChatOpen(chatOpen: boolean) {
    if (chatOpen !== this.snapshot.chatOpen && this.snapshot.mode !== 'chat') this.update({ chatOpen })
  }

  setDesktopSite(desktopSite: boolean) {
    if (desktopSite === this.snapshot.desktopSite) return
    remember(DESKTOP_SITE_KEY, desktopSite ? '1' : null)
    this.update({ desktopSite })
    this.syncChannel()
  }

  /** The size of the screen view (null when it is not shown) and of the area it would take. */
  setScreenSize(size: Size | null) {
    this.screenSize = size
    this.scheduleViewport()
  }

  setStageSize(size: Size) {
    this.stageSize = size
    if (!this.screenSize) this.scheduleViewport()
  }

  private scheduleViewport() {
    window.clearTimeout(this.viewportTimer)
    if (!this.channel) { this.syncChannel(); return }
    this.viewportTimer = window.setTimeout(() => this.syncChannel(), VIEWPORT_DEBOUNCE_MS)
  }

  private closeChannel() {
    window.clearTimeout(this.viewportTimer)
    window.clearTimeout(this.restartTimer)
    if (!this.channel) return
    this.channel.close()
    this.channel = null
    this.update({ screenStatus: 'idle', screenFailure: null, screenAttempts: 0, pageFocus: null, restartFailed: false })
  }

  private syncChannel() {
    if (this.disposed) return
    const { phase, agent, groupId, mode, desktopSite } = this.snapshot
    const backend = this.backend
    const size = this.screenSize ?? this.stageSize
    const wanted = phase === 'agent' && !!agent && !!backend && browserUsable(this.snapshot) && mode !== 'reflow' && !!size && size.width > 0 && size.height > 0
    if (!wanted || !agent || !backend || !size) {
      this.closeChannel()
      this.syncPolling()
      return
    }
    const viewport = viewportFor(size, desktopSite)
    if (this.channel) {
      this.channel.setViewport(viewport)
      return
    }
    const channel = new ScreenChannel({
      groupId,
      viewport,
      prepare: async () => {
        try {
          await backend.call<BrowserStatus>(agent, 'browser.ensure', {})
        } catch (error) {
          throw Object.assign(new Error(error instanceof Error ? error.message : String(error)), { code: browserErrorCode(error) ?? 'transport' })
        }
        return backend.token(agent)
      },
      open: () => backend.openScreen(agent),
      onStatus: (status, failure, attempts) => {
        if (this.channel !== channel) return
        // A new connection is told again what the pages are waiting for; the notices it is not told again stay.
        const requests = status === 'open' ? this.snapshot.requests.filter((item) => item.kind === 'download' || item.kind === 'permission') : this.snapshot.requests
        this.update({ screenStatus: status, screenFailure: failure, screenAttempts: attempts, requests })
        if (status === 'failed' && failure?.code === 'browser_unavailable') void this.refreshStatus()
      },
      onState: (state) => {
        if (this.channel !== channel) return
        this.update({ state, group: state.group, ...(state.running ? { restartFailed: false } : {}) })
        this.watchRunning(state.running)
      },
      onFrame: (frame) => { if (this.channel === channel) for (const listener of [...this.frameListeners]) listener(frame) },
      onRequest: (request) => {
        if (this.channel !== channel) return
        this.update({ requests: [...this.snapshot.requests.filter((item) => item.request_id !== request.request_id), request] })
      },
      onRequestClosed: (requestId) => { if (this.channel === channel) this.removeRequest(requestId) },
      onFocus: (pageFocus) => { if (this.channel === channel) this.update({ pageFocus }) },
      onError: (failure) => {
        if (this.channel !== channel) return
        this.notify(failure.code === 'agent_in_control' ? 'agent_in_control' : 'failed', failure.code)
      },
    })
    this.channel = channel
    this.update({ screenStatus: 'connecting', screenFailure: null, screenAttempts: 0 })
    channel.start()
    this.syncPolling()
  }

  /** `browser.status` keeps the group's state fresh while no channel does, and notices the browser becoming available. */
  private syncPolling() {
    const polling = this.snapshot.phase === 'agent' && !this.channel
    if (polling && !this.pollTimer) this.pollTimer = window.setInterval(() => void this.refreshStatus(), STATUS_POLL_MS)
    if (!polling && this.pollTimer) { window.clearInterval(this.pollTimer); this.pollTimer = 0 }
  }

  retryScreen() {
    if (this.channel?.status === 'failed') this.channel.retry()
    else if (this.channel && this.snapshot.state?.running === false) void this.restartBrowser()
    else void this.refreshStatus()
  }

  /**
   * A browser that stopped under the connected screen (its sign-ins were cleared, it kept crashing) stays stopped
   * until someone asks for it: the screen that is looking does, once the service had the time to restart it itself.
   */
  private watchRunning(running: boolean) {
    window.clearTimeout(this.restartTimer)
    if (!running && !this.snapshot.restartFailed) this.restartTimer = window.setTimeout(() => void this.restartBrowser(), RESTART_MS)
  }

  private async restartBrowser() {
    window.clearTimeout(this.restartTimer)
    const { agent } = this.snapshot
    const backend = this.backend
    const channel = this.channel
    if (!agent || !backend || !channel || this.snapshot.state?.running !== false) return
    this.update({ restartFailed: false })
    try {
      await backend.call<BrowserStatus>(agent, 'browser.ensure', {})
    } catch {
      if (this.channel !== channel || this.snapshot.state?.running !== false) return
      this.update({ restartFailed: true })
      void this.refreshStatus()
    }
  }

  subscribeFrames(listener: (frame: ScreenFrame) => void) {
    this.frameListeners.add(listener)
    return () => { this.frameListeners.delete(listener) }
  }

  lastFrame(): ScreenFrame | null {
    return this.channel?.lastFrame ?? null
  }

  ackFrame(seq: number) {
    this.channel?.ack(seq)
  }

  private notify(kind: Notice['kind'], code?: string) {
    window.clearTimeout(this.noticeTimer)
    const id = ++this.noticeId
    this.update({ notice: { id, kind, ...(code ? { code } : {}) } })
    this.noticeTimer = window.setTimeout(() => { if (this.snapshot.notice?.id === id) this.update({ notice: null }) }, NOTICE_MS)
  }

  /** Sends what the user did to the page; refused (with a notice unless `quiet`) while the Agent operates or the screen is not connected. */
  private operate(message: ScreenUpstream, quiet = false): boolean {
    if (!canOperate(this.snapshot)) { if (!quiet) this.notify('agent_in_control'); return false }
    if (!this.channel?.send(message)) { if (!quiet) this.notify('offline'); return false }
    return true
  }

  /** `quiet` is for what follows a gesture that was already refused or announced (moves, releases). */
  sendInput(input: ScreenInput, quiet = false): boolean {
    const sent = this.operate({ type: 'input', ...input }, quiet)
    if (sent && (input.kind === 'key' || (input.kind === 'mouse' && input.action !== 'move') || (input.kind === 'touch' && input.phase !== 'move'))) this.pressedAt = performance.now()
    return sent
  }

  /** Whether the user pressed the page (pointer or key) within `ms`: what the page asks for right after is the answer to that press. */
  pressedWithin(ms: number): boolean {
    return performance.now() - this.pressedAt <= ms
  }

  /** Every group of the Agent's browser that has tabs, for the list of things. */
  async groups(): Promise<BrowserGroup[]> {
    const { agent } = this.snapshot
    if (!agent || !this.backend) return []
    const epoch = this.epoch
    const { groups } = await this.backend.call<BrowserStatus>(agent, 'browser.status', {})
    if (epoch === this.epoch) {
      this.browserGroups = new Set(groups.map((group) => group.group_id))
      this.refreshThings()
    }
    return groups
  }

  /** Whether an address was sent: a blank tab is opened for it when the group has none. */
  navigate(text: string): boolean {
    const url = toNavigableUrl(text)
    if (!url) return false
    return this.operate(this.snapshot.state?.active_tab_id ? { type: 'nav', action: 'url', url } : { type: 'tab', action: 'open', url })
  }

  nav(action: 'back' | 'forward' | 'reload' | 'stop') {
    this.operate({ type: 'nav', action })
  }

  openTab(url?: string) {
    this.operate({ type: 'tab', action: 'open', ...(url ? { url } : {}) })
  }

  closeTab(tabId: string) {
    this.operate({ type: 'tab', action: 'close', tab_id: tabId })
  }

  activateTab(tabId: string) {
    this.operate({ type: 'tab', action: 'activate', tab_id: tabId })
  }

  // ── Requests outside the page ──

  private removeRequest(requestId: string) {
    if (this.snapshot.requests.some((item) => item.request_id === requestId)) this.update({ requests: this.snapshot.requests.filter((item) => item.request_id !== requestId) })
  }

  respond(requestId: string, result: RequestResult) {
    this.channel?.send({ type: 'respond', request_id: requestId, result })
    this.removeRequest(requestId)
  }

  /** A notice-only request (download, permission) the user has seen. */
  dismissRequest(requestId: string) {
    this.removeRequest(requestId)
  }

  /** Hands the chosen files to a `file_chooser` request: Zone files by path, files of this device by their bytes; the last one carries `done`. */
  async uploadFiles(requestId: string, items: SelectedContent[]) {
    const { agent } = this.snapshot
    const backend = this.backend
    if (!agent || !backend) return
    const files = items.filter((item): item is ZoneContent | LocalFile => item.kind === 'file')
    if (files.length === 0) { this.respond(requestId, { cancel: true }); return }
    try {
      for (const [index, item] of files.entries()) {
        const done = index === files.length - 1
        if (item.origin === 'local') await backend.upload(agent, requestId, item.file, done)
        else {
          if (item.source.kind !== 'cyfs-path') throw Error('Only a Zone path can be uploaded')
          await backend.call(agent, 'browser.upload', { request_id: requestId, zone_path: item.source.path.slice('cyfs://'.length) || '/', ...(done ? { done: true } : {}) })
        }
      }
      this.removeRequest(requestId)
    } catch (error) {
      this.notify('failed', browserErrorCode(error) ?? undefined)
    }
  }

  async exportDownload(downloadId: string): Promise<{ obj_id: string; name: string; size: number }> {
    const { agent } = this.snapshot
    if (!agent || !this.backend) throw Error('no Agent')
    return this.backend.call(agent, 'browser.download_export', { download_id: downloadId })
  }

  reportFailure(error: unknown) {
    this.notify('failed', browserErrorCode(error) ?? undefined)
  }

  // ── Handover ──

  private async groupAction(method: 'take_over' | 'hand_back' | 'hand_to_agent'): Promise<BrowserGroup | null> {
    const { agent, groupId, pending } = this.snapshot
    const backend = this.backend
    if (!agent || !backend || pending) return null
    this.update({ pending: method })
    try {
      const group = await backend.call<BrowserGroup>(agent, `browser.${method}`, { group_id: groupId })
      if (groupId === this.snapshot.groupId) this.update({ group })
      return group
    } catch (error) {
      this.notify('failed', browserErrorCode(error) ?? undefined)
      return null
    } finally {
      this.update({ pending: null })
    }
  }

  takeOver() {
    return this.groupAction('take_over')
  }

  /** `note`: the message that wakes the Agent (DG3); empty hands back without one. */
  async handBack(note = '') {
    await this.handover('hand_back', note)
  }

  /** `note`: what the owner adds to "continue from this page" (DG2). */
  async handToAgent(note = '') {
    await this.handover('hand_to_agent', note)
  }

  private async handover(kind: 'hand_back' | 'hand_to_agent', note: string) {
    const thing = currentThing(this.snapshot)
    const { agent, state } = this.snapshot
    const group = await this.groupAction(kind)
    if (!group || !agent) return
    const tab = group.tabs.find((item) => item.active)
    try {
      await agentBrowserExtensions.afterHandover?.({ kind, agent, thing, group, url: tab?.url ?? state?.url ?? '', title: tab?.title ?? state?.title ?? '', note }, this.thingContext())
    } catch {
      this.notify('failed', 'message_not_sent')
    }
  }

  // ── Settings ──

  async clearProfile(): Promise<boolean> {
    const { agent, pending } = this.snapshot
    const backend = this.backend
    if (!agent || !backend || pending) return false
    this.update({ pending: 'clear_profile' })
    try {
      await backend.call(agent, 'browser.clear_profile', {})
      this.notify('profile_cleared')
      // Clearing stops the browser: the screen that is looking asks for it again, with its tabs.
      if (this.channel) void backend.call<BrowserStatus>(agent, 'browser.ensure', {}).catch(() => undefined)
      return true
    } catch (error) {
      this.notify('failed', browserErrorCode(error) ?? undefined)
      return false
    } finally {
      this.update({ pending: null })
    }
  }

  dispose() {
    this.run += 1
    this.leave()
    window.clearTimeout(this.noticeTimer)
    this.disposed = true
  }
}
