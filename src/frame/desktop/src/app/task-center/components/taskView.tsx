/* ── TaskCenter: status dot and progress meter ── */

import type { Tone } from './taskModel'

export function StatusDot({ tone }: { tone: Tone }) {
  return <span className="tc-dot" data-tone={tone} aria-hidden />
}

export function Meter({ value, tone, size }: { value: number; tone: Tone; size?: 'lg' }) {
  const clamped = Math.max(0, Math.min(100, value))
  return (
    <span
      className="tc-meter"
      data-tone={tone}
      data-size={size}
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={clamped}
    >
      <span style={{ '--tc-progress': clamped / 100 } as React.CSSProperties} />
    </span>
  )
}
