/* eslint-disable react-refresh/only-export-components -- the forms with their draft helpers */
/* The calendar's forms (控件机制 TODO G5, G10): quick create (type a title, Enter), the full event form (title, all
 * day, start / end, repeat, location, colour, reminders, which calendar) and the repeat editor — the choices of a
 * mainstream calendar's "repeat" menu, and "custom": every n days / weeks on given days / months on a day or the n-th
 * (last) weekday / years, ending never, on a date or after n times. The forms only collect; the control turns the
 * result into operations (buckyos-websdk aiws_schedule). */

import { useMemo, useState } from 'react'
import { describeRRule, formatRRule, parseRRule, recurrencePresets, type RRule } from 'buckyos'
import { aiwsText } from '../../../i18n'
import { useAiwsText } from '../../../useAiwsText'
import { CALENDAR_COLORS, dateOfDay, dayOfDate, localDayOf, localInstant, minuteOf } from './model'

export interface EventDraft {
  title: string
  allDay: boolean
  startDate: string
  endDate: string
  /** Minutes of the day (timed). */
  startMin: number
  endMin: number
  rrule: string | null
  location: string
  color: string | null
  reminders: number[]
  calendar: string
}

const pad = (n: number) => String(n).padStart(2, '0')
export const hhmm = (min: number) => `${pad(Math.floor(min / 60) % 24)}:${pad(min % 60)}`
const parseHHMM = (s: string) => { const m = /^(\d{1,2}):(\d{2})$/.exec(s); return m ? Math.min(23, +m[1]) * 60 + Math.min(59, +m[2]) : null }

export const locale = (): 'zh' | 'en' => (aiwsText().locale === 'zh-CN' ? 'zh' : 'en')

/** A draft's times as instants (timed) or dates (all-day). */
export function draftTimes(d: EventDraft): { allDay: true; startDate: string; endDate: string } | { allDay: false; start: string; end: string } {
  if (d.allDay) return { allDay: true, startDate: d.startDate, endDate: d.endDate < d.startDate ? d.startDate : d.endDate }
  const s = localInstant(dayOfDate(d.startDate), d.startMin)
  let e = localInstant(dayOfDate(d.endDate), d.endMin)
  if (e < s) e = s + 30 * 60_000
  return { allDay: false, start: new Date(s).toISOString(), end: new Date(e).toISOString() }
}

/** A draft from an event's values. */
export function draftOf(values: { title: string; allDay: boolean; startMs?: number; endMs?: number; startDate?: string; endDate?: string; ext: Record<string, unknown> }, calendar: string): EventDraft {
  const ext = values.ext
  const reminders = Array.isArray(ext.reminders) ? ext.reminders.map((r) => (r && typeof r === 'object' ? Number((r as { before_min?: unknown }).before_min) : NaN)).filter((n) => Number.isFinite(n)) : []
  const base = { title: values.title, rrule: typeof ext.rrule === 'string' ? ext.rrule : null, location: typeof ext.location === 'string' ? ext.location : '', color: typeof ext.color === 'string' ? ext.color : null, reminders, calendar }
  if (values.allDay) return { ...base, allDay: true, startDate: values.startDate!, endDate: values.endDate!, startMin: 9 * 60, endMin: 10 * 60 }
  const sd = localDayOf(values.startMs!), ed = localDayOf(values.endMs!)
  return { ...base, allDay: false, startDate: dateOfDay(sd), endDate: dateOfDay(ed), startMin: minuteOf(values.startMs!, sd), endMin: minuteOf(values.endMs!, ed) }
}

const REMINDER_CHOICES = [0, 5, 10, 15, 30, 60, 120, 1440, 2880, 10080]

export function reminderLabel(min: number): string {
  const { t } = aiwsText()
  if (min === 0) return t('aiworkspace.calendar.reminder.atStart')
  if (min % 10080 === 0) return t('aiworkspace.calendar.reminder.weeks', { n: min / 10080 })
  if (min % 1440 === 0) return t('aiworkspace.calendar.reminder.days', { n: min / 1440 })
  if (min % 60 === 0) return t('aiworkspace.calendar.reminder.hours', { n: min / 60 })
  return t('aiworkspace.calendar.reminder.minutes', { n: min })
}

/** The full form. `calendars`: the bound calendars a new event may go to. */
export function EventForm({ initial, calendars, isNew, disabled, onSave, onCancel }: {
  initial: EventDraft; calendars: { id: string; label: string; color: string }[]; isNew: boolean; disabled: string | null
  onSave: (draft: EventDraft) => void; onCancel: () => void
}) {
  const { t } = useAiwsText()
  const [d, setD] = useState(initial)
  const set = (patch: Partial<EventDraft>) => setD((x) => ({ ...x, ...patch }))
  const [custom, setCustom] = useState(false)
  const presets = useMemo(() => recurrencePresets(d.startDate, locale()), [d.startDate])
  const preset = presets.find((p) => p.rrule === d.rrule)
  const ruleText = useMemo(() => { try { return d.rrule ? describeRRule(parseRRule(d.rrule), locale()) : null } catch { return null } }, [d.rrule])
  const valid = d.title.trim() !== '' && (d.allDay ? d.endDate >= d.startDate : `${d.endDate}${hhmm(d.endMin)}` >= `${d.startDate}${hhmm(d.startMin)}`)
  return (
    <form className="aiws-ctl-form" data-testid="aiws-cal-form" onSubmit={(e) => { e.preventDefault(); if (valid && !disabled) onSave(d) }}>
      <input className="aiws-ctl-form-title" autoFocus={isNew} value={d.title} placeholder={t('aiworkspace.calendar.titlePlaceholder')} aria-label={t('aiworkspace.calendar.field.title')}
        data-testid="aiws-cal-form-title" onChange={(e) => set({ title: e.target.value })} />
      <label className="aiws-ctl-form-row">
        <span>{t('aiworkspace.calendar.field.allDay')}</span>
        <span><input type="checkbox" checked={d.allDay} data-testid="aiws-cal-form-allday" onChange={(e) => set({ allDay: e.target.checked })} /></span>
      </label>
      <div className="aiws-ctl-form-row">
        <span>{t('aiworkspace.calendar.field.start')}</span>
        <span className="aiws-ctl-form-pair">
          <input type="date" value={d.startDate} aria-label={t('aiworkspace.calendar.field.startDate')} data-testid="aiws-cal-form-start-date"
            onChange={(e) => {
              if (!e.target.value) return
              // the end moves with the start (the length is kept)
              const shift = dayOfDate(e.target.value) - dayOfDate(d.startDate)
              set({ startDate: e.target.value, endDate: dateOfDay(dayOfDate(d.endDate) + shift) })
            }} />
          {!d.allDay && <input type="time" step={300} value={hhmm(d.startMin)} aria-label={t('aiworkspace.calendar.field.startTime')} data-testid="aiws-cal-form-start-time"
            onChange={(e) => { const m = parseHHMM(e.target.value); if (m !== null) set({ startMin: m, endMin: Math.min(24 * 60 - 1, m + (d.endMin - d.startMin)) }) }} />}
        </span>
      </div>
      <div className="aiws-ctl-form-row">
        <span>{t('aiworkspace.calendar.field.end')}</span>
        <span className="aiws-ctl-form-pair">
          <input type="date" value={d.endDate} min={d.startDate} aria-label={t('aiworkspace.calendar.field.endDate')} onChange={(e) => e.target.value && set({ endDate: e.target.value })} />
          {!d.allDay && <input type="time" step={300} value={hhmm(d.endMin)} aria-label={t('aiworkspace.calendar.field.endTime')} data-testid="aiws-cal-form-end-time"
            onChange={(e) => { const m = parseHHMM(e.target.value); if (m !== null) set({ endMin: m }) }} />}
        </span>
      </div>
      <div className="aiws-ctl-form-row">
        <span>{t('aiworkspace.calendar.field.repeat')}</span>
        <span className="aiws-ctl-form-col">
          <select value={custom ? '__custom' : preset ? preset.id : d.rrule ? '__current' : 'none'} data-testid="aiws-cal-form-repeat"
            onChange={(e) => {
              if (e.target.value === '__custom') { setCustom(true); if (!d.rrule) set({ rrule: 'FREQ=WEEKLY' }); return }
              setCustom(false)
              if (e.target.value === '__current') return
              set({ rrule: presets.find((p) => p.id === e.target.value)?.rrule ?? null })
            }}>
            {presets.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
            {d.rrule && !preset && <option value="__current">{ruleText ?? d.rrule}</option>}
            <option value="__custom">{t('aiworkspace.calendar.repeat.custom')}</option>
          </select>
          {custom && d.rrule && <RecurrenceEditor rrule={d.rrule} startDate={d.startDate} onChange={(rrule) => set({ rrule })} />}
          {d.rrule && ruleText && <span className="aiws-dc-note" data-testid="aiws-cal-form-rule">{ruleText}</span>}
        </span>
      </div>
      <label className="aiws-ctl-form-row">
        <span>{t('aiworkspace.calendar.field.location')}</span>
        <input value={d.location} onChange={(e) => set({ location: e.target.value })} />
      </label>
      <div className="aiws-ctl-form-row">
        <span>{t('aiworkspace.calendar.field.reminders')}</span>
        <span className="aiws-ctl-form-col">
          {d.reminders.map((r, i) => (
            <span key={i} className="aiws-ctl-form-pair">
              <select value={r} onChange={(e) => set({ reminders: d.reminders.map((x, j) => (j === i ? Number(e.target.value) : x)) })}>
                {[...new Set([...REMINDER_CHOICES, r])].sort((a, b) => a - b).map((m) => <option key={m} value={m}>{reminderLabel(m)}</option>)}
              </select>
              <button type="button" className="aiws-link" onClick={() => set({ reminders: d.reminders.filter((_, j) => j !== i) })}>{t('aiworkspace.calendar.remove')}</button>
            </span>
          ))}
          {d.reminders.length < 16 && <button type="button" className="aiws-link" onClick={() => set({ reminders: [...d.reminders, d.allDay ? 1440 : 10] })}>{t('aiworkspace.calendar.addReminder')}</button>}
        </span>
      </div>
      <div className="aiws-ctl-form-row">
        <span>{t('aiworkspace.calendar.field.color')}</span>
        <span className="aiws-cal-colors" role="radiogroup" aria-label={t('aiworkspace.calendar.field.color')}>
          <button type="button" role="radio" aria-checked={d.color === null} className={`aiws-cal-swatch is-auto${d.color === null ? ' is-on' : ''}`} title={t('aiworkspace.calendar.colorAuto')} onClick={() => set({ color: null })} />
          {CALENDAR_COLORS.map((c) => <button key={c} type="button" role="radio" aria-checked={d.color === c} className={`aiws-cal-swatch${d.color === c ? ' is-on' : ''}`} style={{ background: c }} title={c} onClick={() => set({ color: c })} />)}
        </span>
      </div>
      {isNew && calendars.length > 1 && (
        <label className="aiws-ctl-form-row">
          <span>{t('aiworkspace.calendar.field.calendar')}</span>
          <select value={d.calendar} onChange={(e) => set({ calendar: e.target.value })}>{calendars.map((c) => <option key={c.id} value={c.id}>{c.label}</option>)}</select>
        </label>
      )}
      <div className="aiws-ctl-form-actions">
        {disabled && <span className="aiws-dc-note">{disabled}</span>}
        <button type="button" onClick={onCancel}>{t('aiworkspace.common.cancel')}</button>
        <button type="submit" className="aiws-primary" disabled={!valid || Boolean(disabled)} data-testid="aiws-cal-form-save">{t('aiworkspace.calendar.save')}</button>
      </div>
    </form>
  )
}

const WD_KEYS = ['MO', 'TU', 'WE', 'TH', 'FR', 'SA', 'SU'] as const
const WD_LABEL = [
  'aiworkspace.calendar.wd.mo', 'aiworkspace.calendar.wd.tu', 'aiworkspace.calendar.wd.we', 'aiworkspace.calendar.wd.th',
  'aiworkspace.calendar.wd.fr', 'aiworkspace.calendar.wd.sa', 'aiworkspace.calendar.wd.su',
]
const FREQ_LABEL = { DAILY: 'aiworkspace.calendar.freq.daily', WEEKLY: 'aiworkspace.calendar.freq.weekly', MONTHLY: 'aiworkspace.calendar.freq.monthly', YEARLY: 'aiworkspace.calendar.freq.yearly' } as const

/** "Custom" repeat: what mainstream calendars offer under it, written as an RRULE of the supported subset. */
export function RecurrenceEditor({ rrule, startDate, onChange }: { rrule: string; startDate: string; onChange: (rrule: string) => void }) {
  const { t } = useAiwsText()
  let rule: RRule
  try { rule = parseRRule(rrule) } catch { rule = { freq: 'WEEKLY', interval: 1, wkst: 0 } }
  const day = dayOfDate(startDate)
  const wd = (((day + 3) % 7) + 7) % 7
  const dom = +startDate.slice(8, 10)
  const nth = Math.floor((dom - 1) / 7) + 1
  const emit = (next: RRule) => onChange(formatRRule(next))
  const ends: 'never' | 'until' | 'count' = rule.count !== undefined ? 'count' : rule.until !== undefined ? 'until' : 'never'
  const monthlyMode = rule.byDay?.length ? (rule.byDay[0].ordinal === -1 ? 'last' : 'nth') : 'day'
  return (
    <div className="aiws-cal-rrule" data-testid="aiws-cal-rrule">
      <span className="aiws-ctl-form-pair">
        {t('aiworkspace.calendar.repeat.every')}
        <input type="number" min={1} max={999} value={rule.interval} style={{ width: 56 }} aria-label={t('aiworkspace.calendar.repeat.interval')}
          onChange={(e) => emit({ ...rule, interval: Math.max(1, Math.min(999, Number(e.target.value) || 1)) })} />
        <select value={rule.freq} aria-label={t('aiworkspace.calendar.repeat.unit')}
          onChange={(e) => {
            const freq = e.target.value as RRule['freq']
            const base: RRule = { freq, interval: rule.interval, wkst: rule.wkst, count: rule.count, until: rule.until }
            if (freq === 'WEEKLY') base.byDay = [{ weekday: wd }]
            if (freq === 'MONTHLY') base.byMonthDay = [dom]
            emit(base)
          }}>
          {(['DAILY', 'WEEKLY', 'MONTHLY', 'YEARLY'] as const).map((f) => <option key={f} value={f}>{t(FREQ_LABEL[f])}</option>)}
        </select>
      </span>
      {rule.freq === 'WEEKLY' && (
        <span className="aiws-cal-weekdays" role="group" aria-label={t('aiworkspace.calendar.repeat.on')}>
          {WD_KEYS.map((k, i) => {
            const on = Boolean(rule.byDay?.some((d) => d.weekday === i))
            return <button key={k} type="button" aria-pressed={on} className={on ? 'is-on' : undefined} data-testid={`aiws-cal-wd-${k}`}
              onClick={() => {
                const days = (rule.byDay ?? [{ weekday: wd }]).filter((d) => d.weekday !== i)
                const next = on ? days : [...days, { weekday: i }].sort((a, b) => a.weekday - b.weekday)
                emit({ ...rule, byDay: next.length ? next : [{ weekday: wd }] })
              }}>{t(WD_LABEL[i])}</button>
          })}
        </span>
      )}
      {rule.freq === 'MONTHLY' && (
        <select value={monthlyMode} aria-label={t('aiworkspace.calendar.repeat.on')}
          onChange={(e) => {
            const v = e.target.value
            const { byDay: _d, byMonthDay: _m, bySetPos: _s, ...rest } = rule
            void _d; void _m; void _s
            if (v === 'day') emit({ ...rest, byMonthDay: [dom] })
            else emit({ ...rest, byDay: [{ weekday: wd, ordinal: v === 'last' ? -1 : Math.min(nth, 4) }] })
          }}>
          <option value="day">{t('aiworkspace.calendar.repeat.monthDay', { d: dom })}</option>
          <option value="nth">{t('aiworkspace.calendar.repeat.monthNth', { n: Math.min(nth, 4), wd: t(WD_LABEL[wd]) })}</option>
          <option value="last">{t('aiworkspace.calendar.repeat.monthLast', { wd: t(WD_LABEL[wd]) })}</option>
        </select>
      )}
      <span className="aiws-ctl-form-pair">
        {t('aiworkspace.calendar.repeat.ends')}
        <select value={ends} onChange={(e) => {
          const { count: _c, until: _u, ...rest } = rule
          void _c; void _u
          if (e.target.value === 'count') emit({ ...rest, count: 10 })
          else if (e.target.value === 'until') emit({ ...rest, until: dateOfDay(day + 90) })
          else emit(rest)
        }}>
          <option value="never">{t('aiworkspace.calendar.repeat.never')}</option>
          <option value="until">{t('aiworkspace.calendar.repeat.onDate')}</option>
          <option value="count">{t('aiworkspace.calendar.repeat.afterN')}</option>
        </select>
        {ends === 'until' && <input type="date" value={rule.until!.slice(0, 10)} min={startDate} onChange={(e) => e.target.value && emit({ ...rule, until: e.target.value })} />}
        {ends === 'count' && <input type="number" min={1} max={1000} value={rule.count} style={{ width: 64 }} onChange={(e) => emit({ ...rule, count: Math.max(1, Math.min(1000, Number(e.target.value) || 1)) })} />}
      </span>
    </div>
  )
}

/** Quick create: a title for the time picked on the grid; "More options" opens the full form. */
export function QuickCreate({ summary, onCreate, onMore, disabled }: { summary: string; onCreate: (title: string) => void; onMore: (title: string) => void; disabled: string | null }) {
  const { t } = useAiwsText()
  const [title, setTitle] = useState('')
  return (
    <form className="aiws-ctl-form" data-testid="aiws-cal-quick" onSubmit={(e) => { e.preventDefault(); if (title.trim() && !disabled) onCreate(title.trim()) }}>
      <input className="aiws-ctl-form-title" autoFocus value={title} placeholder={t('aiworkspace.calendar.titlePlaceholder')} disabled={Boolean(disabled)} aria-label={t('aiworkspace.calendar.field.title')}
        data-testid="aiws-cal-quick-title" onChange={(e) => setTitle(e.target.value)} />
      <div className="aiws-dc-note">{summary}</div>
      {disabled && <div className="aiws-dc-note">{disabled}</div>}
      <div className="aiws-ctl-form-actions">
        <button type="button" onClick={() => onMore(title)} data-testid="aiws-cal-quick-more">{t('aiworkspace.calendar.moreOptions')}</button>
        <button type="submit" className="aiws-primary" disabled={!title.trim() || Boolean(disabled)} data-testid="aiws-cal-quick-save">{t('aiworkspace.calendar.save')}</button>
      </div>
    </form>
  )
}

/** "Only this one / this and following / all": asked before a change of a repeating event is written. */
export function ScopeAsk({ onPick, verb }: { onPick: (scope: 'one' | 'following' | 'all') => void; verb: 'edit' | 'delete' }) {
  const { t } = useAiwsText()
  return (
    <div className="aiws-cal-scope" data-testid="aiws-cal-scope">
      <div className="aiws-cal-scope-q">{verb === 'delete' ? t('aiworkspace.calendar.scope.deleteQ') : t('aiworkspace.calendar.scope.editQ')}</div>
      <button type="button" data-testid="aiws-cal-scope-one" onClick={() => onPick('one')}>{t('aiworkspace.calendar.scope.one')}</button>
      <button type="button" data-testid="aiws-cal-scope-following" onClick={() => onPick('following')}>{t('aiworkspace.calendar.scope.following')}</button>
      <button type="button" data-testid="aiws-cal-scope-all" onClick={() => onPick('all')}>{t('aiworkspace.calendar.scope.all')}</button>
    </div>
  )
}

/** A small month to jump to a date (the standard date editor of the control's pop-overs). */
export function MiniMonth({ value, today, weekStart, onPick }: { value: string; today: string; weekStart: number; onPick: (date: string) => void }) {
  const { t } = useAiwsText()
  const [month, setMonth] = useState(value.slice(0, 7))
  const y = +month.slice(0, 4), m = +month.slice(5, 7)
  const first = Math.floor(Date.UTC(y, m - 1, 1) / 86_400_000)
  const start = first - ((((first + 3) % 7) + 7) % 7 - weekStart + 7) % 7
  const title = new Intl.DateTimeFormat(locale() === 'zh' ? 'zh-CN' : 'en', { year: 'numeric', month: 'long', timeZone: 'UTC' }).format(new Date(first * 86_400_000))
  const shift = (n: number) => { const d = new Date(Date.UTC(y, m - 1 + n, 1)); setMonth(`${d.getUTCFullYear()}-${String(d.getUTCMonth() + 1).padStart(2, '0')}`) }
  return (
    <div className="aiws-cal-mini" data-testid="aiws-cal-mini">
      <div className="aiws-cal-mini-head">
        <button type="button" aria-label={t('aiworkspace.calendar.prev')} onClick={() => shift(-1)}>‹</button>
        <span>{title}</span>
        <button type="button" aria-label={t('aiworkspace.calendar.next')} onClick={() => shift(1)}>›</button>
      </div>
      <div className="aiws-cal-mini-grid">
        {Array.from({ length: 7 }, (_, i) => <span key={`h${i}`} className="aiws-cal-mini-wd">{t(WD_LABEL[(weekStart + i) % 7])}</span>)}
        {Array.from({ length: 42 }, (_, i) => {
          const date = dateOfDay(start + i)
          const out = date.slice(0, 7) !== month
          return <button key={date} type="button" className={`${out ? 'is-out' : ''}${date === today ? ' is-today' : ''}${date === value ? ' is-on' : ''}`} data-date={date} onClick={() => onPick(date)}>{+date.slice(8)}</button>
        })}
      </div>
    </div>
  )
}
