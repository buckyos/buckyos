import { useI18n } from '../../i18n/provider'
import { useMinuteClock } from '../shell'
import type { DesktopWidgetProps } from './types'

export function ClockWidget(_: DesktopWidgetProps) {
  const { locale } = useI18n()
  const now = useMinuteClock()

  return (
    <div className="flex h-full flex-col justify-between rounded-[22px] bg-[linear-gradient(180deg,color-mix(in_srgb,var(--cp-surface-3)_100%,transparent),color-mix(in_srgb,var(--cp-surface-2)_96%,transparent))] p-4">
      <div className="flex justify-end">
        <span className="rounded-full bg-[color:color-mix(in_srgb,var(--cp-surface)_70%,transparent)] px-2.5 py-1 text-[11px] font-medium text-[color:var(--cp-muted)]">
          {new Intl.DateTimeFormat(locale, {
            weekday: 'short',
          }).format(now)}
        </span>
      </div>
      <div className="-mt-1">
        <p className="font-display whitespace-nowrap text-[1.72rem] font-semibold leading-[0.94] tracking-[-0.05em] text-[color:var(--cp-text)] sm:text-[2.8rem] lg:text-5xl">
          {new Intl.DateTimeFormat(locale, {
            hour: '2-digit',
            minute: '2-digit',
          }).format(now)}
        </p>
        <p className="mt-1 text-sm text-[color:var(--cp-muted)]">
          {new Intl.DateTimeFormat(locale, {
            month: 'short',
            day: 'numeric',
            year: 'numeric',
          }).format(now)}
        </p>
      </div>
    </div>
  )
}
