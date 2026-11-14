/* eslint-disable react-refresh/only-export-components -- the record form, its field controls and their hooks */
/* The record form (UI 框架 PRD §7.3, §7.5): every field of one record, one above the other, each with the control of its
 * type — a date picker, a list of options, a number keyboard, a tick box. Each field is written on its own when it is
 * left (or chosen), exactly like a cell of the grid: one undoable Commit guarded by the version seen when the edit began,
 * the save state and a conflict shown under the field. The same form edits a table's record and a `buckyos.record`. A
 * new record's form collects the values and the host writes them in one Commit. */

import { useRef, useState, type ReactNode } from 'react'
import type { Json } from '../../api/types'
import { EDIT_STATE_LABEL, type EditEntry } from '../../state/edits'
import { useEdit, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { ConflictBox } from './ConflictBox'
import { UNSET, formatValue, type Input, type ValueDef } from '../values'
import { inputOf, isEmptyRaw, rawOf, sameRaw, toneOf, type Raw } from './model'

/** The value as a page shows it: option pills, or text. */
export function CellDisplay({ def, value, shown }: { def: ValueDef; value: Json | undefined; shown: string }) {
  const { t } = useAiwsText()
  if ((def.type === 'select' || def.type === 'multi_select') && value !== undefined && value !== null) {
    const ids = Array.isArray(value) ? value.map(String) : [String(value)]
    const known = (def.options ?? []).filter((option) => ids.includes(option.option_id))
    const unknown = ids.filter((id) => !(def.options ?? []).some((option) => option.option_id === id))
    return (
      <span className="aiws-pills">
        {known.map((option) => <span key={option.option_id} className="aiws-pill" data-tone={toneOf(option.option_id)}>{option.label}</span>)}
        {unknown.map((id) => <span key={id} className="aiws-pill" data-tone="0">{t('aiworkspace.data.value.unknownOption', { id })}</span>)}
      </span>
    )
  }
  return <>{shown || <span className="aiws-muted">&nbsp;</span>}</>
}

export interface FormField {
  /** Field id or property key. */
  id: string
  name: string
  def: ValueDef
  value: Json | undefined
  rev: number
  /** The save-state entry this field's writes use (the same as the grid's cell). */
  editId: string
  /** Why the field cannot be edited here, or null. */
  locked: string | null
  /** Shown instead of the value and never edited (a formula's result). */
  display?: ReactNode
  testId: string
}

export type FieldWrite = (field: FormField, input: Input, baseRev: number) => Promise<unknown>

/** One field being edited in place: what is typed stays until it is left, the version is the one seen when it began. */
export function useLiveValue(field: FormField, write: FieldWrite) {
  const { t } = useAiwsText()
  const entry = useEdit(field.editId)
  const pending = Boolean(entry && entry.state !== 'committed' && entry.hasMine)
  const current: Json | undefined = pending ? (entry?.mine === null ? undefined : entry?.mine) : field.value
  const [draft, setDraft] = useState<Raw | null>(null)
  const [error, setError] = useState<string | null>(null)
  const base = useRef<number | null>(null)
  // what was last written or cancelled: a late blur (the control removed, Enter then leaving) must not write it again
  const settled = useRef<{ raw: Raw } | 'cancelled' | null>(null)
  const stored = rawOf(field.def, current)
  const raw = draft ?? stored
  const begin = () => { if (base.current === null) base.current = field.rev }
  const change = (next: Raw) => { begin(); settled.current = null; setDraft(next); setError(null) }
  /** Write `next` (the typed text by default); false when it was refused here (cannot be shaped, required). */
  const commit = (next: Raw = raw): boolean => {
    const done = settled.current
    if (done === 'cancelled' || (done && sameRaw(done.raw, next))) return true
    const rev = base.current ?? field.rev
    base.current = null
    if (sameRaw(next, stored)) { if (draft !== null) { setDraft(null); setError(null) } return true }
    const parsed = inputOf(field.def, next)
    if (!parsed.ok) { setDraft(next); setError(parsed.error); base.current = rev; return false }
    if (parsed.value === UNSET && field.def.required) { setDraft(next); setError(t('aiworkspace.data.record.required', { name: field.name })); base.current = rev; return false }
    settled.current = { raw: next }
    setDraft(null)
    setError(null)
    void write(field, parsed.value, rev)
    return true
  }
  /** Take back what is typed; true when there was something to take back. */
  const cancel = (): boolean => { const had = draft !== null || error !== null; base.current = null; settled.current = 'cancelled'; setDraft(null); setError(null); return had }
  /** After a conflict: my kept input again, based on what is current now. */
  const editAgain = (from: EditEntry) => {
    base.current = from.currentRev ?? field.rev
    settled.current = null
    setDraft(rawOf(field.def, from.mine === null ? undefined : from.mine))
  }
  return { entry, current, raw, draft, error, begin, change, commit, cancel, editAgain }
}

interface ControlProps {
  def: ValueDef
  raw: Raw
  ariaLabel: string
  disabled?: boolean
  autoFocus?: boolean
  id?: string
  onFocus?: () => void
  onChange: (raw: Raw) => void
  /** A typed value is left (blur, Enter); a choice is made (select, tick, option). */
  onCommit: (raw: Raw) => void
  /** Esc: true when it took something back (the key goes no further), false to let it close what holds the control. */
  onCancel?: () => boolean
  /** Choices are kept until the host commits (a new record's form). */
  deferChoices?: boolean
}

/** The control of a value type: what a phone needs to enter it (date picker, number keyboard, option list, tick box). */
export function ValueControl({ def, raw, ariaLabel, disabled, autoFocus, id, onFocus, onChange, onCommit, onCancel, deferChoices }: ControlProps) {
  const { t } = useAiwsText()
  const choose = (next: Raw) => { onChange(next); if (!deferChoices) onCommit(next) }
  if (def.type === 'boolean') {
    return (
      <label className="aiws-field-check">
        <input id={id} type="checkbox" aria-label={ariaLabel} checked={raw === 'true'} disabled={disabled} autoFocus={autoFocus} onFocus={onFocus}
          onChange={(event) => choose(event.target.checked ? 'true' : 'false')} />
        <span>{raw === 'true' ? t('aiworkspace.common.yes') : raw === 'false' ? t('aiworkspace.common.no') : t('aiworkspace.data.value.notSet')}</span>
      </label>
    )
  }
  if (def.type === 'select') {
    return (
      <select id={id} className="aiws-field-input" aria-label={ariaLabel} value={typeof raw === 'string' ? raw : ''} disabled={disabled} autoFocus={autoFocus} onFocus={onFocus}
        onChange={(event) => choose(event.target.value)}>
        <option value="">{t('aiworkspace.data.value.notSet')}</option>
        {(def.options ?? []).map((option) => <option key={option.option_id} value={option.option_id}>{option.label}</option>)}
      </select>
    )
  }
  if (def.type === 'multi_select') {
    const ids = Array.isArray(raw) ? raw : []
    return (
      <span className="aiws-field-options" role="group" aria-label={ariaLabel}>
        {(def.options ?? []).map((option, index) => {
          const on = ids.includes(option.option_id)
          return (
            <button key={option.option_id} type="button" className="aiws-field-option" data-tone={toneOf(option.option_id)} aria-pressed={on} disabled={disabled} autoFocus={autoFocus && index === 0} onFocus={onFocus}
              onClick={() => choose(on ? ids.filter((x) => x !== option.option_id) : [...ids, option.option_id])}>{option.label}</button>
          )
        })}
        {(def.options ?? []).length === 0 && <span className="aiws-muted">{t('aiworkspace.table.form.noOptions')}</span>}
      </span>
    )
  }
  if (def.type === 'object_ref') {
    return <span className="aiws-muted">{t('aiworkspace.data.value.objectRefNotEditable')}</span>
  }
  const text = typeof raw === 'string' ? raw : ''
  const keys = (event: React.KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>, multiline: boolean) => {
    if (event.key === 'Escape' && onCancel?.()) { event.preventDefault(); event.stopPropagation() }
    if (event.key === 'Enter' && (!multiline || event.ctrlKey || event.metaKey) && !event.nativeEvent.isComposing) { event.preventDefault(); onCommit(text) }
  }
  if (def.type === 'json') {
    return (
      <textarea id={id} className="aiws-field-input" aria-label={ariaLabel} rows={Math.min(10, Math.max(3, text.split('\n').length))} value={text} spellCheck={false} disabled={disabled} autoFocus={autoFocus}
        onFocus={onFocus} onChange={(event) => onChange(event.target.value)} onBlur={() => onCommit(text)} onKeyDown={(event) => keys(event, true)} />
    )
  }
  const numeric = def.type === 'number' || def.type === 'decimal'
  return (
    <input id={id} className="aiws-field-input" aria-label={ariaLabel} value={text} disabled={disabled} autoFocus={autoFocus}
      type={def.type === 'date' ? 'date' : def.type === 'datetime' ? 'datetime-local' : 'text'}
      inputMode={numeric ? 'decimal' : undefined} enterKeyHint="done"
      placeholder={def.type === 'currency' && def.currency ? def.currency : undefined}
      onFocus={onFocus} onChange={(event) => onChange(event.target.value)} onBlur={() => onCommit(text)} onKeyDown={(event) => keys(event, false)} />
  )
}

/** The save state and, on a conflict, both values with the choices (the grid's conflict box). */
export function EditState({ field, live, write }: { field: FormField; live: ReturnType<typeof useLiveValue>; write: FieldWrite }) {
  const { t } = useAiwsText()
  const store = useStore()
  const { entry } = live
  if (!entry) return null
  const show = (value: Json | undefined) => formatValue(field.def, value)
  return (
    <>
      <span className={`aiws-state aiws-state-${entry.state}`} data-testid="aiws-edit-state" title={entry.detail}>{EDIT_STATE_LABEL[entry.state]}</span>
      {entry.state === 'needs_attention' && field.locked === null && (
        <ConflictBox
          store={store}
          editId={field.editId}
          theirs={entry.hasTheirs ? show(entry.theirs) || t('aiworkspace.data.value.notSet') : null}
          mine={entry.hasMine ? (entry.mine === null ? t('aiworkspace.data.value.cleared') : show(entry.mine ?? undefined)) : null}
          detail={entry.detail ?? ''}
          onKeepMine={entry.hasMine && entry.currentRev !== undefined ? () => { void write(field, entry.mine === null || entry.mine === undefined ? UNSET : entry.mine, entry.currentRev ?? 0) } : undefined}
          onEditAgain={entry.hasMine ? () => live.editAgain(entry) : undefined}
        />
      )}
    </>
  )
}

function FieldLabel({ field, htmlFor }: { field: { name: string; def: ValueDef }; htmlFor?: string }) {
  const { t } = useAiwsText()
  return (
    <label className="aiws-record-label" htmlFor={htmlFor}>
      {field.name}{field.def.required && <span className="aiws-required" title={t('aiworkspace.table.form.requiredTitle')} aria-hidden="true"> *</span>}
    </label>
  )
}

/** One field of an existing record, edited in place. */
export function LiveField({ field, write, autoFocus }: { field: FormField; write: FieldWrite; autoFocus?: boolean }) {
  const live = useLiveValue(field, write)
  const id = `aiws-f-${field.editId.replace(/[^\w-]/g, '_')}`
  const shown = formatValue(field.def, live.current)
  return (
    <div className="aiws-record-row" data-testid={field.testId} data-state={live.entry?.state ?? 'clean'} data-type={field.def.type}>
      <FieldLabel field={field} htmlFor={field.locked === null && !field.display ? id : undefined} />
      <div className="aiws-record-control">
        {field.display !== undefined ? <span className="aiws-record-value">{field.display}</span>
          : field.locked !== null ? <span className="aiws-record-value" title={field.locked || undefined}><CellDisplay def={field.def} value={live.current} shown={shown} /></span>
            : <ValueControl id={id} def={field.def} raw={live.raw} ariaLabel={field.name} autoFocus={autoFocus} onFocus={live.begin} onChange={live.change} onCommit={(raw) => { live.commit(raw) }} onCancel={live.cancel} />}
      </div>
      {live.error && <div className="aiws-error aiws-record-error" role="alert">{live.error}</div>}
      {live.entry && <div className="aiws-record-state"><EditState field={field} live={live} write={write} /></div>}
    </div>
  )
}

/** Every field of one record, one above the other. */
export function RecordForm({ fields, write, testId, autoFocusFirst }: { fields: FormField[]; write: FieldWrite; testId?: string; autoFocusFirst?: boolean }) {
  const firstEditable = fields.find((field) => field.locked === null && field.display === undefined)?.id
  return (
    <div className="aiws-record-form" data-testid={testId}>
      {fields.map((field) => <LiveField key={field.id} field={field} write={write} autoFocus={autoFocusFirst && field.id === firstEditable} />)}
    </div>
  )
}

// ---- a new record: the values are collected and written together

export interface DraftFieldDef { id: string; name: string; def: ValueDef; testId: string }

export function useDraftRecord(fields: readonly DraftFieldDef[]) {
  const { t } = useAiwsText()
  const [drafts, setDrafts] = useState<Record<string, Raw>>({})
  const [errors, setErrors] = useState<Record<string, string>>({})
  const rawFor = (field: DraftFieldDef): Raw => drafts[field.id] ?? (field.def.type === 'multi_select' ? [] : '')
  const set = (id: string, raw: Raw) => { setDrafts((d) => ({ ...d, [id]: raw })); setErrors((e) => { if (!(id in e)) return e; const next = { ...e }; delete next[id]; return next }) }
  /** The values to insert, or null with every problem shown under its field. */
  const build = (): Record<string, Json> | null => {
    const values: Record<string, Json> = {}
    const problems: Record<string, string> = {}
    for (const field of fields) {
      const raw = rawFor(field)
      if (isEmptyRaw(raw) && !(field.def.type === 'boolean' && raw !== '')) {
        if (field.def.required) problems[field.id] = t('aiworkspace.data.record.required', { name: field.name })
        continue
      }
      const parsed = inputOf(field.def, raw)
      if (!parsed.ok) { problems[field.id] = parsed.error; continue }
      if (parsed.value !== UNSET) values[field.id] = parsed.value
      else if (field.def.required) problems[field.id] = t('aiworkspace.data.record.required', { name: field.name })
    }
    setErrors(problems)
    return Object.keys(problems).length > 0 ? null : values
  }
  const reset = () => { setDrafts({}); setErrors({}) }
  return { rawFor, set, errors, build, reset }
}

/** The fields of a new record: nothing is written until the host adds it. */
export function DraftRecordForm({ fields, draft, ariaLabel, autoFocusFirst = true }: { fields: readonly DraftFieldDef[]; draft: ReturnType<typeof useDraftRecord>; ariaLabel: (name: string) => string; autoFocusFirst?: boolean }) {
  return (
    <div className="aiws-record-form" data-testid="aiws-table-new-form">
      {fields.map((field, index) => {
        const id = `aiws-new-${field.id}`
        const error = draft.errors[field.id]
        return (
          <div key={field.id} className="aiws-record-row" data-testid={field.testId} data-invalid={error ? 'true' : undefined} data-type={field.def.type}>
            <FieldLabel field={field} htmlFor={id} />
            <div className="aiws-record-control">
              <ValueControl id={id} def={field.def} raw={draft.rawFor(field)} ariaLabel={ariaLabel(field.name)} autoFocus={autoFocusFirst && index === 0} deferChoices
                onChange={(raw) => draft.set(field.id, raw)} onCommit={(raw) => draft.set(field.id, raw)} />
            </div>
            {error && <div className="aiws-error aiws-record-error" role="alert" data-testid="aiws-table-field-error">{error}</div>}
          </div>
        )
      })}
    </div>
  )
}
