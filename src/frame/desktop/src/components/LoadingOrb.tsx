import clsx from 'clsx'
import type { CSSProperties } from 'react'

/**
 * Loading indicator used while an app panel (or the desktop itself) is being
 * fetched: a thin ring with one accent arc.
 *
 * Styles live in `index.css` (`cp-orb-*`). The ring keeps rotating under
 * `prefers-reduced-motion`, because a stalled indicator reads as "frozen",
 * not as "loading".
 */
export function LoadingOrb({
  size = 56,
  className,
  label,
}: {
  /** Size of the indicator box in px; the ring scales with it. */
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
      <span className="cp-orb__ring" aria-hidden="true" />
    </div>
  )
}
