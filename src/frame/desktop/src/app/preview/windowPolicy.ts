/**
 * Preview window policy — pure decisions, no DOM (PRD §13.4 – §13.7).
 *
 * Given the metadata of the open Preview windows and the metadata of a new
 * request, decide whether to create a window or reuse one and how. Manual
 * and pinned windows are never chosen for unrelated requests; the automatic
 * window count is capped; at the cap the most relevant automatic window is
 * reused. Kept free of imports so it can be unit-tested with plain node.
 */

export type PreviewWindowOrigin = 'auto' | 'manual'
export type ReuseMode = 'jump' | 'append' | 'replace'

export interface PreviewWindowMeta {
  windowId: string
  createdBy: PreviewWindowOrigin
  createdAt: number
  lastActiveAt: number
  pinned: boolean
  allowAutoReuse: boolean
  originApp?: string
  hostContext?: string
  sessionId?: string
  /** Identity of the resolved browsing set (container / explicit list). */
  sessionKey?: string
  sessionKind?: 'single' | 'container' | 'list' | 'provider'
  containerKey?: string
  /** Identities of the items in the window's session (bounded sample). */
  itemKeys?: string[]
  currentKey?: string
  currentMediaType?: string
}

export interface PreviewRequestInfo {
  currentKey: string
  sessionId?: string
  sessionKey?: string
  sessionKind?: PreviewWindowMeta['sessionKind']
  containerKey?: string
  /** Parent of the request's container (parent ↔ child navigation). */
  parentContainerKey?: string
  originApp?: string
  hostContext?: string
  mediaType?: string
}

export interface PreviewWindowPolicy {
  windowMode: 'smart' | 'single'
  autoWindowLimit: number
}

export type PreviewWindowDecision =
  | { action: 'create' }
  | { action: 'reuse'; windowId: string; mode: ReuseMode; score: number }

/** Scores strong enough to prefer reuse over a new window in Smart mode. */
export const STRONG_RELEVANCE = 60

function majorType(mediaType: string | undefined): string {
  return (mediaType ?? '').split('/')[0]
}

function isParentChild(a: string | undefined, b: string | undefined, parentOfB: string | undefined, parentOfA?: string): boolean {
  if (!a || !b) return false
  if (parentOfB && parentOfB === a) return true
  if (parentOfA && parentOfA === b) return true
  return false
}

/** §13.6 relevance order, expressed as a score; ties fall to recency. */
export function relevanceScore(meta: PreviewWindowMeta, request: PreviewRequestInfo, now: number): number {
  let score = 0
  if (request.sessionId && meta.sessionId === request.sessionId) score = Math.max(score, 100)
  if (request.sessionKey && meta.sessionKey === request.sessionKey) score = Math.max(score, 95)
  if (meta.itemKeys?.includes(request.currentKey)) score = Math.max(score, 90)
  if (request.containerKey && meta.containerKey === request.containerKey) score = Math.max(score, 80)
  if (isParentChild(meta.containerKey, request.containerKey, request.parentContainerKey)) score = Math.max(score, 60)
  if (request.originApp && meta.originApp === request.originApp) {
    score = Math.max(score, request.hostContext && meta.hostContext === request.hostContext ? 45 : 30)
  }
  if (request.mediaType && meta.currentMediaType && majorType(meta.currentMediaType) === majorType(request.mediaType)) {
    score = Math.max(score, 15)
  }
  // Recency tiebreaker: up to +5 within the last hour.
  const age = Math.max(0, now - meta.lastActiveAt)
  score += Math.max(0, 5 - age / (60 * 60 * 1000) * 5)
  return score
}

function reuseMode(meta: PreviewWindowMeta, request: PreviewRequestInfo, score: number): ReuseMode {
  const sameSession =
    (request.sessionId && meta.sessionId === request.sessionId) ||
    (request.sessionKey && meta.sessionKey === request.sessionKey) ||
    meta.itemKeys?.includes(request.currentKey)
  if (sameSession && score >= 90) return 'jump'
  // A single item from the same host can join an explicit list the window already shows.
  if ((!request.sessionKind || request.sessionKind === 'single') && meta.sessionKind === 'list' && request.originApp && meta.originApp === request.originApp) {
    return 'append'
  }
  return 'replace'
}

export function chooseWindow(
  windows: PreviewWindowMeta[],
  request: PreviewRequestInfo,
  policy: PreviewWindowPolicy,
  now: number,
): PreviewWindowDecision {
  const autoWindows = windows.filter((w) => w.createdBy === 'auto')
  const candidates = autoWindows.filter((w) => w.allowAutoReuse && !w.pinned)

  if (policy.windowMode === 'single') {
    // One main window: the oldest automatic window; everything else stays untouched.
    const main = [...candidates].sort((a, b) => a.createdAt - b.createdAt)[0]
    if (!main) return { action: 'create' }
    const score = relevanceScore(main, request, now)
    return { action: 'reuse', windowId: main.windowId, mode: reuseMode(main, request, score), score }
  }

  const scored = candidates
    .map((meta) => ({ meta, score: relevanceScore(meta, request, now) }))
    .sort((a, b) => b.score - a.score || b.meta.lastActiveAt - a.meta.lastActiveAt)
  const best = scored[0]

  if (best && best.score >= STRONG_RELEVANCE) {
    return { action: 'reuse', windowId: best.meta.windowId, mode: reuseMode(best.meta, request, best.score), score: best.score }
  }
  if (autoWindows.length < Math.max(1, policy.autoWindowLimit)) {
    return { action: 'create' }
  }
  // At the cap: the most relevant reusable automatic window, else the most recent one.
  const fallback = best ?? null
  if (!fallback) {
    // Every automatic window is pinned or protected — the cap still holds (§13.5).
    const recent = [...autoWindows].sort((a, b) => b.lastActiveAt - a.lastActiveAt)[0]
    return recent ? { action: 'reuse', windowId: recent.windowId, mode: 'replace', score: 0 } : { action: 'create' }
  }
  return { action: 'reuse', windowId: fallback.meta.windowId, mode: reuseMode(fallback.meta, request, fallback.score), score: fallback.score }
}
