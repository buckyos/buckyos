/**
 * Preview window scheduler — the Preview App's cross-window responsibility
 * (PRD §7.1, §13). It keeps per-window Session metadata (§13.7), applies the
 * Smart / Single Window policy (`windowPolicy.ts`) and drives the desktop
 * store through the generic `openAppWindow` primitive. Only metadata crosses
 * this boundary — never content (§17.3).
 */

import {
  implicitContainerOf,
  parentCyfsPath,
  refDisplayName,
  refIdentity,
  sessionKeyOf,
} from '../../components/preview/session'
import { extensionOf, mediaTypeFromExtension } from '../../components/preview/mediaTypes'
import { isCyfsPathRef, type ContentRef, type PreviewSessionContext } from '../../components/preview/types'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import { previewSettingsStore } from './settings'
import { PREVIEW_APP_ID, PREVIEW_LAUNCH_KIND, type PreviewLaunchPayload, type PreviewOpenRequest } from './types'
import { chooseWindow, type PreviewRequestInfo, type PreviewWindowMeta } from './windowPolicy'

interface ManagedWindow extends PreviewWindowMeta {
  payload: PreviewLaunchPayload | null
}

const LAST_SESSION_KEY = 'buckyos.preview.lastSession.v1'
const ITEM_KEY_SAMPLE = 256

let requestCounter = 0
function newRequestId(): string {
  requestCounter += 1
  return `pv-${Date.now().toString(36)}-${requestCounter}`
}

function containerKeyOf(source: ContentRef, session: PreviewSessionContext | undefined): string | undefined {
  if (session?.kind === 'container') return refIdentity(session.container)
  const implicit = implicitContainerOf(source)
  return implicit ? refIdentity(implicit) : undefined
}

function parentContainerKeyOf(source: ContentRef, session: PreviewSessionContext | undefined): string | undefined {
  const container = session?.kind === 'container' ? session.container : implicitContainerOf(source)
  if (!container || !isCyfsPathRef(container)) return undefined
  const parent = parentCyfsPath(container.path)
  return parent ? refIdentity({ kind: 'cyfs-path', path: parent }) : undefined
}

function itemKeysOf(session: PreviewSessionContext | undefined): string[] | undefined {
  if (session?.kind !== 'list') return undefined
  return session.items.slice(0, ITEM_KEY_SAMPLE).map((item) => item.id ?? refIdentity(item.source))
}

function requestInfoOf(request: PreviewOpenRequest): PreviewRequestInfo {
  const { source, session } = request
  const name = refDisplayName(source)
  return {
    currentKey: refIdentity(source),
    sessionId: session?.kind === 'list' || session?.kind === 'provider' ? session.sessionId : undefined,
    sessionKey: session ? sessionKeyOf(session, source) : undefined,
    sessionKind: session?.kind ?? 'single',
    containerKey: containerKeyOf(source, session),
    parentContainerKey: parentContainerKeyOf(source, session),
    originApp: request.origin?.app,
    hostContext: request.origin?.hostContext,
    mediaType: mediaTypeFromExtension(extensionOf(name)),
  }
}

/** Applies the app-level navigation defaults when the host did not decide (§13.8). */
function withNavigationDefaults(session: PreviewSessionContext | undefined): PreviewSessionContext | undefined {
  if (!session) return session
  const settings = previewSettingsStore.getSnapshot()
  if (session.kind === 'container' && !session.navigation) return { ...session, navigation: settings.containerNavigation }
  if (session.kind === 'list' && !session.navigation) return { ...session, navigation: settings.listNavigation }
  return session
}

function isPersistable(payload: PreviewLaunchPayload): boolean {
  const refs: ContentRef[] = [payload.source]
  if (payload.session?.kind === 'list') refs.push(...payload.session.items.map((i) => i.source))
  if (payload.session?.kind === 'container') refs.push(payload.session.container, payload.session.current)
  return refs.every((ref) => ref.kind === 'cyfs-path' || ref.kind === 'object-id')
}

export class PreviewWindowManager {
  private windows = new Map<string, ManagedWindow>()

  constructor() {
    desktopUIStore.subscribe(() => this.prune())
  }

  /** Drop metadata of windows the shell has closed. */
  private prune() {
    const alive = new Set(desktopUIStore.getSnapshot().runtime.windows.map((w) => w.id))
    for (const id of [...this.windows.keys()]) {
      if (!alive.has(id)) this.windows.delete(id)
    }
  }

  list(): PreviewWindowMeta[] {
    this.prune()
    return [...this.windows.values()]
  }

  get(windowId: string): ManagedWindow | undefined {
    return this.windows.get(windowId)
  }

  /**
   * Opens content in the Preview App. Returns the window id (null when the
   * Preview App is not installed in this desktop).
   */
  open(request: PreviewOpenRequest): string | null {
    this.prune()
    const settings = previewSettingsStore.getSnapshot()
    const session = withNavigationDefaults(request.session)
    const base: Omit<PreviewLaunchPayload, 'requestId' | 'createdBy'> = {
      kind: PREVIEW_LAUNCH_KIND,
      source: request.source,
      session,
      uiMode: request.uiMode ?? settings.defaultUiMode,
      fitMode: request.fitMode ?? settings.defaultFitMode,
      origin: request.origin,
    }
    const title = refDisplayName(request.source)
    const info = requestInfoOf({ ...request, session })
    const now = Date.now()

    if (request.newWindow) {
      const payload: PreviewLaunchPayload = { ...base, requestId: newRequestId(), createdBy: 'manual' }
      const windowId = desktopUIStore.openAppWindow(PREVIEW_APP_ID, { newInstance: true, launch: { requestId: payload.requestId, payload }, title })
      if (!windowId) return null
      this.windows.set(windowId, this.metaFor(windowId, 'manual', payload, info, now))
      return windowId
    }

    const decision = chooseWindow(this.list(), info, { windowMode: settings.windowMode, autoWindowLimit: settings.autoWindowLimit }, now)
    if (decision.action === 'create') {
      const payload: PreviewLaunchPayload = { ...base, requestId: newRequestId(), createdBy: 'auto' }
      const windowId = desktopUIStore.openAppWindow(PREVIEW_APP_ID, { newInstance: true, launch: { requestId: payload.requestId, payload }, title })
      if (!windowId) return null
      this.windows.set(windowId, this.metaFor(windowId, 'auto', payload, info, now))
      this.rememberLast(payload)
      return windowId
    }

    const target = this.windows.get(decision.windowId)
    let payload: PreviewLaunchPayload
    if (decision.mode === 'jump' && target?.payload) {
      // Same browsing set: keep the window's session, move to the requested item.
      payload = { ...target.payload, requestId: newRequestId(), source: request.source, session: session ?? target.payload.session }
    } else if (decision.mode === 'append' && target?.payload?.session?.kind === 'list') {
      const items = [...target.payload.session.items, { source: request.source, title }]
      payload = {
        ...target.payload,
        requestId: newRequestId(),
        source: request.source,
        session: { ...target.payload.session, items, currentIndex: items.length - 1, version: String(now) },
      }
    } else {
      payload = { ...base, requestId: newRequestId(), createdBy: 'auto' }
    }
    const windowId = desktopUIStore.openAppWindow(PREVIEW_APP_ID, { windowId: decision.windowId, launch: { requestId: payload.requestId, payload }, title })
    if (!windowId) return null
    const meta = this.metaFor(windowId, target?.createdBy ?? 'auto', payload, requestInfoOf(payload), now)
    meta.createdAt = target?.createdAt ?? now
    meta.pinned = target?.pinned ?? false
    this.windows.set(windowId, meta)
    this.rememberLast(payload)
    return windowId
  }

  /** Panels announce payloads the scheduler did not hand out (landing page, restore). */
  register(windowId: string, payload: PreviewLaunchPayload) {
    const existing = this.windows.get(windowId)
    if (existing?.payload?.requestId === payload.requestId) return
    const now = Date.now()
    const meta = this.metaFor(windowId, existing?.createdBy ?? payload.createdBy, payload, requestInfoOf(payload), now)
    meta.createdAt = existing?.createdAt ?? now
    meta.pinned = existing?.pinned ?? false
    meta.allowAutoReuse = existing?.allowAutoReuse ?? payload.createdBy === 'auto'
    this.windows.set(windowId, meta)
  }

  touch(windowId: string, patch?: Partial<Pick<PreviewWindowMeta, 'currentKey' | 'currentMediaType'>>) {
    const meta = this.windows.get(windowId)
    if (!meta) return
    meta.lastActiveAt = Date.now()
    if (patch) Object.assign(meta, patch)
  }

  setPinned(windowId: string, pinned: boolean) {
    const meta = this.windows.get(windowId)
    if (!meta) return
    meta.pinned = pinned
    meta.allowAutoReuse = !pinned && meta.createdBy === 'auto'
  }

  isPinned(windowId: string): boolean {
    return this.windows.get(windowId)?.pinned ?? false
  }

  private metaFor(windowId: string, createdBy: 'auto' | 'manual', payload: PreviewLaunchPayload, info: PreviewRequestInfo, now: number): ManagedWindow {
    return {
      windowId,
      createdBy,
      createdAt: now,
      lastActiveAt: now,
      pinned: false,
      allowAutoReuse: createdBy === 'auto',
      originApp: info.originApp,
      hostContext: info.hostContext,
      sessionId: info.sessionId,
      sessionKey: info.sessionKey,
      sessionKind: info.sessionKind,
      containerKey: info.containerKey,
      itemKeys: itemKeysOf(payload.session),
      currentKey: info.currentKey,
      currentMediaType: info.mediaType,
      payload,
    }
  }

  private rememberLast(payload: PreviewLaunchPayload) {
    if (!isPersistable(payload)) return
    try {
      window.localStorage.setItem(LAST_SESSION_KEY, JSON.stringify({ source: payload.source, session: payload.session, uiMode: payload.uiMode, fitMode: payload.fitMode, origin: payload.origin }))
    } catch {
      // best effort
    }
  }

  /** Last automatically opened session, for the "restore" setting (§13.8). */
  lastSession(): PreviewOpenRequest | null {
    try {
      const raw = window.localStorage.getItem(LAST_SESSION_KEY)
      if (!raw) return null
      const parsed = JSON.parse(raw) as PreviewOpenRequest
      return parsed && typeof parsed.source === 'object' ? parsed : null
    } catch {
      return null
    }
  }
}

export const previewWindowManager = new PreviewWindowManager()
