/* ── Local behaviour events (PRD FR-FEEDBACK-002). Never stores raw data or full prompts. ── */

export type CanvasEventName =
  | 'canvas_created'
  | 'sample_opened'
  | 'file_imported'
  | 'wish_created'
  | 'wish_run_started'
  | 'wish_run_succeeded'
  | 'wish_run_failed'
  | 'wish_run_cancelled'
  | 'result_edited'
  | 'result_refreshed'
  | 'result_detached'
  | 'presentation_created'
  | 'presentation_played'
  | 'undo_used'
  | 'feedback_submitted'
  | 'onboarding_completed'
  | 'onboarding_skipped'

const KEY = 'aicanvas.events'
const MAX = 2000

export interface CanvasEvent {
  name: CanvasEventName
  at: string
  props?: Record<string, string | number | boolean>
}

function read(): CanvasEvent[] {
  try {
    const raw = localStorage.getItem(KEY)
    return raw ? (JSON.parse(raw) as CanvasEvent[]) : []
  } catch {
    return []
  }
}

export function trackEvent(name: CanvasEventName, props?: CanvasEvent['props']) {
  try {
    const list = read()
    list.push({ name, at: new Date().toISOString(), props })
    localStorage.setItem(KEY, JSON.stringify(list.slice(-MAX)))
  } catch {
    /* storage unavailable */
  }
}

export function eventCounts(): Record<string, number> {
  const counts: Record<string, number> = {}
  for (const e of read()) counts[e.name] = (counts[e.name] ?? 0) + 1
  return counts
}

export function allEvents(): CanvasEvent[] {
  return read()
}

export function firstEventAt(name: CanvasEventName): string | undefined {
  return read().find((e) => e.name === name)?.at
}
