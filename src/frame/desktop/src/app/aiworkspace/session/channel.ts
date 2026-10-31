/* The session channel (第四期规划 §5.1, Q04, Q16, Q18): one WebSocket to the Workspace host per terminal. It says
 * hello with the caller's own token (or the link), resumes the timeline after the last event it saw, keeps the
 * presence with a heartbeat every 10 s, estimates the demonstration clock from the round trip of the heartbeat
 * (never from this device's wall clock), carries `session.*` calls, and falls back to the long poll
 * (`session.watch`) when the socket cannot be opened. */

import type { Json } from '../api/types'
import type { SessionApi } from './api'
import type { SessionEvent, SessionInfo, SessionRole } from './types'

export type ChannelStatus = 'connecting' | 'open' | 'polling' | 'closed' | 'failed'
export type Purpose = 'presenter' | 'audience' | 'display' | 'prompter'

export interface Welcome {
  session: SessionInfo
  role: SessionRole
  principal: string
  level: 'verified' | 'guest'
  name: string
  terminal_id: string
  controller: string | null
  t_rel_ms: number
}

const PING_MS = 10_000
const WATCH_MS = 20_000
const FALLBACK_AFTER = 3

type Pending = { resolve: (v: unknown) => void; reject: (e: unknown) => void }

export class ChannelError extends Error {
  readonly error: { code: string; sub_code?: string; detail?: string }
  constructor(error: { code: string; sub_code?: string; detail?: string }) {
    super(`${error.code}${error.sub_code ? `/${error.sub_code}` : ''}${error.detail ? `: ${error.detail}` : ''}`)
    this.error = error
  }
}

export class SessionChannel {
  readonly api: SessionApi
  readonly purpose: Purpose
  readonly terminalId: string
  private readonly name: string | undefined
  private ws: WebSocket | null = null
  private closed = false
  private failures = 0
  private pingTimer = 0
  private retryTimer = 0
  private nextId = 1
  private readonly pending = new Map<string, Pending>()
  private readonly listeners = new Set<(event: SessionEvent) => void>()
  private readonly statusListeners = new Set<() => void>()
  private readonly controlListeners = new Set<(controller: string | null) => void>()
  private welcomeWaiters: { resolve: (w: Welcome) => void; reject: (e: unknown) => void }[] = []
  status: ChannelStatus = 'connecting'
  welcome: Welcome | null = null
  lastSeq = 0
  /** The demonstration time at `anchor` (performance.now()), from the last heartbeat's answer. */
  private clock: { tRel: number; anchor: number; rtt: number } | null = null
  failure: string | null = null

  constructor(api: SessionApi, purpose: Purpose, terminalId: string, name?: string) {
    this.api = api
    this.purpose = purpose
    this.terminalId = terminalId
    this.name = name
  }

  /** Connect and wait for the welcome (the first one only; reconnections resume quietly). */
  open(): Promise<Welcome> {
    if (this.welcome) return Promise.resolve(this.welcome)
    const p = new Promise<Welcome>((resolve, reject) => this.welcomeWaiters.push({ resolve, reject }))
    this.connect()
    return p
  }

  onEvent(listener: (event: SessionEvent) => void) { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  onStatus(listener: () => void) { this.statusListeners.add(listener); return () => { this.statusListeners.delete(listener) } }
  onControl(listener: (controller: string | null) => void) { this.controlListeners.add(listener); return () => { this.controlListeners.delete(listener) } }

  /** The demonstration time now, as the service's clock has it (Q16). */
  tRel(): number {
    if (!this.clock) return this.welcome?.t_rel_ms ?? 0
    return Math.round(this.clock.tRel + (performance.now() - this.clock.anchor))
  }

  private setStatus(status: ChannelStatus) {
    if (this.status === status) return
    this.status = status
    for (const l of [...this.statusListeners]) l()
  }

  private url(): string {
    const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    return `${proto}//${window.location.host}${this.api.transport.basePath()}/session/ws`
  }

  private connect() {
    if (this.closed) return
    let ws: WebSocket
    try {
      ws = new WebSocket(this.url())
    } catch {
      this.lost()
      return
    }
    this.ws = ws
    let welcomed = false
    ws.onopen = () => {
      void this.api.transport.channelToken().then((token) => {
        const hello: Record<string, Json> = { type: 'hello', session_id: this.api.sessionId, token: this.isLinkToken(token) && this.api.link ? null : token, terminal_id: this.terminalId, purpose: this.purpose, after: this.lastSeq }
        if (this.api.link) hello.link = this.api.link
        if (this.name) hello.name = this.name
        ws.send(JSON.stringify(hello))
      })
    }
    ws.onmessage = (message) => {
      let m: Record<string, unknown>
      try { m = JSON.parse(String(message.data)) as Record<string, unknown> } catch { return }
      switch (m.type) {
        case 'welcome': {
          welcomed = true
          this.failures = 0
          this.failure = null
          const w = m as unknown as Welcome
          this.welcome = w
          this.clock = { tRel: w.t_rel_ms, anchor: performance.now(), rtt: 0 }
          this.setStatus('open')
          for (const waiter of this.welcomeWaiters.splice(0)) waiter.resolve(w)
          this.startPing()
          // joined: the link is no longer needed for calls
          if (w.role === 'audience') this.api.link = null
          break
        }
        case 'event': {
          const e = m.event as SessionEvent
          if (e.seq > this.lastSeq) this.lastSeq = e.seq
          for (const l of [...this.listeners]) l(e)
          break
        }
        case 'pong': {
          const sent = Number(m.c)
          const now = performance.now()
          const rtt = Number.isFinite(sent) ? now - sent : 0
          const tRel = Number(m.t_rel_ms)
          if (Number.isFinite(tRel)) this.clock = { tRel: tRel + rtt / 2, anchor: now, rtt }
          break
        }
        case 'result': {
          const p = this.pending.get(String(m.id))
          if (!p) break
          this.pending.delete(String(m.id))
          if (m.error) p.reject(new ChannelError(m.error as ChannelError['error']))
          else p.resolve(m.result)
          break
        }
        case 'control': {
          const c = (m.controller as string | null) ?? null
          if (this.welcome) this.welcome = { ...this.welcome, controller: c }
          for (const l of [...this.controlListeners]) l(c)
          break
        }
        case 'resync':
          ws.close()
          break
        case 'error': {
          const err = (m.error as ChannelError['error'] | undefined) ?? { code: String(m.code ?? 'UNKNOWN'), detail: String(m.detail ?? '') }
          this.failure = new ChannelError(err).message
          if (!welcomed) {
            this.closed = true
            this.setStatus('failed')
            for (const waiter of this.welcomeWaiters.splice(0)) waiter.reject(new ChannelError(err))
          }
          break
        }
      }
    }
    ws.onclose = () => {
      window.clearInterval(this.pingTimer)
      for (const [id, p] of this.pending) { p.reject(new Error('the session channel closed')); this.pending.delete(id) }
      if (this.ws === ws) this.ws = null
      if (!this.closed) this.lost()
    }
  }

  private isLinkToken(token: string): boolean {
    return /^(dt|pt)_[a-z2-7]{26}$/.test(token)
  }

  private startPing() {
    window.clearInterval(this.pingTimer)
    const ping = () => { if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify({ type: 'ping', c: performance.now() })) }
    ping()
    this.pingTimer = window.setInterval(ping, PING_MS)
  }

  private lost() {
    if (this.closed) return
    this.failures += 1
    if (this.failures >= FALLBACK_AFTER && this.welcome === null) {
      // the socket never opened: the long poll carries the timeline (Q04: WebSocket first, polling only as the fallback)
      void this.poll()
      return
    }
    this.setStatus('connecting')
    const delay = Math.min(500 * 2 ** Math.min(this.failures, 5), 10_000)
    window.clearTimeout(this.retryTimer)
    this.retryTimer = window.setTimeout(() => this.connect(), delay)
  }

  private async poll() {
    this.setStatus('polling')
    try {
      const info = await this.api.join(this.name)
      const w: Welcome = { session: info, role: info.role ?? 'audience', principal: info.principal ?? '', level: info.level ?? 'verified', name: this.name ?? '', terminal_id: this.terminalId, controller: info.controller, t_rel_ms: info.t_rel_ms ?? 0 }
      this.welcome = w
      this.clock = { tRel: w.t_rel_ms, anchor: performance.now(), rtt: 0 }
      for (const waiter of this.welcomeWaiters.splice(0)) waiter.resolve(w)
    } catch (error) {
      this.setStatus('failed')
      for (const waiter of this.welcomeWaiters.splice(0)) waiter.reject(error)
      return
    }
    while (!this.closed) {
      try {
        const page = await this.api.watch(this.lastSeq, WATCH_MS)
        for (const e of page.events) {
          if (e.seq <= this.lastSeq) continue
          this.lastSeq = e.seq
          for (const l of [...this.listeners]) l(e)
        }
      } catch {
        await new Promise((r) => window.setTimeout(r, 2000))
      }
    }
  }

  /** A `session.*` call: over the socket when it is open (as this terminal), otherwise over kRPC. */
  call<T>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    const ws = this.ws
    if (!ws || ws.readyState !== WebSocket.OPEN || this.status !== 'open') {
      return this.api.call<T>(method, { ...params, terminal_id: this.terminalId })
    }
    const id = `c${this.nextId++}`
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject })
      ws.send(JSON.stringify({ type: 'call', id, method, params }))
    })
  }

  /** Following the presenter or browsing on one's own (the stage feed counts the followers). */
  follow(on: boolean) {
    if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify({ type: 'follow', on }))
  }

  close() {
    this.closed = true
    window.clearInterval(this.pingTimer)
    window.clearTimeout(this.retryTimer)
    this.ws?.close()
    this.setStatus('closed')
  }
}
