/* The screen channel (Agent Browser Protocol §3): one WebSocket per group. It says `hello` with the user's token, the
 * group and the viewport, hands every frame to the view (which answers `ack` once it is painted), keeps the
 * connection alive with `ping` while idle, and reconnects with the latest viewport when the socket is lost. How clear
 * the frames are (`quality`) follows the user's choice, or in "auto" what the frames show of the connection. */

import { QualityGovernor, type QualityChoice, type Tier } from '../screen/quality'
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
  quality: QualityChoice
  /** The tier the frames are asked for: at the start, and whenever it changes. */
  onTier: (tier: Tier) => void
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
const ADJUST_MS = 1000
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
  private adjustTimer = 0
  private sentAt = 0
  private receivedAt = 0
  private pendingMeta: FrameMeta | null = null
  private metaAt = 0
  private generation = 0
  private readonly governor: QualityGovernor
  /** Frames handed to the view and not acknowledged yet: when each arrived, its bytes, how long it took. */
  private readonly arrived = new Map<number, { at: number; bytes: number; transfer: number }>()
  /** Frames up to this one may have been painted at the tier before. */
  private tierSeq = 0
  private lastSeq = 0

  constructor(options: ScreenChannelOptions) {
    this.options = options
    this.groupId = options.groupId
    this.viewport = options.viewport
    this.governor = new QualityGovernor(options.quality, () => window.devicePixelRatio || 1)
  }

  start() {
    window.addEventListener('online', this.wake)
    document.addEventListener('visibilitychange', this.wake)
    this.options.onTier(this.governor.tier())
    void this.connect()
  }

  close() {
    if (this.closed) return
    this.closed = true
    window.removeEventListener('online', this.wake)
    document.removeEventListener('visibilitychange', this.wake)
    window.clearTimeout(this.retryTimer)
    window.clearInterval(this.pingTimer)
    window.clearInterval(this.adjustTimer)
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
    const arrived = this.arrived.get(seq)
    this.arrived.delete(seq)
    // A page in the background paints late: that says nothing of the connection.
    if (!arrived || seq <= this.tierSeq || document.visibilityState === 'hidden') return
    const now = performance.now()
    this.governor.sample(arrived.bytes, arrived.transfer, now - arrived.at, now)
    this.adjust()
  }

  setViewport(viewport: Viewport) {
    const current = this.viewport
    if (current.width === viewport.width && current.height === viewport.height && current.mobile === viewport.mobile) return
    this.viewport = viewport
    this.send({ type: 'viewport', ...viewport, pixel_ratio: this.governor.tier().pixel_ratio })
  }

  /** The user's choice of how clear the frames are. */
  setQuality(choice: QualityChoice) {
    this.governor.choose(choice)
    this.useTier(this.governor.tier())
  }

  private adjust() {
    const tier = this.governor.adjust(performance.now())
    if (tier) this.useTier(tier)
  }

  private useTier(tier: Tier) {
    // The frame on its way when the service hears of the tier is still of the one before.
    this.tierSeq = this.lastSeq + 1
    this.sendQuality(tier)
    this.options.onTier(tier)
  }

  private sendQuality(tier: Tier) {
    this.send({ type: 'quality', max_fps: tier.max_fps, jpeg_quality: tier.jpeg_quality, pixel_ratio: tier.pixel_ratio })
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
    this.arrived.clear()
    this.lastSeq = this.tierSeq = 0
    window.clearInterval(this.adjustTimer)
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
        socket.send(JSON.stringify({ type: 'hello', token, group_id: this.groupId, viewport: { ...this.viewport, pixel_ratio: this.governor.tier().pixel_ratio } } satisfies ScreenUpstream))
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
      const now = performance.now()
      this.arrived.set(meta.seq, { at: now, bytes: data.byteLength, transfer: now - this.metaAt })
      this.lastSeq = meta.seq
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
        this.sendQuality(this.governor.tier())
        this.startPing()
        window.clearInterval(this.adjustTimer)
        this.adjustTimer = window.setInterval(() => this.adjust(), ADJUST_MS)
        break
      case 'state':
        this.options.onState(body(message))
        break
      case 'frame_meta':
        this.pendingMeta = { seq: message.seq, tab_id: message.tab_id, width: message.width, height: message.height }
        this.metaAt = performance.now()
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
