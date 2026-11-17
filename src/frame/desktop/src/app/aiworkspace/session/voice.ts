/* The voice of a session (第四期规划 §7.3.1; 多人发言 TODO §4): one WebRTC connection to the Workspace host per
 * terminal. Everyone receives the host's mix; whoever may speak also sends, on the same connection (a right that comes
 * or goes renegotiates it, §4.1). The microphone goes through a gate (an AudioWorklet, §4.4): in auto mode it opens on
 * a voice above the noise floor and closes after a pause, in manual mode it is open while "talk" is held, in mute the
 * microphone is off. While the gate is closed the track is silent (Opus DTX sends almost nothing) and after a while
 * the sender stops sending at all, so the host neither receives nor decodes a quiet room. The gate saves bandwidth,
 * it is no permission: the host decides who is mixed (§4.2).
 *
 * Signalling is `session.rtc` on the session channel; the host is ICE-lite, so the offer needs no candidates of this
 * side. The voice is on only once the connection to the host is up: a media port the packets cannot reach is said
 * (Q22). The host drops a connection whose channel went away: when the channel comes back, so does the voice. */

import type { SessionChannel } from './channel'
import type { SpeakMode } from './types'

const AUDIO: MediaTrackConstraints = { echoCancellation: true, noiseSuppression: true, autoGainControl: true }
const CONNECT_MS = 10_000
/** How long a closed gate keeps the sender on before it stops sending (§4.4). */
const IDLE_MS = { auto: 2000, manual: 300 } as const

/** The host's media port did not answer (a firewall in between, usually). */
export class VoiceUnreachable extends Error {
  constructor() { super('the media port of the host could not be reached') }
}

function connected(pc: RTCPeerConnection): Promise<void> {
  return new Promise((resolve, reject) => {
    let timer = 0
    const check = () => {
      const s = pc.connectionState
      if (s !== 'connected' && s !== 'failed' && s !== 'closed') return
      window.clearTimeout(timer)
      pc.removeEventListener('connectionstatechange', check)
      if (s === 'connected') resolve()
      else reject(new VoiceUnreachable())
    }
    timer = window.setTimeout(() => { pc.removeEventListener('connectionstatechange', check); reject(new VoiceUnreachable()) }, CONNECT_MS)
    pc.addEventListener('connectionstatechange', check)
    check()
  })
}

/* The gate (§4.4). The noise floor is a low percentile of the last 3 s of 100 ms minima; the gate opens once the level
 * stays `margin` dB above it for 30 ms and closes after 500 ms below. What goes out is delayed by 40 ms while the gate
 * decides on the undelayed signal, so the first word is not cut. Messages: in `{ mode, ptt }`, out `{ open }` on
 * every change and `{ level }` (0…1) ten times a second. */
const GATE = `
class AiwsMicGate extends AudioWorkletProcessor {
  constructor() {
    super()
    this.mode = 'auto'; this.ptt = false; this.margin = 12
    this.lookahead = Math.round(sampleRate * 0.04)
    this.ring = new Float32Array(this.lookahead + 256); this.w = 0
    this.attack = Math.round(sampleRate * 0.03); this.hang = Math.round(sampleRate * 0.5)
    this.above = 0; this.below = 0; this.open = false; this.gain = 0; this.step = 1 / Math.round(sampleRate * 0.005)
    this.hist = new Float32Array(30); this.filled = 0; this.hi = 0; this.floor = -60
    this.blockMin = Infinity; this.blockLen = 0; this.peak = -100
    this.port.onmessage = (e) => {
      const d = e.data || {}
      if (typeof d.mode === 'string') this.mode = d.mode
      if (typeof d.ptt === 'boolean') this.ptt = d.ptt
      if (typeof d.margin === 'number') this.margin = d.margin
    }
  }
  process(inputs, outputs) {
    const input = inputs[0] && inputs[0][0]
    const out = outputs[0] && outputs[0][0]
    if (!out) return true
    const n = out.length
    let e = 0
    if (input) for (let i = 0; i < n; i++) e += input[i] * input[i]
    const db = 10 * Math.log10(e / n + 1e-10)
    this.blockMin = Math.min(this.blockMin, db); this.peak = Math.max(this.peak, db); this.blockLen += n
    if (this.blockLen >= sampleRate * 0.1) {
      this.hist[this.hi] = this.blockMin; this.hi = (this.hi + 1) % this.hist.length; this.filled = Math.min(this.filled + 1, this.hist.length)
      const xs = Array.from(this.hist.subarray(0, this.filled)).sort((a, b) => a - b)
      this.floor = Math.max(-90, Math.min(-25, xs[Math.floor(xs.length * 0.1)]))
      this.port.postMessage({ level: Math.max(0, Math.min(1, (this.peak + 60) / 60)) })
      this.blockMin = Infinity; this.blockLen = 0; this.peak = -100
    }
    const loud = db > this.floor + this.margin && db > -65
    if (loud) { this.above += n; this.below = 0 } else { this.below += n; this.above = Math.max(0, this.above - 2 * n) }
    let want
    if (this.mode === 'mute') want = false
    else if (this.mode === 'manual') want = this.ptt
    else want = this.open ? this.below < this.hang : this.above >= this.attack
    if (want !== this.open) { this.open = want; this.port.postMessage({ open: want }) }
    const len = this.ring.length
    for (let i = 0; i < n; i++) {
      this.ring[this.w] = input ? input[i] : 0
      let r = this.w - this.lookahead
      if (r < 0) r += len
      if (this.open) this.gain = Math.min(1, this.gain + this.step)
      else this.gain = Math.max(0, this.gain - this.step)
      out[i] = this.ring[r] * this.gain
      this.w = (this.w + 1) % len
    }
    return true
  }
}
registerProcessor('aiws-mic-gate', AiwsMicGate)
`

let gateUrl: string | null = null
function gateModule(): string {
  gateUrl ??= URL.createObjectURL(new Blob([GATE], { type: 'text/javascript' }))
  return gateUrl
}

export interface VoiceSnapshot {
  state: 'idle' | 'connecting' | 'connected' | 'lost'
  /** The host takes this connection's voice into the mix (it may speak, §4.2). */
  sending: boolean
  mode: SpeakMode
  /** The microphone is on. */
  mic: boolean
  /** The browser refused the microphone. */
  micDenied: boolean
  /** The gate is open: what is said goes out now. */
  open: boolean
  /** The sender is sending (it stops a while after the gate closed). */
  transmitting: boolean
  /** The microphone's level, 0…1, for the meter. */
  level: number
  /** The browser holds audio until a click (its autoplay policy). */
  suspended: boolean
}

export class VoiceClient {
  readonly element: HTMLAudioElement
  private readonly channel: SessionChannel
  private pc: RTCPeerConnection | null = null
  private transceiver: RTCRtpTransceiver | null = null
  private stream: MediaStream | null = null
  private ctx: AudioContext | null = null
  private node: AudioWorkletNode | null = null
  private source: MediaStreamAudioSourceNode | null = null
  /** What goes out: the gate's output (or, without AudioWorklet, the microphone itself). */
  private track: MediaStreamTrack | null = null
  private idleTimer = 0
  private ptt = false
  private wantSend = false
  private retries = 0
  private closed = false
  private queue: Promise<unknown> = Promise.resolve()
  private offStatus: (() => void) | null = null
  private snap: VoiceSnapshot = { state: 'idle', sending: false, mode: 'mute', mic: false, micDenied: false, open: false, transmitting: false, level: 0, suspended: false }
  private readonly listeners = new Set<() => void>()
  private readonly onDevice = () => { void this.serial(() => this.switchDevice()) }

  constructor(channel: SessionChannel) {
    this.channel = channel
    this.element = new Audio()
    this.element.autoplay = true
  }

  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  snapshot = (): VoiceSnapshot => this.snap

  private set(patch: Partial<VoiceSnapshot>) {
    const next = { ...this.snap, ...patch }
    if ((Object.keys(patch) as (keyof VoiceSnapshot)[]).every((k) => next[k] === this.snap[k])) return
    this.snap = next
    for (const l of [...this.listeners]) l()
  }

  /** Signalling and the microphone happen one at a time. */
  private serial<T>(fn: () => Promise<T>): Promise<T> {
    const run = this.queue.then(fn, fn)
    this.queue = run.catch(() => undefined)
    return run
  }

  /** Connect: always to listen, to send too when `send` (the host decides, §4.2), in `mode`. Call it from a click
   * when there is one: the browser plays sound (and runs the gate) only after one. */
  start(send: boolean, mode: SpeakMode): Promise<void> {
    return this.serial(async () => {
      this.closed = false
      await this.open(send, mode)
      let last = this.channel.status
      this.offStatus?.()
      // the host dropped this connection with the channel it came on: a new channel, a new connection
      this.offStatus = this.channel.onStatus(() => {
        const now = this.channel.status
        if (now === 'open' && last !== 'open' && this.pc) void this.serial(() => this.reopen())
        last = now
      })
      navigator.mediaDevices?.addEventListener?.('devicechange', this.onDevice)
    })
  }

  private async open(send: boolean, mode: SpeakMode) {
    this.wantSend = send
    this.set({ state: 'connecting', mode })
    const pc = new RTCPeerConnection({ iceServers: [] })
    this.pc = pc
    this.transceiver = pc.addTransceiver('audio', { direction: send ? 'sendrecv' : 'recvonly' })
    pc.ontrack = (event) => {
      this.element.srcObject = event.streams[0] ?? new MediaStream([event.track])
      void this.element.play().then(() => this.set({ suspended: false }), () => this.set({ suspended: true }))
    }
    try {
      const sending = await this.negotiate(send)
      await connected(pc)
      this.set({ state: 'connected', sending })
      this.retries = 0
    } catch (error) {
      this.dropConnection()
      this.set({ state: 'idle', sending: false })
      throw error
    }
    pc.addEventListener('connectionstatechange', () => {
      if (pc !== this.pc || this.closed || (pc.connectionState !== 'failed' && pc.connectionState !== 'closed')) return
      if (this.retries++ < 3) window.setTimeout(() => { if (pc === this.pc) void this.serial(() => this.reopen()) }, 1000 * this.retries)
      else this.set({ state: 'lost' })
    })
    await this.applyMode(mode)
  }

  private async reopen() {
    if (this.closed) return
    const mode = this.snap.mode
    await this.micOff()
    this.dropConnection()
    try { await this.open(this.wantSend, mode) } catch { this.set({ state: 'lost' }) }
  }

  private async negotiate(send: boolean): Promise<boolean> {
    const pc = this.pc
    if (!pc) return false
    const offer = await pc.createOffer()
    await pc.setLocalDescription(offer)
    const answer = await this.channel.call<{ sdp: string; send: boolean }>('session.rtc', { op: 'offer', sdp: offer.sdp ?? '', send })
    await pc.setRemoteDescription({ type: 'answer', sdp: answer.sdp })
    const sender = this.transceiver?.sender
    if (answer.send && sender) {
      const params = sender.getParameters()
      if (params.encodings?.length && params.encodings[0].maxBitrate !== 40_000) {
        params.encodings[0].maxBitrate = 40_000
        await sender.setParameters(params).catch(() => undefined)
      }
    }
    return answer.send === true
  }

  /** The right to speak came or went (`voice.allow`): the same connection turns sending or listening. */
  setAllowed(allowed: boolean, mode: SpeakMode): Promise<void> {
    return this.serial(async () => {
      this.wantSend = allowed
      if (!this.pc || !this.transceiver || this.snap.state !== 'connected') return
      if (!allowed) await this.micOff()
      this.transceiver.direction = allowed ? 'sendrecv' : 'recvonly'
      const sending = await this.negotiate(allowed)
      this.set({ sending })
      await this.applyMode(sending ? mode : this.snap.mode)
    })
  }

  setMode(mode: SpeakMode): Promise<void> {
    return this.serial(() => this.applyMode(mode))
  }

  /** Manual mode: the "talk" button or key is held. */
  talk(down: boolean) {
    this.ptt = down
    this.node?.port.postMessage({ ptt: down })
    if (!this.node && this.snap.mode === 'manual') void this.attach(down)
  }

  /** A click lets held audio run (the notice's button). */
  resume() {
    void this.ctx?.resume().catch(() => undefined)
    void this.element.play().then(() => this.set({ suspended: false }), () => undefined)
  }

  setListenMuted(muted: boolean) { this.element.muted = muted }

  private async applyMode(mode: SpeakMode) {
    this.set({ mode })
    if (!this.snap.sending || mode === 'mute') {
      await this.micOff()
      return
    }
    if (!this.stream && !(await this.micOn())) {
      this.set({ mode: 'mute' })
      return
    }
    if (this.node) this.node.port.postMessage({ mode, ptt: this.ptt })
    else await this.attach(mode === 'auto' || this.ptt)
  }

  private async micOn(): Promise<boolean> {
    let stream: MediaStream
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: AUDIO })
    } catch {
      this.set({ micDenied: true })
      return false
    }
    this.stream = stream
    this.set({ mic: true, micDenied: false })
    try {
      const ctx = new AudioContext()
      this.ctx = ctx
      await ctx.audioWorklet.addModule(gateModule())
      const node = new AudioWorkletNode(ctx, 'aiws-mic-gate', { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1] })
      const dest = ctx.createMediaStreamDestination()
      this.source = ctx.createMediaStreamSource(stream)
      this.source.connect(node).connect(dest)
      node.port.onmessage = (event: MessageEvent<{ open?: boolean; level?: number }>) => this.onGate(event.data)
      this.node = node
      this.track = dest.stream.getAudioTracks()[0] ?? null
      if (ctx.state !== 'running') {
        this.set({ suspended: true })
        void ctx.resume().then(() => this.set({ suspended: ctx.state !== 'running' }), () => undefined)
      }
    } catch {
      // no AudioWorklet here: the microphone goes out as it is, without the gate
      void this.ctx?.close().catch(() => undefined)
      this.ctx = null
      this.node = null
      this.track = stream.getAudioTracks()[0] ?? null
    }
    return true
  }

  private onGate(m: { open?: boolean; level?: number }) {
    if (typeof m.level === 'number') this.set({ level: Math.round(m.level * 20) / 20 })
    if (typeof m.open !== 'boolean') return
    this.set({ open: m.open })
    window.clearTimeout(this.idleTimer)
    if (m.open) void this.attach(true)
    else this.idleTimer = window.setTimeout(() => { void this.attach(false) }, IDLE_MS[this.snap.mode === 'manual' ? 'manual' : 'auto'])
  }

  /** Send the gated track, or nothing at all (the sender stops sending, the connection stays). */
  private async attach(on: boolean) {
    const sender = this.transceiver?.sender
    if (!sender) return
    const want = on && this.snap.sending ? this.track : null
    if (sender.track !== want) await sender.replaceTrack(want).catch(() => undefined)
    this.set({ transmitting: want !== null })
  }

  private async switchDevice() {
    if (!this.stream) return
    try {
      const next = await navigator.mediaDevices.getUserMedia({ audio: AUDIO })
      if (this.ctx && this.node) {
        const source = this.ctx.createMediaStreamSource(next)
        this.source?.disconnect()
        source.connect(this.node)
        this.source = source
      } else {
        this.track = next.getAudioTracks()[0] ?? null
        if (this.snap.transmitting) await this.transceiver?.sender.replaceTrack(this.track).catch(() => undefined)
      }
      this.stream.getTracks().forEach((t) => t.stop())
      this.stream = next
      void this.channel.call('session.sample', { kind: 'device', data: { label: next.getAudioTracks()[0]?.label ?? '' } }).catch(() => undefined)
    } catch { /* keep the current device */ }
  }

  private async micOff() {
    window.clearTimeout(this.idleTimer)
    await this.attach(false)
    this.source?.disconnect()
    this.node?.disconnect()
    this.source = null
    this.node = null
    void this.ctx?.close().catch(() => undefined)
    this.ctx = null
    this.stream?.getTracks().forEach((t) => t.stop())
    this.stream = null
    this.track = null
    this.set({ mic: false, open: false, level: 0, transmitting: false })
  }

  private dropConnection() {
    this.pc?.close()
    this.pc = null
    this.transceiver = null
  }

  async stop() {
    this.closed = true
    this.offStatus?.()
    this.offStatus = null
    navigator.mediaDevices?.removeEventListener?.('devicechange', this.onDevice)
    await this.serial(async () => {
      const had = this.pc !== null
      await this.micOff()
      this.dropConnection()
      this.element.srcObject = null
      this.set({ state: 'idle', sending: false })
      if (had) await this.channel.call('session.rtc', { op: 'close' }).catch(() => undefined)
    })
  }
}
