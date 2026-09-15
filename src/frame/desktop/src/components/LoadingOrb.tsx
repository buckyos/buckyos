import clsx from 'clsx'
import type { CSSProperties } from 'react'

/**
 * Loading indicator used while an app panel (or the desktop itself) is being
 * fetched: a glossy orb with a conventional spinner arc rotating around it
 * and a soft halo that fades in and out.
 *
 * Styles live in `index.css` (`cp-orb-*`). The spinner keeps rotating under
 * `prefers-reduced-motion` (only the decorative float/pulse are dropped),
 * because a stalled indicator reads as "frozen", not as "loading".
 */
export function LoadingOrb({
  size = 56,
  className,
  label,
}: {
  /** Diameter of the core orb in px; the ring and halo scale with it. */
  size?: number
  className?: string
  /** Accessible label; defaults to a generic "Loading". */
  label?: string
}) {
  return (
    <div
      role="status"
      aria-label={label ?? 'Loading'}
      aria-live="polite"
      className={clsx('cp-orb', className)}
      style={{ '--cp-orb-size': `${size}px` } as CSSProperties}
    >
      <span className="cp-orb__halo" aria-hidden="true" />
      <span className="cp-orb__ring" aria-hidden="true" />
      <span className="cp-orb__core" aria-hidden="true" />
    </div>
  )
}
