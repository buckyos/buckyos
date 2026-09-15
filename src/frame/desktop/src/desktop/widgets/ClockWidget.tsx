import { useMemo } from 'react'
import { useI18n } from '../../i18n/provider'
import { useMinuteClock } from '../shell'

/**
 * Clock widget. Layout is driven by container queries (see `.cp-clock` in
 * index.css): in a single-row cell the time sits left with weekday and date
 * stacked on the right; with two rows or more the weekday chip moves to the
 * top-right and the time/date stack underneath.
 */
export function ClockWidget() {
  const { locale } = useI18n()
  const now = useMinuteClock()
  // Intl.DateTimeFormat construction is comparatively expensive; build the
  // three formatters once per locale instead of on every render.
  const formatters = useMemo(
    () => ({
      weekday: new Intl.DateTimeFormat(locale, { weekday: 'short' }),
      time: new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit' }),
      date: new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric', year: 'numeric' }),
    }),
    [locale],
  )

  return (
    <div className="cp-clock rounded-[22px] bg-[linear-gradient(180deg,color-mix(in_srgb,var(--cp-surface-3)_100%,transparent),color-mix(in_srgb,var(--cp-surface-2)_96%,transparent))]">
      <p className="cp-clock__time font-display font-semibold text-[color:var(--cp-text)]">
        {formatters.time.format(now)}
      </p>
      <div className="cp-clock__meta">
        <span className="cp-clock__weekday rounded-full bg-[color:color-mix(in_srgb,var(--cp-surface)_70%,transparent)] font-medium text-[color:var(--cp-muted)]">
          {formatters.weekday.format(now)}
        </span>
        <p className="cp-clock__date text-[color:var(--cp-muted)]">
          {formatters.date.format(now)}
        </p>
      </div>
    </div>
  )
}
