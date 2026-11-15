/* The screen channel (Agent Browser Protocol §3): one WebSocket per group. It says `hello` with the user's token, the
 * group and the viewport, hands every frame to the view (which answers `ack` once it is painted), keeps the
 * connection alive with `ping` while idle, and reconnects with the latest viewport when the socket is lost. */

import type { FrameMeta, ScreenDownstream, ScreenFocus, ScreenFrame, ScreenRequest, ScreenState, ScreenUpstream, Viewport } from './types'

/** A WebSocket as the channel uses it; the mock Agent Service hands out the same shape. */
export interface ScreenSocket {
  binaryType: BinaryType
  onopen: ((event: Event) => void) | null
  onmessage: ((event: MessageEvent) => void) | null
  onclose: ((event: CloseEvent) => void) | null
  onerror: ((event: Event) => void) | null
  send(data: string): void
  close(): void
}

export type ChannelStatus = 'connecting' | 'open' | 'reconnecting' | 'failed' | 'closed'
export interface ChannelFailure { code: string; message: string }

export interface ScreenChannelOptions {
  groupId: string
  viewport: Viewport
  /** Before every connection: whatever must hold first (`browser.ensure`), then the token of the `hello`. */
  prepare: () => Promise<string>
  open: () => ScreenSocket
  onStatus: (status: ChannelStatus, failure: ChannelFailure | null, attempts: number) => void
  onState: (state: ScreenState) => void
  onFrame: (frame: ScreenFrame) => void
  onRequest: (request: ScreenRequest) => void
  onRequestClosed: (requestId: string) => void
  onFocus: (focus: ScreenFocus) => void
  onError: (failure: ChannelFailure) => void
}

const PING_MS = 10_000
const SILENCE_MS = 30_000
/** Handshake errors a new attempt cannot fix. */
const FATAL = new Set(['unauthorized', 'bad_hello', 'browser_unavailable'])

function body<T extends { type: string }>(message: T): Omit<T, 'type'> {
  const copy: Partial<T> = { ...message }
  delete copy.type
  return copy as Omit<T, 'type'>
}

export class ScreenChannel {
  readonly groupId: string
  status: ChannelStatus = 'connecting'
  failure: ChannelFailure | null = null
  /** The last frame received: a view mounted later paints it first. */
  lastFrame: ScreenFrame | null = null
  private readonly options: ScreenChannelOptions
  private viewport: Viewport
  private socket: ScreenSocket | null = null
  private ready = false
  private closed = false
  private attempts = 0
  private retryTimer = 0
  private pingTimer = 0
  private sentAt = 0
  private receivedAt = 0
  private pendingMeta: FrameMeta | null = null
  private generation = 0

  constructor(options: ScreenChannelOptions) {
    this.options = options
    this.groupId = options.groupId
    this.viewport = options.viewport
  }

  start() {
    window.addEventListener('online', this.wake)
    document.addEventListener('visibilitychange', this.wake)
    void this.connect()
  }

  close() {
    if (this.closed) return
    this.closed = true
    window.removeEventListener('online', this.wake)
    document.removeEventListener('visibilitychange', this.wake)
    window.clearTimeout(this.retryTimer)
    window.clearInterval(this.pingTimer)
    this.dropSocket()
    this.setStatus('closed', null)
  }

  /** Whether the message went out: nothing is queued while the channel is not open. */
  send(message: ScreenUpstream): boolean {
    const socket = this.socket
    if (!socket || !this.ready) return false
    try {
      socket.send(JSON.stringify(message))
    } catch {
      return false
    }
    this.sentAt = Date.now()
    return true
  }

  ack(seq: number) {
    this.send({ type: 'ack', seq })
  }

  setViewport(viewport: Viewport) {
    const current = this.viewport
    if (current.width === viewport.width && current.height === viewport.height && current.mobile === viewport.mobile) return
    this.viewport = viewport
    this.send({ type: 'viewport', ...viewport })
  }

  /** A failed channel tries again (the user asked, or the reason may be gone). */
  retry() {
    if (this.closed || this.status !== 'failed') return
    this.attempts = 0
    void this.connect()
  }

  private readonly wake = () => {
    if (this.closed || document.visibilityState === 'hidden' || this.status !== 'reconnecting') return
    window.clearTimeout(this.retryTimer)
    void this.connect()
  }

  private setStatus(status: ChannelStatus, failure: ChannelFailure | null) {
    this.status = status
    this.failure = failure
    this.options.onStatus(status, failure, this.attempts)
  }

  private dropSocket() {
    const socket = this.socket
    this.socket = null
    this.ready = false
    this.pendingMeta = null
    if (!socket) return
    socket.onopen = socket.onmessage = socket.onclose = socket.onerror = null
    try { socket.close() } catch { /* already closed */ }
  }

  private async connect() {
    if (this.closed) return
    const generation = ++this.generation
    this.dropSocket()
    this.setStatus(this.attempts > 0 ? 'reconnecting' : 'connecting', null)
    let token: string
    try {
      token = await this.options.prepare()
    } catch (error) {
      if (this.closed || generation !== this.generation) return
      const code = (error as { code?: unknown } | null)?.code
      if (typeof code === 'string' && FATAL.has(code)) this.fail({ code, message: error instanceof Error ? error.message : String(error) })
      else this.lost()
      return
    }
    if (this.closed || generation !== this.generation) return
    let socket: ScreenSocket
    try {
      socket = this.options.open()
    } catch {
      this.lost()
      return
    }
    socket.binaryType = 'arraybuffer'
    this.socket = socket
    socket.onopen = () => {
      try {
        socket.send(JSON.stringify({ type: 'hello', token, group_id: this.groupId, viewport: this.viewport } satisfies ScreenUpstream))
        this.sentAt = Date.now()
      } catch {
        this.lost()
      }
    }
    socket.onmessage = (event) => this.receive(socket, event.data as unknown)
    socket.onclose = () => { if (this.socket === socket) this.lost() }
    socket.onerror = () => { /* a close follows */ }
  }

  private receive(socket: ScreenSocket, data: unknown) {
    if (this.socket !== socket) return
    this.receivedAt = Date.now()
    if (typeof data !== 'string') {
      const meta = this.pendingMeta
      this.pendingMeta = null
      if (!meta || !(data instanceof ArrayBuffer)) return
      const frame = { ...meta, data }
      this.lastFrame = frame
      this.options.onFrame(frame)
      return
    }
    let message: ScreenDownstream
    try { message = JSON.parse(data) as ScreenDownstream } catch { return }
    switch (message.type) {
      case 'ready':
        this.ready = true
        this.attempts = 0
        this.setStatus('open', null)
        this.options.onState(message.state)
        this.startPing()
        break
      case 'state':
        this.options.onState(body(message))
        break
      case 'frame_meta':
        this.pendingMeta = { seq: message.seq, tab_id: message.tab_id, width: message.width, height: message.height }
        break
      case 'request':
        this.options.onRequest(body(message) as ScreenRequest)
        break
      case 'request_closed':
        this.options.onRequestClosed(message.request_id)
        break
      case 'focus':
        this.options.onFocus(body(message))
        break
      case 'error': {
        const failure = { code: String(message.code ?? 'unknown'), message: String(message.message ?? '') }
        if (this.ready) this.options.onError(failure)
        else if (FATAL.has(failure.code)) this.fail(failure)
        break
      }
      case 'pong':
        break
    }
  }

  private startPing() {
    window.clearInterval(this.pingTimer)
    this.pingTimer = window.setInterval(() => {
      const now = Date.now()
      if (now - this.receivedAt > SILENCE_MS) { this.lost(); return }
      if (now - this.sentAt >= PING_MS - 1000) this.send({ type: 'ping' })
    }, PING_MS / 2)
  }

  private fail(failure: ChannelFailure) {
    window.clearInterval(this.pingTimer)
    this.dropSocket()
    this.setStatus('failed', failure)
  }

  private lost() {
    if (this.closed) return
    window.clearInterval(this.pingTimer)
    this.dropSocket()
    this.attempts += 1
    this.setStatus('reconnecting', null)
    const delay = Math.min(500 * 2 ** Math.min(this.attempts - 1, 5), 10_000)
    window.clearTimeout(this.retryTimer)
    this.retryTimer = window.setTimeout(() => void this.connect(), delay)
  }
}
