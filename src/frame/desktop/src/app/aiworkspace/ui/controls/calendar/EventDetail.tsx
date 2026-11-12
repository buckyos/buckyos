/* eslint-disable react-refresh/only-export-components -- the details with their time text */
/* An event's details (控件机制 TODO §0.1, G5, G15): when it is, how it repeats (and which occurrences were changed or
 * excluded), where, the reminders, and — for a series an Agent keeps — how far its schedule reaches and when it last
 * succeeded (D4 §5.4: read from AIWS, the execution records stay in the task center, which is linked). The notes are the
 * record's rich-text body. Actions: edit, delete, skip this time, duplicate. */

import { useMemo } from 'react'
import { Bell, CalendarClock, Copy, Link2, MapPin, MessageSquarePlus, Pencil, Repeat, SkipForward, Trash2, TriangleAlert, Wrench } from 'lucide-react'
import { describeRRule, parseRRule } from 'buckyos'
import { randomId } from '../../../api/ids'
import { useEntity, useStore } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { RichTextCell } from '../../blocks/editors'
import { locale, reminderLabel } from './EventForm'
import type { Placed } from './model'

export function whenText(p: Placed): string {
  const loc = locale() === 'zh' ? 'zh-CN' : 'en'
  const day = new Intl.DateTimeFormat(loc, { month: 'short', day: 'numeric', weekday: 'short' })
  const time = new Intl.DateTimeFormat(loc, { hour: '2-digit', minute: '2-digit' })
  const o = p.occ
  if (o.allDay) {
    const s = new Date(`${o.startDate}T00:00:00`), e = new Date(`${o.endDate}T00:00:00`)
    return o.startDate === o.endDate ? day.format(s) : `${day.format(s)} – ${day.format(e)}`
  }
  const s = new Date(o.startMs!), e = new Date(o.endMs!)
  return p.startDay === p.endDay ? `${day.format(s)} ${time.format(s)} – ${time.format(e)}` : `${day.format(s)} ${time.format(s)} – ${day.format(e)} ${time.format(e)}`
}

export function EventDetail({ p, sourceId, canWrite, noWrite, onEdit, onDelete, onSkip, onDuplicate, onCopyLink, onAnnotate }: {
  p: Placed; sourceId: string; canWrite: boolean; noWrite: string
  onEdit: () => void; onDelete: () => void; onSkip: (() => void) | null; onDuplicate: () => void; onCopyLink: () => void; onAnnotate: (() => void) | null
}) {
  const { t, dateTime, list } = useAiwsText()
  const store = useStore()
  const o = p.occ
  const master = o.master ?? o.row
  const ext = (master.values.ext && typeof master.values.ext === 'object' ? master.values.ext : {}) as Record<string, unknown>
  const own = o.ext as Record<string, unknown>
  const rule = useMemo(() => { try { return typeof ext.rrule === 'string' ? describeRRule(parseRRule(ext.rrule), locale()) : null } catch { return null } }, [ext.rrule])
  const exdates = Array.isArray(ext.exdate) ? ext.exdate.length : 0
  const location = typeof own.location === 'string' ? own.location : null
  const reminders = Array.isArray(own.reminders) ? own.reminders.map((r) => Number((r as { before_min?: unknown })?.before_min)).filter((n) => Number.isFinite(n)) : []
  const maintenance = (ext.maintenance && typeof ext.maintenance === 'object' ? ext.maintenance : null) as Record<string, unknown> | null
  const covered = (typeof ext.covered_until === 'string' ? ext.covered_until : null) ?? (typeof maintenance?.covered_until === 'string' ? maintenance.covered_until : null)
  const bodyId = o.row.body_ref?.entity_id ?? null
  const body = useEntity(bodyId ?? '')
  const addNotes = () => {
    const id = randomId('rt')
    void store.submit({ editId: `cal:${sourceId}:${o.row.record_id}:body`, label: t('aiworkspace.calendar.addNotes'), operations: [
      { op: 'entity.create', entity_id: id, type_id: 'buckyos.richtext', parent_id: sourceId, order_key: 'n', payload: { content: { type: 'doc', content: [{ type: 'paragraph', attrs: { block_id: randomId('b') } }] } } },
      { op: 'table.set_body', source_id: sourceId, record_id: o.row.record_id, body_ref: { entity_id: id }, expect: { rev: o.row.rev ?? 0 } },
    ] })
  }
  const disabled = canWrite ? undefined : noWrite
  if (p.todo) {
    return (
      <div className="aiws-cal-detail" data-testid="aiws-cal-detail">
        <div className="aiws-cal-detail-title"><span className="aiws-cal-dot" style={{ background: p.color }} />{o.title}</div>
        <div className="aiws-cal-detail-row"><CalendarClock size={14} aria-hidden="true" />{t('aiworkspace.calendar.todoDue', { date: whenText(p) })}</div>
        <div className="aiws-dc-note">{p.todo.done ? t('aiworkspace.calendar.todoDone') : t('aiworkspace.calendar.todoOpen')}</div>
      </div>
    )
  }
  return (
    <div className="aiws-cal-detail" data-testid="aiws-cal-detail">
      <div className="aiws-cal-detail-title"><span className="aiws-cal-dot" style={{ background: p.color }} />{o.title || t('aiworkspace.calendar.untitled')}</div>
      <div className="aiws-cal-detail-row" data-testid="aiws-cal-detail-when"><CalendarClock size={14} aria-hidden="true" />{whenText(p)}</div>
      {(rule || typeof ext.rule_text === 'string') && (
        <div className="aiws-cal-detail-row" data-testid="aiws-cal-detail-rule"><Repeat size={14} aria-hidden="true" />
          <span>{rule ?? String(ext.rule_text)}{exdates ? t('aiworkspace.calendar.exceptions', { n: exdates }) : ''}{o.override ? t('aiworkspace.calendar.changedOnce') : ''}</span>
        </div>
      )}
      {o.agentWritten && <div className="aiws-cal-detail-row"><Wrench size={14} aria-hidden="true" />{t('aiworkspace.calendar.agentWritten')}</div>}
      {location && <div className="aiws-cal-detail-row"><MapPin size={14} aria-hidden="true" />{location}</div>}
      {reminders.length > 0 && <div className="aiws-cal-detail-row"><Bell size={14} aria-hidden="true" />{list(reminders.map(reminderLabel))}</div>}
      {(covered || maintenance) && (
        <div className="aiws-cal-maint" data-testid="aiws-cal-maintenance">
          {covered && <div>{t('aiworkspace.calendar.coveredUntil', { at: covered.length === 10 ? covered : dateTime(covered) })}</div>}
          {typeof maintenance?.last_success_at === 'string' && <div>{t('aiworkspace.calendar.lastSuccess', { at: dateTime(maintenance.last_success_at) })}</div>}
          {typeof maintenance?.last_error === 'string' && <div className="aiws-cal-maint-error"><TriangleAlert size={13} aria-hidden="true" />{maintenance.last_error}</div>}
          {typeof maintenance?.maintainer === 'string' && <div className="aiws-dc-note">{t('aiworkspace.calendar.maintainer', { who: maintenance.maintainer })}</div>}
          <a className="aiws-link" href="/taskmgr" target="_blank" rel="noreferrer">{t('aiworkspace.calendar.openTasks')}</a>
        </div>
      )}
      <div className="aiws-ctl-form-notes">
        {bodyId ? <div className="aiws-ctl-form-body"><RichTextCell entity={body} entityId={bodyId} renderEmbed={() => null} /></div>
          : canWrite && <button type="button" className="aiws-dc-ghostbtn" onClick={addNotes}>{t('aiworkspace.calendar.addNotes')}</button>}
      </div>
      <div className="aiws-ctl-form-actions">
        <button type="button" disabled={!canWrite} title={disabled} data-testid="aiws-cal-detail-edit" onClick={onEdit}><Pencil size={14} aria-hidden="true" /> {t('aiworkspace.calendar.edit')}</button>
        {onSkip && <button type="button" disabled={!canWrite} title={disabled} data-testid="aiws-cal-detail-skip" onClick={onSkip}><SkipForward size={14} aria-hidden="true" /> {t('aiworkspace.calendar.skip')}</button>}
        <button type="button" disabled={!canWrite} title={disabled} onClick={onDuplicate}><Copy size={14} aria-hidden="true" /> {t('aiworkspace.calendar.duplicate')}</button>
        <button type="button" className="aiws-dc-iconbtn" aria-label={t('aiworkspace.datactl.copyLink')} title={t('aiworkspace.datactl.copyLink')} data-testid="aiws-cal-detail-link" onClick={onCopyLink}><Link2 size={15} aria-hidden="true" /></button>
        {onAnnotate && <button type="button" className="aiws-dc-iconbtn" aria-label={t('aiworkspace.datactl.annotate')} title={t('aiworkspace.datactl.annotate')} onClick={onAnnotate}><MessageSquarePlus size={15} aria-hidden="true" /></button>}
        <button type="button" className="aiws-danger-link" disabled={!canWrite} title={disabled} data-testid="aiws-cal-detail-delete" onClick={onDelete}><Trash2 size={14} aria-hidden="true" /> {t('aiworkspace.common.delete')}</button>
      </div>
    </div>
  )
}
