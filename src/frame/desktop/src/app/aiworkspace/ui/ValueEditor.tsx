import { useRef, useState, type KeyboardEvent } from 'react'
import type { FieldDef, Json } from '../api/types'
import { useAiwsText } from '../useAiwsText'
import { ReferenceInput } from './table/TableReference'
import { UNSET, editText, parseInput, type Input } from './values'

interface Props {
  def: FieldDef
  value: Json | undefined
  initialText?: string
  ariaLabel: string
  onCommit: (value: Input) => Promise<boolean>
  onCancel: () => void
  onTab: (back: boolean) => void
}

export function ValueEditor({ def, value, initialText, ariaLabel, onCommit, onCancel, onTab }: Props) {
  const { t } = useAiwsText()
  const [startText] = useState(() => editText(def, value))
  const [text, setText] = useState(initialText ?? startText)
  const [error, setError] = useState<string | null>(null)
  const [selected, setSelected] = useState<string[]>(Array.isArray(value) ? value.map(String) : [])
  const settled = useRef(false)

  const cancel = () => {
    if (settled.current) return
    settled.current = true
    onCancel()
  }
  const finish = async (next: Input, back?: boolean) => {
    if (settled.current) return
    if (next === UNSET && def.required && !def.nullable) {
      setError(t('aiworkspace.data.record.required', { name: def.name }))
      return
    }
    settled.current = true
    const input = next === UNSET && def.required && def.nullable ? null : next
    const unchanged = (input === UNSET && value === undefined) || input === value || (Array.isArray(input) && JSON.stringify(input) === JSON.stringify(value))
    if (unchanged) onCancel()
    else if (!await onCommit(input)) return
    if (back !== undefined) onTab(back)
  }
  const commit = (back?: boolean) => {
    if (settled.current) return
    if (def.type === 'multi_select') { void finish(selected.length ? selected : UNSET, back); return }
    if (text === startText) {
      cancel()
      if (back !== undefined) onTab(back)
      return
    }
    const parsed = parseInput(def, text)
    if (!parsed.ok) { setError(parsed.error); return }
    void finish(parsed.value, back)
  }
  const keyDown = (event: KeyboardEvent) => {
    if (event.nativeEvent.isComposing || event.key === 'Process') return
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); cancel() }
    if (event.key === 'Tab') { event.preventDefault(); event.stopPropagation(); if (!event.repeat) commit(event.shiftKey) }
    if (event.key === 'Enter' && (def.type !== 'json' || event.ctrlKey || event.metaKey) && def.type !== 'multi_select' && def.type !== 'select' && def.type !== 'object_ref') {
      event.preventDefault(); event.stopPropagation(); commit()
    }
  }
  if (def.type === 'object_ref' && def.ref_purpose) {
    return <span onKeyDown={keyDown}><ReferenceInput field={def} value={value} ariaLabel={ariaLabel} autoFocus onChange={(v) => { void finish(v ?? UNSET) }} onCancel={cancel} /></span>
  }
  return (
    <span className={`aiws-editor${def.type === 'json' ? ' aiws-editor-json' : def.type === 'multi_select' ? ' aiws-editor-multi' : ''}`} onKeyDown={keyDown}
      role={def.type === 'multi_select' ? 'group' : undefined} aria-label={def.type === 'multi_select' ? ariaLabel : undefined}>
      {def.type === 'select' || def.type === 'boolean' ? (
        <select aria-label={ariaLabel} autoFocus defaultValue={value === undefined || value === null ? '' : String(value)} onBlur={cancel}
          onChange={(event) => { const v = event.target.value; void finish(v === '' ? UNSET : def.type === 'boolean' ? v === 'true' : v) }}>
          <option value="">{t('aiworkspace.data.value.notSet')}</option>
          {def.type === 'boolean' ? <><option value="true">{t('aiworkspace.common.yes')}</option><option value="false">{t('aiworkspace.common.no')}</option></>
            : def.options?.map((option) => <option key={option.option_id} value={option.option_id}>{option.label}</option>)}
        </select>
      ) : def.type === 'multi_select' ? (
        <>
          {def.options?.map((option, i) => (
            <label key={option.option_id}>
              <input type="checkbox" autoFocus={i === 0} checked={selected.includes(option.option_id)}
                onChange={(event) => setSelected(event.target.checked ? [...selected, option.option_id] : selected.filter((id) => id !== option.option_id))} />
              {option.label}
            </label>
          ))}
          <button type="button" onClick={() => commit()}>{t('aiworkspace.common.ok')}</button>
          <button type="button" onClick={cancel}>{t('aiworkspace.common.cancel')}</button>
        </>
      ) : def.type === 'json' ? (
        <textarea aria-label={ariaLabel} autoFocus rows={Math.min(12, Math.max(3, text.split('\n').length))} value={text} spellCheck={false}
          onChange={(event) => { setText(event.target.value); setError(null) }} onBlur={() => commit()} />
      ) : (
        <input aria-label={ariaLabel} autoFocus type={def.type === 'date' ? 'date' : def.type === 'datetime' ? 'datetime-local' : 'text'}
          inputMode={def.type === 'number' || def.type === 'decimal' ? 'decimal' : undefined}
          placeholder={def.type === 'currency' && def.currency ? def.currency : undefined}
          value={text} onChange={(event) => { setText(event.target.value); setError(null) }} onBlur={() => commit()} />
      )}
      {error && <span className="aiws-error" role="alert">{error}</span>}
    </span>
  )
}
