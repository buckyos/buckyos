/* The stage's side of the show relay (第三期规划 §11.3, §12): it renews the show lock and the clone, publishes the
 * newest state (one request at a time, the latest wins), receives the commands prompters send, and ends the show.
 * A show that ended elsewhere (forced end, lease lost) is reported once through `onEnded`. */

import { AiwsClient, ServiceFailure, unwrap } from '../api/client'
import type { ShowCommand, ShowStart, ShowState } from '../api/types'
import { TransportError } from '../api/transport'

/** The stage renews well within the lease (§8.1: every 20 s, lease 60 s). */
const HEARTBEAT_SHARE = 3
const WATCH_MS = 25_000

function ended(error: unknown): boolean {
  return error instanceof ServiceFailure && error.code === 'NOT_FOUND' && error.subCode === 'SHOW_ENDED'
}

export class ShowRelay {
  readonly start: ShowStart
  private readonly client: AiwsClient
  private readonly ws: { workspace_id: string }
  private closed = false
  private heartbeat = 0
  private seq = 0
  private publishing = false
  private queued: ShowState | null = null
  private watch: AbortController | null = null
  private cursor = 0
  onCommand: ((command: ShowCommand) => void) | null = null
  onEnded: ((reason: string) => void) | null = null
  /** The relay could not be reached for a while (the show goes on locally). */
  onTrouble: ((detail: string | null) => void) | null = null

  constructor(client: AiwsClient, workspaceId: string, start: ShowStart, resume?: { seq: number; cursor: number }) {
    this.client = client
    this.ws = { workspace_id: workspaceId }
    this.start = start
    if (resume) { this.seq = resume.seq; this.cursor = resume.cursor }
  }

  get showId(): string { return this.start.show_id }

  /** Heartbeats and the command loop. */
  run() {
    const every = Math.max(1000, Math.floor(this.start.lease_ms / HEARTBEAT_SHARE))
    this.heartbeat = window.setInterval(() => { void this.beat() }, every)
    void this.followCommands()
  }

  private async beat() {
    if (this.closed) return
    try {
      unwrap(await this.client.showHeartbeat(this.ws, this.showId))
      this.onTrouble?.(null)
    } catch (error) {
      if (ended(error)) this.end('放映已在别处结束（被所有者强制结束，或舞台超过租期没有续期）')
      else this.onTrouble?.(error instanceof Error ? error.message : String(error))
    }
  }

  /** Publish the newest state; a state queued while another one is on its way replaces the queued one. */
  publish(state: ShowState) {
    if (this.closed) return
    this.queued = state
    if (!this.publishing) void this.flush()
  }

  private async flush() {
    this.publishing = true
    try {
      while (this.queued && !this.closed) {
        const state = this.queued
        this.queued = null
        this.seq += 1
        try {
          unwrap(await this.client.showPublish(this.ws, this.showId, this.seq, state))
        } catch (error) {
          if (ended(error)) { this.end('放映已在别处结束'); return }
          // the next state goes out with a newer seq; prompters keep the last one they saw
        }
      }
    } finally {
      this.publishing = false
    }
  }

  private async followCommands() {
    let backoff = 500
    while (!this.closed) {
      this.watch = new AbortController()
      try {
        const page = unwrap(await this.client.showWatch(this.ws, this.showId, { after_command: this.cursor, timeout_ms: WATCH_MS }, this.watch.signal))
        backoff = 500
        for (const { cursor, command } of page.commands) {
          if (cursor <= this.cursor) continue
          this.cursor = cursor
          try { this.onCommand?.(command) } catch (error) { console.error('[aiworkspace] show command failed', error) }
        }
      } catch (error) {
        if (this.closed) return
        if (ended(error)) { this.end('放映已在别处结束（被所有者强制结束，或舞台超过租期没有续期）'); return }
        if (!(error instanceof TransportError) && !(error instanceof ServiceFailure)) throw error
        await new Promise((resolve) => window.setTimeout(resolve, backoff))
        backoff = Math.min(backoff * 2, 8000)
      }
    }
  }

  /** Where a refreshed stage continues from. */
  position(): { seq: number; cursor: number } { return { seq: this.seq, cursor: this.cursor } }

  private end(reason: string) {
    if (this.closed) return
    this.stop()
    this.onEnded?.(reason)
  }

  private stop() {
    this.closed = true
    window.clearInterval(this.heartbeat)
    this.watch?.abort()
  }

  /** End the show: the lock is released, the clone deleted and every prompter link dies. */
  async close(): Promise<void> {
    if (this.closed) return
    this.stop()
    try { unwrap(await this.client.showEnd(this.ws, this.showId)) } catch { /* the lease runs out by itself */ }
  }

  /** Leave without ending (a reload): the show lives on until its lease runs out. */
  detach() { this.stop() }
}

/** The prompter link of a show (§11.1): the token after `#` never reaches access logs or Referer headers. */
export function prompterLink(workspaceId: string, start: Pick<ShowStart, 'show_id' | 'prompter_token'>): string {
  return `${window.location.origin}/workspace/${encodeURIComponent(workspaceId)}/show/${encodeURIComponent(start.show_id)}#k=${encodeURIComponent(start.prompter_token)}`
}

/** A show this tab is running, kept for a reload of the stage (§11.3: "舞台刷新后继续"). Per tab, never shared. */
export interface ActiveShow { workspaceId: string; pathId: string; start: ShowStart; seq: number; cursor: number; stepId: string | null }
const ACTIVE_KEY = 'aiworkspace.active-show'

export function saveActiveShow(show: ActiveShow | null) {
  try {
    if (show) window.sessionStorage.setItem(`${ACTIVE_KEY}:${show.workspaceId}`, JSON.stringify(show))
  } catch { /* a reload then simply ends the show */ }
}
export function clearActiveShow(workspaceId: string) {
  try { window.sessionStorage.removeItem(`${ACTIVE_KEY}:${workspaceId}`) } catch { /* nothing kept */ }
}
export function loadActiveShow(workspaceId: string): ActiveShow | null {
  try {
    const raw = window.sessionStorage.getItem(`${ACTIVE_KEY}:${workspaceId}`)
    return raw ? JSON.parse(raw) as ActiveShow : null
  } catch {
    return null
  }
}
