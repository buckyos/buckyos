/* The voice of a session (第四期规划 §7.3.1): the presenter sends the microphone to the Workspace host over WebRTC, the
 * audience receives it from there. Signalling is `session.rtc` on the session channel; the host is ICE-lite, so the
 * offer needs no candidates of this side. A device change (Bluetooth headset in or out) replaces the track without a
 * new connection and is put on the time line. */

import type { SessionChannel } from './channel'

const AUDIO: MediaTrackConstraints = { echoCancellation: true, noiseSuppression: true, autoGainControl: true }

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
  private readonly onDevice = () => { void this.switchDevice() }
  /** The level meter's analyser of the microphone. */
  analyser: AnalyserNode | null = null
  private audio: AudioContext | null = null

  constructor(channel: SessionChannel) {
    this.channel = channel
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
    await negotiate(this.channel, pc, true)
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
    await negotiate(this.channel, pc, false)
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
