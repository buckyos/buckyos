/* The voice of a session (第四期规划 §7.3.1): the presenter sends the microphone to the Workspace host over WebRTC, the
 * audience receives it from there. Signalling is `session.rtc` on the session channel; the host is ICE-lite, so the
 * offer needs no candidates of this side. The voice is on only once the connection to the host is up: a media port the
 * packets cannot reach is said (Q22), not shown as a microphone that is on. A device change (Bluetooth headset in or
 * out) replaces the track without a new connection and is put on the time line. */

import type { SessionChannel } from './channel'

const AUDIO: MediaTrackConstraints = { echoCancellation: true, noiseSuppression: true, autoGainControl: true }
const CONNECT_MS = 10_000

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

async function negotiate(channel: SessionChannel, pc: RTCPeerConnection, publish: boolean): Promise<void> {
  const offer = await pc.createOffer()
  await pc.setLocalDescription(offer)
  const answer = await channel.call<{ sdp: string }>('session.rtc', { op: 'offer', sdp: offer.sdp ?? '', publish })
  await pc.setRemoteDescription({ type: 'answer', sdp: answer.sdp })
}

export class VoicePublisher {
  private pc: RTCPeerConnection | null = null
  private stream: MediaStream | null = null
  private sender: RTCRtpSender | null = null
  private readonly channel: SessionChannel
  private readonly onLost: () => void
  private readonly onDevice = () => { void this.switchDevice() }
  /** The level meter's analyser of the microphone. */
  analyser: AnalyserNode | null = null
  private audio: AudioContext | null = null

  /** `onLost`: the connection went down after it was up. */
  constructor(channel: SessionChannel, onLost: () => void) {
    this.channel = channel
    this.onLost = onLost
  }

  async start(): Promise<void> {
    this.stream = await navigator.mediaDevices.getUserMedia({ audio: AUDIO })
    const pc = new RTCPeerConnection({ iceServers: [] })
    this.pc = pc
    const track = this.stream.getAudioTracks()[0]
    const transceiver = pc.addTransceiver(track, { direction: 'sendonly', streams: [this.stream] })
    this.sender = transceiver.sender
    const params = this.sender.getParameters()
    if (params.encodings?.length) {
      params.encodings[0].maxBitrate = 40_000
      await this.sender.setParameters(params).catch(() => undefined)
    }
    try {
      await negotiate(this.channel, pc, true)
      await connected(pc)
    } catch (error) {
      await this.stop()
      throw error
    }
    pc.addEventListener('connectionstatechange', () => { if (pc === this.pc && pc.connectionState === 'failed') this.onLost() })
    navigator.mediaDevices.addEventListener('devicechange', this.onDevice)
    try {
      this.audio = new AudioContext()
      this.analyser = this.audio.createAnalyser()
      this.audio.createMediaStreamSource(this.stream).connect(this.analyser)
    } catch { /* no meter */ }
  }

  get state(): RTCPeerConnectionState { return this.pc?.connectionState ?? 'closed' }

  private async switchDevice() {
    if (!this.sender) return
    try {
      const next = await navigator.mediaDevices.getUserMedia({ audio: AUDIO })
      const track = next.getAudioTracks()[0]
      await this.sender.replaceTrack(track)
      this.stream?.getTracks().forEach((t) => t.stop())
      this.stream = next
      void this.channel.call('session.sample', { kind: 'device', data: { label: track.label } }).catch(() => undefined)
    } catch { /* keep the current device */ }
  }

  setMuted(muted: boolean) {
    this.stream?.getAudioTracks().forEach((t) => { t.enabled = !muted })
  }

  async stop() {
    navigator.mediaDevices.removeEventListener('devicechange', this.onDevice)
    this.stream?.getTracks().forEach((t) => t.stop())
    this.pc?.close()
    this.pc = null
    void this.audio?.close().catch(() => undefined)
    await this.channel.call('session.rtc', { op: 'close' }).catch(() => undefined)
  }
}

export class VoiceListener {
  private pc: RTCPeerConnection | null = null
  readonly element: HTMLAudioElement
  private readonly channel: SessionChannel

  constructor(channel: SessionChannel) {
    this.channel = channel
    this.element = new Audio()
    this.element.autoplay = true
  }

  /** Call from a user gesture (the "join" button): browsers play sound only after one. */
  async start(): Promise<void> {
    const pc = new RTCPeerConnection({ iceServers: [] })
    this.pc = pc
    pc.addTransceiver('audio', { direction: 'recvonly' })
    pc.ontrack = (event) => {
      this.element.srcObject = event.streams[0] ?? new MediaStream([event.track])
      void this.element.play().catch(() => undefined)
    }
    try {
      await negotiate(this.channel, pc, false)
      await connected(pc)
    } catch (error) {
      await this.stop()
      throw error
    }
  }

  get state(): RTCPeerConnectionState { return this.pc?.connectionState ?? 'closed' }

  setMuted(muted: boolean) { this.element.muted = muted }

  async stop() {
    this.pc?.close()
    this.pc = null
    this.element.srcObject = null
    await this.channel.call('session.rtc', { op: 'close' }).catch(() => undefined)
  }
}
