/* Progress of a TaskMgr 2.0 task, shared by the Task Center and the status-bar tray. */

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function asNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

/**
 * 2.0 progress is free-form JSON owned by the runner, so pull a percentage
 * out of the shapes the platform runners actually write and give up rather
 * than guess. `TaskDataProgress` (`{items|bytes: {completed, total}}`,
 * items first) is the platform's own shape.
 */
export function toProgressPercent(progress: unknown): number | null {
  const clamp = (value: number) => Math.max(0, Math.min(100, Math.round(value)))
  const direct = asNumber(progress)
  if (direct !== null) return clamp(direct <= 1 ? direct * 100 : direct)
  if (!isRecord(progress)) return null

  const percent = asNumber(progress.percent) ?? asNumber(progress.percentage)
  if (percent !== null) return clamp(percent)

  const ratio = asNumber(progress.ratio)
  if (ratio !== null) return clamp(ratio * 100)

  const completed = asNumber(progress.completed) ?? asNumber(progress.completed_items)
  const total = asNumber(progress.total) ?? asNumber(progress.total_items)
  if (completed !== null && total !== null && total > 0) return clamp((completed / total) * 100)

  for (const counter of [progress.items, progress.bytes]) {
    if (!isRecord(counter)) continue
    const done = asNumber(counter.completed)
    const of = asNumber(counter.total)
    if (done !== null && of !== null && of > 0) return clamp((done / of) * 100)
  }
  return null
}
