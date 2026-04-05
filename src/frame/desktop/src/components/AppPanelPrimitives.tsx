import clsx from 'clsx'
import type { ReactNode } from 'react'
import { panelToneClasses } from './DesktopVisualTokens'

export function PanelIntro({
  aside,
  body,
  kicker,
  title,
}: {
  aside?: ReactNode
  body: string
  kicker?: string
  title: string
}) {
  return (
    <section className="shell-panel px-5 py-5 sm:px-6">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div className="max-w-xl">
          {kicker ? <p className="shell-kicker">{kicker}</p> : null}
          <p className="mt-2 font-display text-xl font-semibold sm:text-2xl">{title}</p>
          <p className="mt-2 text-sm leading-6 text-[color:var(--cp-muted)]">{body}</p>
        </div>
        {aside ? <div className="hidden sm:block sm:shrink-0">{aside}</div> : null}
      </div>
    </section>
  )
}

export function MetricCard({
  label,
  tone = 'neutral',
  value,
}: {
  label: string
  tone?: keyof typeof panelToneClasses
  value: ReactNode
}) {
  return (
    <div className="shell-subtle-panel px-4 py-4">
      <div
        className={clsx(
          'mb-3 inline-flex rounded-full px-2.5 py-1 text-[11px] font-semibold uppercase tracking-[0.18em]',
          panelToneClasses[tone],
        )}
      >
        {label}
      </div>
      <div className="font-display text-2xl font-semibold text-[color:var(--cp-text)]">
        {value}
      </div>
    </div>
  )
}

export function DemoSection({
  body,
  children,
  title,
}: {
  body: string
  children: ReactNode
  title: string
}) {
  return (
    <section className="shell-subtle-panel p-4 sm:p-5">
      <div className="max-w-2xl">
        <p className="font-display text-lg font-semibold text-[color:var(--cp-text)]">
          {title}
        </p>
        <p className="mt-1 text-sm leading-6 text-[color:var(--cp-muted)]">{body}</p>
      </div>
      <div className="mt-4">{children}</div>
    </section>
  )
}
