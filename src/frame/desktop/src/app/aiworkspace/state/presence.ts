/* Presence (UI 框架 PRD §4.2, §4.5, §12, K2): who has this Workspace open right now. One reporter per open Workspace
 * (`presenceOf(store)`): while anything uses it (`usePresence`, or `retain()`), this window registers with the service,
 * reports the canvas it shows every `interval_ms` (10 s) and at once when that changes, and leaves when its last user
 * goes. The members come back with each heartbeat: one per person (all its windows and devices together), this window's
 * person first. Without the service — offline, refused, before the first answer — the list is this window's own person
 * only, never the grant list or the authors of Commits standing in for who is online (§4.2 “接入前”). Presence is the
 * service's memory: it is never a Commit, never queued and never stored here. */

import { useEffect, useSyncExternalStore } from 'react'
import type { Json, PresenceMember } from '../api/types'
import { isGuestPrincipal } from '../api/transport'
import { aiwsT } from '../i18n'
import { Emitter } from './emitter'
import { guestLabel, readGuestName } from './guest'
import { useStore } from './hooks'
import type { WorkspaceStore } from './store'

const DEFAULT_INTERVAL_MS = 10_000
/** A release followed at once by a retain (a remount) does not leave and register again. */
const RELEASE_GRACE_MS = 3_000
/** Canvas switches in a row are reported once. */
const SURFACE_DEBOUNCE_MS = 250

export interface PresenceSnapshot {
  /** This window's person first, then the others by arrival. */
  members: readonly PresenceMember[]
  /** The service answered the latest heartbeat; false: `members` is this window's person only. */
  live: boolean
  /** How many people are online (at least this one). */
  online: number
}

function nowText(): string {
  return new Date().toISOString()
}

export class PresenceService {
  private readonly store: WorkspaceStore
  private readonly emitter = new Emitter()
  readonly subscribe = this.emitter.subscribe
  private snap: PresenceSnapshot
  private users = 0
  private running = false
  private inFlight = false
  private again = false
  private interval = DEFAULT_INTERVAL_MS
  private timer: number | null = null
  private releaseTimer: number | null = null
  private surfaceTimer: number | null = null
  private reported: string | null | undefined = undefined
  private offs: (() => void)[] = []

  constructor(store: WorkspaceStore) {
    this.store = store
    this.snap = { members: [this.selfMember()], live: false, online: 1 }
  }

  snapshot = (): PresenceSnapshot => this.snap

  /** Use presence while the returned release is not called (several users share one registration). */
  retain(): () => void {
    this.users += 1
    if (this.releaseTimer !== null) { window.clearTimeout(this.releaseTimer); this.releaseTimer = null }
    if (!this.running) this.start()
    let released = false
    return () => {
      if (released) return
      released = true
      this.users -= 1
      if (this.users === 0 && this.releaseTimer === null) this.releaseTimer = window.setTimeout(() => { this.releaseTimer = null; if (this.users === 0) this.stop() }, RELEASE_GRACE_MS)
    }
  }

  /** Report now (the next regular heartbeat follows `interval_ms` later). */
  beat(): Promise<void> {
    if (!this.running) return Promise.resolve()
    if (this.inFlight) { this.again = true; return Promise.resolve() }
    return this.send()
  }

  /** The canvas this window shows: the remembered active Surface when it still exists, else the first one (the
   * shell's own rule, WorkspaceShell). Data mode keeps reporting the canvas it would return to. */
  currentSurface(): string | null {
    const outline = this.store.outline
    const remembered = this.store.userState.get<Json>('surface:active')
    if (typeof remembered === 'string') {
      const e = outline.get(remembered)
      if (e && e.kind === 'surface' && !e.deleted) return remembered
    }
    return outline.childrenOf('surfaces').find((e) => e.kind === 'surface' && !e.deleted)?.entity_id ?? null
  }

  private selfMember(): PresenceMember {
    const principal = this.store.session.principal ?? ''
    const guest = isGuestPrincipal(principal)
    const at = nowText()
    return { principal, self: true, guest, name: guest ? readGuestName() || null : null, surface_id: this.reported ?? null, devices: 1, since: at, last_seen: at }
  }

  private set(members: PresenceMember[], live: boolean) {
    const self = members.filter((m) => m.self)
    const ordered = self.length > 0 ? [...self, ...members.filter((m) => !m.self)] : [this.selfMember(), ...members]
    this.snap = { members: ordered, live, online: ordered.length }
    this.emitter.emit()
  }

  private start() {
    this.running = true
    const onSurface = () => {
      if (this.currentSurface() === this.reported) return
      if (this.surfaceTimer !== null) window.clearTimeout(this.surfaceTimer)
      this.surfaceTimer = window.setTimeout(() => { this.surfaceTimer = null; void this.beat() }, SURFACE_DEBOUNCE_MS)
    }
    const onVisible = () => { if (document.visibilityState === 'visible') void this.beat() }
    const onHide = () => { void this.store.session.presenceLeave().catch(() => undefined) }
    let wasLive = this.store.session.status().kind === 'live'
    const onStatus = () => {
      const live = this.store.session.status().kind === 'live'
      if (live && !wasLive) void this.beat()
      if (!live && this.snap.live) this.set([], false)
      wasLive = live
    }
    document.addEventListener('visibilitychange', onVisible)
    window.addEventListener('pagehide', onHide)
    this.offs = [
      this.store.userState.subscribeKey('surface:active', onSurface),
      this.store.outline.subscribe(onSurface),
      this.store.session.subscribeStatus(onStatus),
      () => document.removeEventListener('visibilitychange', onVisible),
      () => window.removeEventListener('pagehide', onHide),
    ]
    void this.send()
  }

  private stop() {
    this.running = false
    for (const off of this.offs.splice(0)) off()
    for (const timer of [this.timer, this.surfaceTimer]) if (timer !== null) window.clearTimeout(timer)
    this.timer = null
    this.surfaceTimer = null
    void this.store.session.presenceLeave().catch(() => undefined)
    this.set([], false)
  }

  private async send(): Promise<void> {
    this.inFlight = true
    if (this.timer !== null) { window.clearTimeout(this.timer); this.timer = null }
    const surface = this.currentSurface()
    this.reported = surface
    const guest = isGuestPrincipal(this.store.session.principal ?? '')
    try {
      const list = await this.store.session.presenceHeartbeat(surface, guest ? readGuestName() || undefined : undefined)
      if (this.running) {
        if (list.interval_ms > 0) this.interval = list.interval_ms
        this.set(list.members, true)
      } else {
        // it registered after this window stopped using presence: leave again
        void this.store.session.presenceLeave().catch(() => undefined)
      }
    } catch {
      if (this.running) this.set([], false)
    } finally {
      this.inFlight = false
      if (this.running) {
        if (this.again) { this.again = false; void this.send() } else this.timer = window.setTimeout(() => { this.timer = null; void this.send() }, this.interval)
      }
    }
  }
}

const services = new WeakMap<WorkspaceStore, PresenceService>()

/** The presence reporter of an open Workspace (created on first use; nothing is sent until something retains it). */
export function presenceOf(store: WorkspaceStore): PresenceService {
  let service = services.get(store)
  if (!service) { service = new PresenceService(store); services.set(store, service) }
  return service
}

/** Who is online in this Workspace, this window's person first; registers this window while mounted. Mount it in a
 * part of the shell that is always there (the status bar), so the window registers when the Workspace opens. */
export function usePresence(): PresenceSnapshot {
  const service = presenceOf(useStore())
  useEffect(() => service.retain(), [service])
  return useSyncExternalStore(service.subscribe, service.snapshot)
}

/** The name a member is shown by: a guest's chosen name or “访客 abc123”; an account's principal (the panel may show
 * the signed-in user's own display name for the `self` row). */
export function presenceName(member: PresenceMember): string {
  if (member.guest) return member.name?.trim() || guestLabel(member.principal)
  return member.principal
}

/** What the member list shows as the group (PRD §4.2): `所有者` for the owner, else the titles of its logical groups,
 * else `访客`; null when the reader may not see it (someone else's, to a non-manager, Q6). */
export function presenceGroupLabel(member: PresenceMember): string | null {
  if (member.owner === undefined && member.groups === undefined) return null
  const titles = (member.groups ?? []).map((g) => g.title || g.group_id)
  if (member.owner) return [aiwsT('aiworkspace.activity.group.owner'), ...titles].join(aiwsT('aiworkspace.activity.group.separator'))
  return titles.length > 0 ? titles.join(aiwsT('aiworkspace.activity.group.separator')) : aiwsT('aiworkspace.activity.group.guest')
}
