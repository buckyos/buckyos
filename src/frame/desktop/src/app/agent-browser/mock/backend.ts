/* The Agent Service of the mock runtime (`VITE_CP_USE_MOCK`): kRPC `browser.*`, the screen socket and the upload,
 * speaking the wire format of the protocol document, so the App above api/backend.ts runs the same code as on a
 * Zone. A group has tabs showing the page of mock/page.ts; the Agent's side (lease, release, handover request) is
 * driven from `window.__agentBrowserMock`, which the e2e specs also read the received messages from. */

import { getMockEntityDid } from '../../messagehub/mock/data'
import { ownReadyAgents, toBrowserAgent, type BrowserAgent, type BrowserBackend } from '../api/backend'
import type { ScreenSocket } from '../api/channel'
import { DEFAULT_GROUP, VIEWPORT_MAX, VIEWPORT_MIN } from '../api/types'
import type { AgentState, BrowserGroup, BrowserStatus, BrowserTab, HandoverReason, RequestResult, ScreenDownstream, ScreenInput, ScreenRequest, ScreenState, ScreenUpstream, Viewport } from '../api/types'
import { MOCK_PAGE_HEIGHT, mockElementAt, mockPageRects, mockSelectOptions, newMockPage, renderMockPage, type MockElement, type MockPage } from './page'
import { mockReflowCall, mockReflowControl } from './reflow'

const TOKEN = 'mock-session-token'
const MAX_TABS = 12
const LATENCY_MS = 4
const NAV_MS = 120

/** The MessageHub mock's own Agent: the one its Agent panel belongs to. `agent.list` of the mock control panel does not list it. */
export const MOCK_HUB_AGENT: BrowserAgent = { agentId: 'codeassistant', agentDid: getMockEntityDid('agent-coder'), name: 'CodeAssistant' }

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
  constructor(code: string, message = code) {
    super(`RPC call error: Failed due to reason: ${JSON.stringify({ kind: 'browser', code, message })}`)
    this.code = code
  }
}

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

function wireStatus(agentId: string, groupId?: string): BrowserStatus {
  const browser = browserOf(agentId)
  const usable = browser.enabled && browser.available
  const groups = groupId ? [groupOf(agentId, groupId)] : [groupOf(agentId, DEFAULT_GROUP), ...[...browser.groups.values()].filter((group) => group.id !== DEFAULT_GROUP && (group.tabs.length > 0 || !!group.lease || group.userHold || agentStateOf(group) !== 'idle'))]
  return {
    enabled: browser.enabled,
    available: browser.available,
    running: usable && browser.running,
    unavailable: usable ? null : browser.unavailable ?? (browser.enabled ? 'chromium was not found on this node' : 'the browser is turned off for this Agent'),
    version: browser.available ? { browser: '159.0.8154.6', playwright: '1.65.1' } : null,
    default_session_id: browser.defaultSessionId,
    groups: groups.map(wireGroup),
  }
}

function clampViewport(viewport: Viewport): Viewport {
  const clamp = (value: number) => Math.max(VIEWPORT_MIN, Math.min(VIEWPORT_MAX, Math.round(Number(value) || VIEWPORT_MIN)))
  return { width: clamp(viewport.width), height: clamp(viewport.height), mobile: viewport.mobile === true }
}

class MockSocket implements ScreenSocket {
  binaryType: BinaryType = 'arraybuffer'
  onopen: ((event: Event) => void) | null = null
  onmessage: ((event: MessageEvent) => void) | null = null
  onclose: ((event: CloseEvent) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  readonly agentId: string
  groupId: string | null = null
  viewport: Viewport = { width: 412, height: 915, mobile: true }
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
    data = await renderMockPage(tab.page, { viewport, signedIn: browserOf(socket.agentId).signedIn, frame: ++frameCount })
  } catch {
    socket.rendering = false
    return
  }
  socket.rendering = false
  if (socket.closed) return
  socket.unacked = ++socket.seq
  socket.deliver({ type: 'frame_meta', seq: socket.seq, tab_id: tab.id, width: viewport.width, height: viewport.height })
  socket.deliver(data)
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
    case 'location': {
      let origin = page.url
      try { origin = new URL(page.url).origin } catch { /* shown as it is */ }
      emitRequest(agentId, group.id, { ...base, kind: 'permission', permission: 'geolocation', origin }, false)
      break
    }
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
    const refuse = (code: string) => { socket.deliver({ type: 'error', code, message: code }); window.setTimeout(() => socket.drop(), LATENCY_MS * 3) }
    if (message.token !== TOKEN) { refuse('unauthorized'); return }
    if (!message.group_id || !message.viewport) { refuse('bad_hello'); return }
    if (!browser.enabled || !browser.available) { refuse('browser_unavailable'); return }
    browser.running = true
    socket.groupId = message.group_id
    socket.viewport = clampViewport(message.viewport)
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
      socket.viewport = clampViewport(message)
      socket.deliver({ type: 'state', ...screenState(socket, group) })
      void pushFrame(socket)
      break
    case 'respond':
      applyRespond(agentId, group, message.request_id, message.result)
      changed(agentId, group.id)
      break
    case 'quality':
      break
  }
}

window.setInterval(() => {
  for (const socket of sockets) void pushFrame(socket)
}, 1000)

function requireUsable(agentId: string) {
  const browser = browserOf(agentId)
  if (!browser.enabled || !browser.available) throw new MockError('browser_unavailable')
  return browser
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
      return wireStatus(agentId, groupId || undefined)
    case 'browser.ensure':
      requireUsable(agentId).running = true
      return wireStatus(agentId)
    case 'browser.take_over':
    case 'browser.hand_back':
    case 'browser.hand_to_agent': {
      requireUsable(agentId)
      if (!groupId) throw new MockError('no_such_group')
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
      if (!sessionId) throw new MockError('invalid_argument')
      const browser = browserOf(agentId)
      const named = browser.groups.get(sessionId)
      browser.defaultSessionId = sessionId
      if (named && sessionId !== DEFAULT_GROUP) {
        const target = groupOf(agentId, DEFAULT_GROUP)
        target.tabs.push(...named.tabs)
        target.activeTabId ??= named.activeTabId
        browser.groups.delete(sessionId)
        changed(agentId, DEFAULT_GROUP)
      }
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
    case 'browser.reflow_state':
    case 'browser.reflow_refresh':
    case 'browser.reflow_action':
      return mockReflowCall(method, params, browserOf(agentId))
    default:
      throw new MockError('invalid_argument', `unknown method ${method}`)
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
  rects: mockPageRects,
  reflow: mockReflowControl,
  page(agentId: string, groupId: string) {
    const group = groupOf(agentId, groupId)
    const tab = activeTab(group)
    return tab ? { ...tab.page, signedIn: browserOf(agentId).signedIn } : null
  },
  agentAcquire(agentId: string, groupId: string, sessionId = 'mock-work-session') {
    const group = groupOf(agentId, groupId)
    if (group.userHold) return false
    group.lease = { session_id: sessionId, expires_at_ms: Date.now() + 60_000 }
    group.released = null
    changed(agentId, groupId)
    return true
  },
  agentOpen(agentId: string, groupId: string, url: string) {
    const group = groupOf(agentId, groupId)
    if (!group.lease) return false
    const tab = activeTab(group)
    if (tab) {
      tab.history = [...tab.history.slice(0, tab.cursor + 1), url]
      tab.cursor = tab.history.length - 1
      load(agentId, groupId, tab, url)
    } else load(agentId, groupId, openTab(agentId, groupId, url), url)
    return true
  },
  agentRelease(agentId: string, groupId: string, state?: 'done' | 'stuck', note?: string) {
    const group = groupOf(agentId, groupId)
    group.lease = null
    group.released = state ? { state, note: note ?? null } : null
    changed(agentId, groupId)
  },
  agentRequestHandover(agentId: string, groupId: string, reason: HandoverReason, note?: string) {
    const group = groupOf(agentId, groupId)
    group.lease = null
    group.userHold = true
    group.handover = { reason, note: note ?? null, at_ms: Date.now() }
    changed(agentId, groupId)
  },
  agentAsk(agentId: string, groupId: string, waiting: boolean) {
    groupOf(agentId, groupId).waitingAnswer = waiting
    changed(agentId, groupId)
  },
  /** The page moves its own focus onto its text field, or away from it (a script, not a press). */
  pageFocus(agentId: string, groupId: string, focused: boolean) {
    const tab = activeTab(groupOf(agentId, groupId))
    if (!tab) return
    tab.page.focused = focused
    changed(agentId, groupId)
    emitFocus(agentId, groupId)
  },
  setAvailability(agentId: string, patch: { enabled?: boolean; available?: boolean; unavailable?: string | null }) {
    Object.assign(browserOf(agentId), patch)
  },
  dropConnections(agentId: string) {
    for (const socket of [...sockets]) if (socket.agentId === agentId) socket.drop()
  },
}

export type AgentBrowserMockControl = typeof control

Object.assign(window, { __agentBrowserMock: control })
