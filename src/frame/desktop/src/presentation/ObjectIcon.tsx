/* ── Object icons: the first loadable candidate image, else the kind's default ── */

import clsx from 'clsx'
import { useState, type CSSProperties, type ReactNode } from 'react'
import { initialOf } from './model'

/** Renders the first candidate that has not failed to load; a new URL gets a fresh try. */
export function ObjectImage({
  icons,
  alt = '',
  className,
  style,
  fallback,
}: {
  icons: readonly (string | null | undefined)[]
  alt?: string
  className?: string
  style?: CSSProperties
  fallback: ReactNode
}) {
  const [failed, setFailed] = useState<readonly string[]>([])
  const src = icons.find((icon): icon is string => Boolean(icon) && !failed.includes(icon as string))
  if (!src) return <>{fallback}</>
  return (
    <img
      key={src}
      src={src}
      alt={alt}
      draggable={false}
      className={className}
      style={style}
      onError={() => setFailed((previous) => [...previous, src])}
    />
  )
}

/** A round (people, Agents) or rounded-square (Zone, apps) icon with a loading and fallback state. */
export function ObjectAvatar({
  title,
  icons,
  size,
  shape = 'circle',
  fallbackIcon,
  tone = 'var(--cp-accent)',
  loading = false,
  decorative = false,
  className,
  testId,
}: {
  title: string
  icons: readonly (string | null | undefined)[]
  size: number
  shape?: 'circle' | 'square'
  /** Default glyph when no image loads; the title's initial otherwise. */
  fallbackIcon?: ReactNode
  tone?: string
  loading?: boolean
  /** Next to a visible name the image adds nothing for screen readers. */
  decorative?: boolean
  className?: string
  testId?: string
}) {
  const radius = shape === 'circle' ? '9999px' : `${Math.round(size * 0.24)}px`
  const box: CSSProperties = { width: size, height: size, borderRadius: radius }
  const alt = decorative ? '' : title
  const fallback = loading ? (
    <span
      aria-hidden="true"
      className="block animate-pulse"
      style={{ ...box, background: 'color-mix(in srgb, var(--cp-muted) 18%, var(--cp-surface))' }}
    />
  ) : (
    <span
      role={decorative ? undefined : 'img'}
      aria-label={decorative ? undefined : title}
      aria-hidden={decorative ? true : undefined}
      className="flex items-center justify-center font-display font-semibold"
      style={{
        ...box,
        fontSize: Math.round(size * 0.4),
        background: `color-mix(in srgb, ${tone} 16%, var(--cp-surface))`,
        color: `color-mix(in srgb, ${tone} 80%, var(--cp-text))`,
      }}
    >
      {fallbackIcon ?? initialOf(title)}
    </span>
  )
  return (
    <span className={clsx('relative inline-flex shrink-0', className)} style={box} data-testid={testId}>
      <ObjectImage
        icons={icons}
        alt={alt}
        className="block object-cover"
        style={box}
        fallback={fallback}
      />
    </span>
  )
}
