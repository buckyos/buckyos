/* The Agent Service of the mock runtime (`VITE_CP_USE_MOCK`): kRPC `browser.*`, the screen socket and the upload,
 * speaking the wire format of the protocol document, so the App above api/backend.ts runs the same code as on a
 * Zone. The Agent's one group has tabs showing the page of mock/page.ts; the Agent's side (lease, release, handover
 * request) is driven from `window.__agentBrowserMock`, which the e2e specs also read the received messages from. */

import { mockOnlineInPlace, mockOnlineStatus, setMockOnline } from '../../../api/settings_mock'
import { getMockEntityDid } from '../../messagehub/mock/data'
import { ownReadyAgents, toBrowserAgent, type BrowserAgent, type BrowserBackend } from '../api/backend'
import type { ScreenSocket } from '../api/channel'
import { DEFAULT_GROUP, VIEWPORT_MAX, VIEWPORT_MIN } from '../api/types'
import type { AgentState, BrowserGroup, BrowserStatus, BrowserTab, HandoverReason, PageHint, RequestResult, ScreenDownstream, ScreenInput, ScreenRequest, ScreenState, ScreenUpstream, UnavailableCode, Viewport } from '../api/types'
import { MOCK_PAGE_HEIGHT, mockElementAt, mockPageRects, mockSelectOptions, newMockPage, renderMockPage, type MockElement, type MockPage } from './page'
import { mockManageCall, mockManageControl } from './manage'
import { mockReflowControl, mockReflowState } from './reflow'
import { mockSeed } from './site'

const TOKEN = 'mock-session-token'
const MAX_TABS = 12
const LATENCY_MS = 4
const NAV_MS = 120

/** The MessageHub mock's own Agent: the one its Agent panel belongs to. `agent.list` of the mock control panel does not list it. */
export const MOCK_HUB_AGENT: BrowserAgent = { agentId: 'codeassistant', agentDid: getMockEntityDid('agent-coder'), name: 'CodeAssistant', readOnly: mockSeed.readOnly?.includes('codeassistant') ?? false }

interface MockTab { id: string; history: string[]; cursor: number; loading: boolean; page: MockPage }

interface MockGroup {
  id: string
  tabs: MockTab[]
  activeTabId: string | null
  userHold: boolean
  lease: BrowserGroup['lease']
  handover: BrowserGroup['handover']
  released: { state: 'done' | 'stuck'; note: string | null } | null
  waitingAnswer: boolean
  requests: Map<string, ScreenRequest>
}

interface MockBrowser {
  enabled: boolean
  /** ext-core is on the node; the browser itself is ext-core's online item `chromium`, which the mock control panel downloads (api/settings_mock.ts). */
  available: boolean
  unavailable: string | null
  running: boolean
  signedIn: boolean
  defaultSessionId: string | null
  groups: Map<string, MockGroup>
  downloads: Map<string, { name: string; size: number }>
}

export interface MockLogEntry { agentId: string; groupId: string; dropped: boolean; message: ScreenUpstream }
export interface MockUpload { agentId: string; requestId: string; name: string; source: 'zone' | 'local'; done: boolean }

class MockError extends Error {
  readonly code: string
  constructor(code: string, message = code, unavailableCode?: UnavailableCode) {
    super(`RPC call error: Failed due to reason: ${JSON.stringify({ kind: 'browser', code, ...(unavailableCode ? { unavailable_code: unavailableCode } : {}), message })}`)
    this.code = code
  }
}

/** The node the mock Agents run on, and the online item that is their browser. */
const MOCK_NODE = 'ood1'
const BROWSER_ITEM = 'chromium'
const DOWNLOAD_HINT = 'Settings › Extension tools, or with `buckyos ext-tools download ext-core chromium`'

const browsers = new Map<string, MockBrowser>()
const sockets = new Set<MockSocket>()
const log: MockLogEntry[] = []
const rpcLog: Array<{ agentId: string; method: string; params: Record<string, unknown> }> = []
const uploads: MockUpload[] = []
let nextId = 1
let frameCount = 0

function browserOf(agentId: string): MockBrowser {
  let browser = browsers.get(agentId)
  if (!browser) {
    browser = { enabled: true, available: true, unavailable: null, running: false, signedIn: false, defaultSessionId: null, groups: new Map(), downloads: new Map() }
    browsers.set(agentId, browser)
  }
  return browser
}

function groupOf(agentId: string, groupId: string): MockGroup {
  const browser = browserOf(agentId)
  let group = browser.groups.get(groupId)
  if (!group) {
    group = { id: groupId, tabs: [], activeTabId: null, userHold: false, lease: null, handover: null, released: null, waitingAnswer: false, requests: new Map() }
    browser.groups.set(groupId, group)
  }
  return group
}

const activeTab = (group: MockGroup) => group.tabs.find((tab) => tab.id === group.activeTabId) ?? null
const controllerOf = (group: MockGroup) => (group.lease && !group.userHold ? 'agent' : 'user')

function agentStateOf(group: MockGroup): AgentState {
  if (group.handover && group.userHold) return 'waiting_takeover'
  if (group.lease) return 'working'
  if (group.waitingAnswer) return 'waiting_answer'
  return group.released?.state ?? 'idle'
}

function wireTab(group: MockGroup, tab: MockTab): BrowserTab {
  return { tab_id: tab.id, url: tab.page.url, title: tab.page.title, loading: tab.loading, active: tab.id === group.activeTabId }
}

function wireGroup(group: MockGroup): BrowserGroup {
  return {
    group_id: group.id,
    controller: controllerOf(group),
    user_hold: group.userHold,
    agent_state: agentStateOf(group),
    note: group.handover?.note ?? group.released?.note ?? null,
    handover: group.handover,
    lease: group.lease,
    tabs: group.tabs.map((tab) => wireTab(group, tab)),
  }
}

/** Whether the node has the browser: ext-core is there and its `chromium` is downloaded (a download of it again leaves the one in use). */
const browserThere = (browser: MockBrowser) => browser.available && mockOnlineInPlace(MOCK_NODE, BROWSER_ITEM)

/** §2.1: the first of `disabled`, `no_package`, `not_downloaded` that holds, with the service's sentence for it. */
function unavailability(browser: MockBrowser): { code: UnavailableCode; text: string } | null {
  if (!browser.enabled) return { code: 'disabled', text: browser.unavailable ?? 'the browser is switched off for this agent ([loader.browser] enabled = false)' }
  if (!browser.available) return { code: 'no_package', text: browser.unavailable ?? 'no `chromium` on this node (it comes with the ext-core extension tools)' }
  if (browserThere(browser)) return null
  const download = mockOnlineStatus(MOCK_NODE, BROWSER_ITEM)
  const text = download?.state === 'downloading'
    ? `the browser is being downloaded to this node${download.size_bytes ? ` (${Math.floor((download.downloaded_bytes * 100) / download.size_bytes)}% of ${Math.round(download.size_bytes / 1024 ** 2)} MiB)` : ''}`
    : download?.state === 'failed'
      ? `the browser's download to this node failed: ${download.error ?? 'no reason recorded'}. Start it again in ${DOWNLOAD_HINT}`
      : `the browser is not downloaded on this node yet (the online item \`chromium\` of ext-core): download it in ${DOWNLOAD_HINT}`
  return { code: 'not_downloaded', text }
}

function wireStatus(agentId: string): BrowserStatus {
  const browser = browserOf(agentId)
  const why = unavailability(browser)
  const groups = [groupOf(agentId, DEFAULT_GROUP)]
  return {
    enabled: browser.enabled,
    available: browserThere(browser),
    running: !why && browser.running,
    unavailable: why?.text ?? null,
    unavailable_code: why?.code ?? null,
    // The mock page is a headless browser's: it is made the device that looks and asks the screen for its lists and dialogs.
    display: 'headless',
    device_scale_factor: null,
    version: why?.code === 'disabled' || why?.code === 'no_package' ? null : { browser: '159.0.8154.6', playwright: '1.65.1' },
    default_session_id: browser.defaultSessionId,
    groups: groups.map(wireGroup),
  }
}

/** §3.2: 1–2, two decimals; `fallback` when the message names none. */
function clampRatio(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? Math.round(Math.max(1, Math.min(2, value)) * 100) / 100 : fallback
}

function clampViewport(viewport: Viewport, pixelRatio: number): Viewport {
  const clamp = (value: number) => Math.max(VIEWPORT_MIN, Math.min(VIEWPORT_MAX, Math.round(Number(value) || VIEWPORT_MIN)))
  return { width: clamp(viewport.width), height: clamp(viewport.height), mobile: viewport.mobile === true, pixel_ratio: clampRatio(viewport.pixel_ratio, pixelRatio) }
}

/** A slow link between the service and the App (`window.__agentBrowserMock.network`): a frame's JPEG arrives after its
 * `frame_meta` as late as its bytes take at this rate; null: at once. */
const network: { bytesPerSecond: number | null } = { bytesPerSecond: null }

class MockSocket implements ScreenSocket {
  binaryType: BinaryType = 'arraybuffer'
  onopen: ((event: Event) => void) | null = null
  onmessage: ((event: MessageEvent) => void) | null = null
  onclose: ((event: CloseEvent) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  readonly agentId: string
  groupId: string | null = null
  viewport: Viewport = { width: 412, height: 915, mobile: true, pixel_ratio: 1 }
  jpegQuality = 60
  closed = false
  seq = 0
  unacked: number | null = null
  dirty = false
  rendering = false
  errorAt = 0
  touch: { x: number; y: number; moved: number } | null = null
  private outbox: Array<string | ArrayBuffer> = []
  private flushing = false

  constructor(agentId: string) {
    this.agentId = agentId
    sockets.add(this)
    window.setTimeout(() => { if (!this.closed) this.onopen?.(new Event('open')) }, LATENCY_MS)
  }

  send(data: string) {
    if (this.closed) throw Error('socket is closed')
    const message = JSON.parse(data) as ScreenUpstream
    window.setTimeout(() => { if (!this.closed) receive(this, message) }, LATENCY_MS)
  }

  close() {
    this.closed = true
    sockets.delete(this)
  }

  deliver(message: ScreenDownstream | ArrayBuffer) {
    if (this.closed) return
    this.outbox.push(message instanceof ArrayBuffer ? message : JSON.stringify(message))
    if (this.flushing) return
    this.flushing = true
    window.setTimeout(() => {
      this.flushing = false
      for (const data of this.outbox.splice(0)) if (!this.closed) this.onmessage?.(new MessageEvent('message', { data }))
    }, LATENCY_MS)
  }

  /** The service's side closes the connection. */
  drop() {
    if (this.closed) return
    this.close()
    this.onclose?.(new CloseEvent('close'))
  }
}

function screenState(socket: MockSocket, group: MockGroup): ScreenState {
  const tab = activeTab(group)
  return {
    group: wireGroup(group),
    active_tab_id: tab?.id ?? null,
    url: tab && tab.page.url !== 'about:blank' ? tab.page.url : '',
    title: tab?.page.title ?? '',
    loading: tab?.loading ?? false,
    can_go_back: !!tab && tab.cursor > 0,
    can_go_forward: !!tab && tab.cursor < tab.history.length - 1,
    viewport: socket.viewport,
    running: browserOf(socket.agentId).running,
  }
}

function watchers(agentId: string, groupId: string): MockSocket[] {
  return [...sockets].filter((socket) => socket.agentId === agentId && socket.groupId === groupId)
}

function broadcastState(agentId: string, groupId: string) {
  const group = groupOf(agentId, groupId)
  for (const socket of watchers(agentId, groupId)) socket.deliver({ type: 'state', ...screenState(socket, group) })
}

async function pushFrame(socket: MockSocket) {
  if (socket.closed || !socket.groupId) return
  const tab = activeTab(groupOf(socket.agentId, socket.groupId))
  if (!tab) return
  if (socket.unacked !== null || socket.rendering) { socket.dirty = true; return }
  socket.rendering = true
  socket.dirty = false
  const viewport = socket.viewport
  let data: ArrayBuffer
  try {
    data = await renderMockPage(tab.page, { viewport, signedIn: browserOf(socket.agentId).signedIn, frame: ++frameCount, quality: socket.jpegQuality })
  } catch {
    socket.rendering = false
    return
  }
  socket.rendering = false
  if (socket.closed) return
  socket.unacked = ++socket.seq
  socket.deliver({ type: 'frame_meta', seq: socket.seq, tab_id: tab.id, width: viewport.width, height: viewport.height })
  if (network.bytesPerSecond) window.setTimeout(() => socket.deliver(data), (data.byteLength / network.bytesPerSecond) * 1000)
  else socket.deliver(data)
}

function repaint(agentId: string, groupId: string) {
  for (const socket of watchers(agentId, groupId)) void pushFrame(socket)
}

function changed(agentId: string, groupId: string) {
  broadcastState(agentId, groupId)
  repaint(agentId, groupId)
}

function emitRequest(agentId: string, groupId: string, request: ScreenRequest, open: boolean) {
  if (open) groupOf(agentId, groupId).requests.set(request.request_id, request)
  for (const socket of watchers(agentId, groupId)) socket.deliver({ type: 'request', ...request })
}

function closeRequest(agentId: string, groupId: string, requestId: string) {
  if (!groupOf(agentId, groupId).requests.delete(requestId)) return
  for (const socket of watchers(agentId, groupId)) socket.deliver({ type: 'request_closed', request_id: requestId })
}

/** `focus` (§3.3): whether the current tab's focus is on its text field, and where that is in each watcher's viewport. */
function emitFocus(agentId: string, groupId: string) {
  const tab = activeTab(groupOf(agentId, groupId))
  if (!tab) return
  for (const socket of watchers(agentId, groupId)) {
    const rect = mockPageRects(socket.viewport.width).input
    socket.deliver(tab.page.focused
      ? { type: 'focus', tab_id: tab.id, editable: true, input_type: 'text', rect: { ...rect, y: rect.y - tab.page.scrollY } }
      : { type: 'focus', tab_id: tab.id, editable: false })
  }
}

function openTab(agentId: string, groupId: string, url: string): MockTab {
  const browser = browserOf(agentId)
  const total = [...browser.groups.values()].reduce((sum, group) => sum + group.tabs.length, 0)
  if (total >= MAX_TABS) throw new MockError('too_many_tabs')
  const group = groupOf(agentId, groupId)
  const tab: MockTab = { id: `TAB${nextId++}`, history: [url], cursor: 0, loading: false, page: newMockPage(url) }
  group.tabs.push(tab)
  group.activeTabId = tab.id
  return tab
}

function load(agentId: string, groupId: string, tab: MockTab, url: string) {
  tab.page = newMockPage(url)
  tab.loading = true
  changed(agentId, groupId)
  emitFocus(agentId, groupId)
  window.setTimeout(() => {
    if (tab.page.url !== url) return
    tab.loading = false
    changed(agentId, groupId)
  }, NAV_MS)
}

function click(socket: MockSocket, group: MockGroup, tab: MockTab, x: number, y: number) {
  const agentId = socket.agentId
  const page = tab.page
  const element: MockElement | null = mockElementAt(page, socket.viewport.width, x, y)
  const focused = page.focused
  page.focused = element === 'input'
  if (page.focused !== focused) emitFocus(agentId, group.id)
  const base = { request_id: `r${nextId++}`, tab_id: tab.id }
  switch (element) {
    case 'select':
      emitRequest(agentId, group.id, { ...base, kind: 'select', multiple: page.multiple, options: mockSelectOptions(page), rect: { ...mockPageRects(socket.viewport.width).select, y: mockPageRects(socket.viewport.width).select.y - page.scrollY } }, true)
      break
    case 'alert':
      emitRequest(agentId, group.id, { ...base, kind: 'dialog', dialog_type: 'alert', message: 'This is an alert from the page.' }, true)
      break
    case 'confirm':
      emitRequest(agentId, group.id, { ...base, kind: 'dialog', dialog_type: 'confirm', message: 'Do you confirm?' }, true)
      break
    case 'prompt':
      emitRequest(agentId, group.id, { ...base, kind: 'dialog', dialog_type: 'prompt', message: 'What is your name?', default_prompt: 'Alice' }, true)
      break
    case 'upload':
      emitRequest(agentId, group.id, { ...base, kind: 'file_chooser', multiple: true, accept: ['.pdf', '.txt', 'image/*'] }, true)
      break
    case 'download': {
      const downloadId = `d${nextId++}`
      browserOf(agentId).downloads.set(downloadId, { name: 'report.txt', size: 2048 })
      emitRequest(agentId, group.id, { ...base, kind: 'download', download_id: downloadId, name: 'report.txt', size: 2048, state: 'completed' }, false)
      break
    }
    case 'location':
      // A permission is the browser's to ask for: the service tells the App nothing.
      break
    case 'newTab':
      try { openTab(agentId, group.id, `${page.url.split('#')[0]}#popup`); emitFocus(agentId, group.id) } catch { /* the tab limit */ }
      break
    case 'signIn':
      browserOf(agentId).signedIn = true
      break
  }
}

function applyInput(socket: MockSocket, group: MockGroup, tab: MockTab, input: ScreenInput) {
  const page = tab.page
  const maxScroll = Math.max(0, MOCK_PAGE_HEIGHT - socket.viewport.height)
  const scrollBy = (delta: number) => { page.scrollY = Math.max(0, Math.min(maxScroll, page.scrollY + delta)) }
  switch (input.kind) {
    case 'mouse':
      page.pointer = { x: input.x, y: input.y }
      if (input.action === 'up' && input.button === 'left') click(socket, group, tab, input.x, input.y)
      break
    case 'wheel':
      scrollBy(input.delta_y)
      break
    case 'touch': {
      const point = input.points[0]
      if (input.phase === 'start' && point) socket.touch = { x: point.x, y: point.y, moved: 0 }
      else if (input.phase === 'move' && point && socket.touch) {
        const dy = point.y - socket.touch.y
        socket.touch = { x: point.x, y: point.y, moved: socket.touch.moved + Math.abs(dy) + Math.abs(point.x - socket.touch.x) }
        scrollBy(-dy)
      } else if (input.phase === 'end' && socket.touch && input.points.length === 0) {
        const { x, y, moved } = socket.touch
        socket.touch = null
        page.pointer = { x, y }
        if (moved < 8) click(socket, group, tab, x, y)
      } else if (input.phase === 'cancel') socket.touch = null
      break
    }
    case 'key':
      if (input.action !== 'down' || !page.focused) break
      if (input.key === 'Backspace') page.text = [...page.text].slice(0, -1).join('')
      else if (input.key === 'Enter') page.dialog = `submitted:${page.text}`
      else if (input.text) page.text += input.text
      break
    case 'text':
      page.composition = ''
      if (page.focused) page.text += input.text
      break
    case 'composition':
      if (page.focused) page.composition = input.text
      break
  }
}

function applyRespond(agentId: string, group: MockGroup, requestId: string, result: RequestResult) {
  const request = group.requests.get(requestId)
  if (!request) return
  const page = group.tabs.find((tab) => tab.id === request.tab_id)?.page
  if (page && request.kind === 'select' && 'selected' in result) page.selected = result.selected
  if (page && request.kind === 'dialog' && 'accept' in result) page.dialog = `${request.dialog_type}:${result.accept ? 'ok' : 'cancel'}${result.prompt_text !== undefined ? `:${result.prompt_text}` : ''}`
  closeRequest(agentId, group.id, requestId)
}

function receive(socket: MockSocket, message: ScreenUpstream) {
  const agentId = socket.agentId
  if (message.type === 'hello') {
    const browser = browserOf(agentId)
    const refuse = (code: string, why?: { code: UnavailableCode; text: string }) => {
      socket.deliver({ type: 'error', code, message: why?.text ?? code, ...(why ? { unavailable_code: why.code } : {}) })
      window.setTimeout(() => socket.drop(), LATENCY_MS * 3)
    }
    if (message.token !== TOKEN) { refuse('unauthorized'); return }
    if (!message.group_id || !message.viewport) { refuse('bad_hello'); return }
    const why = unavailability(browser)
    if (why) { refuse('browser_unavailable', why); return }
    browser.running = true
    socket.groupId = message.group_id
    socket.viewport = clampViewport(message.viewport, 1)
    const group = groupOf(agentId, message.group_id)
    log.push({ agentId, groupId: group.id, dropped: false, message })
    socket.deliver({ type: 'ready', state: screenState(socket, group) })
    for (const request of group.requests.values()) socket.deliver({ type: 'request', ...request })
    void pushFrame(socket)
    return
  }
  if (!socket.groupId) return
  const group = groupOf(agentId, socket.groupId)
  if (message.type === 'ack') {
    if (message.seq === socket.unacked) socket.unacked = null
    if (socket.dirty) void pushFrame(socket)
    return
  }
  if (message.type === 'ping') { socket.deliver({ type: 'pong' }); return }
  const guarded = message.type === 'input' || message.type === 'nav' || message.type === 'tab'
  const dropped = guarded && controllerOf(group) === 'agent'
  log.push({ agentId, groupId: group.id, dropped, message })
  if (dropped) {
    const now = Date.now()
    if (now - socket.errorAt >= 1000) {
      socket.errorAt = now
      socket.deliver({ type: 'error', code: 'agent_in_control', message: 'the Agent is operating this group' })
    }
    return
  }
  const tab = activeTab(group)
  switch (message.type) {
    case 'input':
      if (tab) applyInput(socket, group, tab, message)
      if ((message.kind === 'mouse' || message.kind === 'touch') && (message.kind === 'mouse' ? message.action : message.phase) === 'move') repaint(agentId, group.id)
      else changed(agentId, group.id)
      break
    case 'nav':
      if (!tab) break
      if (message.action === 'url') {
        tab.history = [...tab.history.slice(0, tab.cursor + 1), message.url]
        tab.cursor = tab.history.length - 1
        load(agentId, group.id, tab, message.url)
      } else if (message.action === 'back' && tab.cursor > 0) load(agentId, group.id, tab, tab.history[--tab.cursor])
      else if (message.action === 'forward' && tab.cursor < tab.history.length - 1) load(agentId, group.id, tab, tab.history[++tab.cursor])
      else if (message.action === 'reload') load(agentId, group.id, tab, tab.page.url)
      else if (message.action === 'stop') { tab.loading = false; changed(agentId, group.id) }
      break
    case 'tab':
      if (message.action === 'open') {
        try {
          const opened = openTab(agentId, group.id, message.url ?? 'about:blank')
          load(agentId, group.id, opened, opened.page.url)
        } catch (error) {
          socket.deliver({ type: 'error', code: (error as MockError).code, message: 'too many tabs' })
        }
      } else if (message.action === 'activate') {
        if (group.tabs.some((item) => item.id === message.tab_id)) group.activeTabId = message.tab_id
        changed(agentId, group.id)
        emitFocus(agentId, group.id)
      } else {
        const index = group.tabs.findIndex((item) => item.id === message.tab_id)
        if (index < 0) { socket.deliver({ type: 'error', code: 'no_such_tab', message: message.tab_id }); break }
        group.tabs.splice(index, 1)
        for (const request of [...group.requests.values()]) if (request.tab_id === message.tab_id) closeRequest(agentId, group.id, request.request_id)
        if (group.activeTabId === message.tab_id) group.activeTabId = (group.tabs[index] ?? group.tabs[index - 1])?.id ?? null
        changed(agentId, group.id)
        emitFocus(agentId, group.id)
      }
      break
    case 'viewport':
      socket.viewport = clampViewport(message, socket.viewport.pixel_ratio ?? 1)
      socket.deliver({ type: 'state', ...screenState(socket, group) })
      void pushFrame(socket)
      break
    case 'respond':
      applyRespond(agentId, group, message.request_id, message.result)
      changed(agentId, group.id)
      break
    case 'quality':
      if (typeof message.jpeg_quality === 'number') socket.jpegQuality = Math.round(Math.max(10, Math.min(95, message.jpeg_quality)))
      socket.viewport = { ...socket.viewport, pixel_ratio: clampRatio(message.pixel_ratio, socket.viewport.pixel_ratio ?? 1) }
      socket.deliver({ type: 'state', ...screenState(socket, group) })
      void pushFrame(socket)
      break
  }
}

window.setInterval(() => {
  for (const socket of sockets) void pushFrame(socket)
}, 1000)

function requireUsable(agentId: string) {
  const browser = browserOf(agentId)
  const why = unavailability(browser)
  if (why) throw new MockError('browser_unavailable', why.text, why.code)
  return browser
}

/** `browser.page_hint` (§7.2): the mock page has no content to count, so the address stands for it — a sign-in or a form is a task, a story is reading. */
function pageHint(agentId: string): PageHint {
  const browser = browserOf(agentId)
  const tab = browser.running ? activeTab(groupOf(agentId, DEFAULT_GROUP)) : null
  if (!tab || !/^https?:/.test(tab.page.url)) return { hint: 'unknown', url: null, title: null }
  const url = new URL(tab.page.url)
  const hint = mockSeed.hints?.[url.hostname] ?? (/login|signin|form|apply/.test(url.pathname) ? 'task' : /^news\.|\/(story|articles?)\//.test(url.hostname + url.pathname) ? 'reading' : 'unknown')
  return { hint, url: tab.page.url, title: tab.page.title }
}

function chooserOf(agentId: string, requestId: string): { group: MockGroup; request: Extract<ScreenRequest, { kind: 'file_chooser' }> } {
  for (const group of browserOf(agentId).groups.values()) {
    const request = group.requests.get(requestId)
    if (request?.kind === 'file_chooser') return { group, request }
  }
  throw new MockError('no_such_request')
}

function acceptUpload(upload: MockUpload) {
  const { group, request } = chooserOf(upload.agentId, upload.requestId)
  uploads.push(upload)
  const page = group.tabs.find((tab) => tab.id === request.tab_id)?.page
  if (page) page.files = [...page.files, upload.name]
  if (upload.done) closeRequest(upload.agentId, group.id, upload.requestId)
  changed(upload.agentId, group.id)
}

function call(agentId: string, method: string, params: Record<string, unknown>): unknown {
  rpcLog.push({ agentId, method, params })
  const groupId = typeof params.group_id === 'string' ? params.group_id : ''
  switch (method) {
    case 'browser.status':
      if (groupId && groupId !== DEFAULT_GROUP) throw new MockError('no_such_group')
      return wireStatus(agentId)
    case 'browser.ensure':
      requireUsable(agentId).running = true
      return wireStatus(agentId)
    case 'browser.take_over':
    case 'browser.hand_back':
    case 'browser.hand_to_agent': {
      requireUsable(agentId)
      if (groupId !== DEFAULT_GROUP) throw new MockError('no_such_group')
      const group = groupOf(agentId, groupId)
      group.lease = null
      group.released = null
      group.userHold = method === 'browser.take_over'
      if (!group.userHold) group.handover = null
      changed(agentId, groupId)
      return wireGroup(group)
    }
    case 'browser.bind_default': {
      const sessionId = typeof params.session_id === 'string' ? params.session_id : ''
      if (!sessionId || sessionId === DEFAULT_GROUP) throw new MockError('invalid_argument')
      browserOf(agentId).defaultSessionId = sessionId
      return { default_session_id: sessionId }
    }
    case 'browser.upload': {
      const path = String(params.zone_path ?? '')
      acceptUpload({ agentId, requestId: String(params.request_id ?? ''), name: path.slice(path.lastIndexOf('/') + 1), source: 'zone', done: params.done === true })
      return {}
    }
    case 'browser.download_export': {
      const downloadId = String(params.download_id ?? '')
      const download = browserOf(agentId).downloads.get(downloadId)
      if (!download) throw new MockError('no_such_download')
      return { obj_id: `mock-download-${downloadId}`, ...download }
    }
    case 'browser.clear_profile': {
      const browser = requireUsable(agentId)
      browser.signedIn = false
      for (const group of browser.groups.values()) repaint(agentId, group.id)
      return {}
    }
    case 'browser.page_hint':
      return pageHint(agentId)
    case 'browser.reflow_state':
      return mockReflowState(params)
    default: {
      const answer = mockManageCall(agentId, method, params)
      if (answer === undefined) throw new MockError('invalid_argument', `unknown method ${method}`)
      return answer
    }
  }
}

export const mockBackend: BrowserBackend = {
  async agents() {
    return [...(await ownReadyAgents()).map(toBrowserAgent), MOCK_HUB_AGENT]
  },
  async call<T>(agent: BrowserAgent, method: string, params: Record<string, unknown> = {}) {
    await new Promise((resolve) => window.setTimeout(resolve, LATENCY_MS))
    return call(agent.agentId, method, params) as T
  },
  async token() {
    return TOKEN
  },
  openScreen(agent) {
    return new MockSocket(agent.agentId)
  },
  async upload(agent, requestId, file, done) {
    await new Promise((resolve) => window.setTimeout(resolve, LATENCY_MS))
    acceptUpload({ agentId: agent.agentId, requestId, name: file.name, source: 'local', done })
  },
}

/** The Agent's side of the mock and what the service received, for the e2e specs and for trying the App by hand. */
const control = {
  log,
  rpc: rpcLog,
  uploads,
  network,
  rects: mockPageRects,
  reflow: mockReflowControl,
  /** The personal site's data, as whoever maintains it reads and writes it (mock/site.ts). */
  site: { read: mockReflowControl.read, put: mockReflowControl.put },
  /** The jobs of the site's maintenance Goal and the record of browsed pages (mock/manage.ts). */
  manage: mockManageControl,
  page(agentId: string) {
    const tab = activeTab(groupOf(agentId, DEFAULT_GROUP))
    return tab ? { ...tab.page, signedIn: browserOf(agentId).signedIn } : null
  },
  agentAcquire(agentId: string, sessionId = 'mock-work-session') {
    const group = groupOf(agentId, DEFAULT_GROUP)
    if (group.userHold) return false
    group.lease = { session_id: sessionId, expires_at_ms: Date.now() + 60_000 }
    group.released = null
    changed(agentId, DEFAULT_GROUP)
    return true
  },
  agentOpen(agentId: string, url: string) {
    const group = groupOf(agentId, DEFAULT_GROUP)
    if (!group.lease) return false
    const tab = activeTab(group)
    if (tab) {
      tab.history = [...tab.history.slice(0, tab.cursor + 1), url]
      tab.cursor = tab.history.length - 1
      load(agentId, DEFAULT_GROUP, tab, url)
    } else load(agentId, DEFAULT_GROUP, openTab(agentId, DEFAULT_GROUP, url), url)
    return true
  },
  agentRelease(agentId: string, state?: 'done' | 'stuck', note?: string) {
    const group = groupOf(agentId, DEFAULT_GROUP)
    group.lease = null
    group.released = state ? { state, note: note ?? null } : null
    changed(agentId, DEFAULT_GROUP)
  },
  agentRequestHandover(agentId: string, reason: HandoverReason, note?: string) {
    const group = groupOf(agentId, DEFAULT_GROUP)
    group.lease = null
    group.userHold = true
    group.handover = { reason, note: note ?? null, at_ms: Date.now() }
    changed(agentId, DEFAULT_GROUP)
  },
  /** A question the Agent asked in the browser conversation is open (`waiting_answer`), or not any more. */
  agentAsk(agentId: string, waiting: boolean) {
    groupOf(agentId, DEFAULT_GROUP).waitingAnswer = waiting
    changed(agentId, DEFAULT_GROUP)
  },
  /** The page moves its own focus onto its text field, or away from it (a script, not a press). */
  pageFocus(agentId: string, focused: boolean) {
    const tab = activeTab(groupOf(agentId, DEFAULT_GROUP))
    if (!tab) return
    tab.page.focused = focused
    changed(agentId, DEFAULT_GROUP)
    emitFocus(agentId, DEFAULT_GROUP)
  },
  /** `download`: where the node's `chromium` stands (shared by every Agent of the mock); it then moves only by `ext_tools.download`. */
  setAvailability(agentId: string, { download, ...patch }: { enabled?: boolean; available?: boolean; unavailable?: string | null; download?: 'missing' | 'failed' | 'ready' }) {
    Object.assign(browserOf(agentId), patch)
    if (download) setMockOnline(MOCK_NODE, BROWSER_ITEM, download)
  },
  dropConnections(agentId: string) {
    for (const socket of [...sockets]) if (socket.agentId === agentId) socket.drop()
  },
}

export type AgentBrowserMockControl = typeof control

Object.assign(window, { __agentBrowserMock: control })
