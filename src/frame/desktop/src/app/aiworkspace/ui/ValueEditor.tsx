import { useRef, useState } from 'react'
import type { Json } from '../api/types'
import { UNSET, editText, parseInput, type Input, type ValueDef } from './values'

interface Props {
  def: ValueDef
  value: Json | undefined
  /** Text to start from instead of the stored value (my kept input after a conflict). */
  initialText?: string
  ariaLabel: string
  onCommit: (value: Input) => void
  onCancel: () => void
}

/** Inline editor for one value. Enter / blur commits, Escape cancels; nothing is committed when the text is unchanged. */
export function ValueEditor({ def, value, initialText, ariaLabel, onCommit, onCancel }: Props) {
  const startText = initialText ?? editText(def, value)
  const [text, setText] = useState(startText)
  const [error, setError] = useState<string | null>(null)
  const [selected, setSelected] = useState<string[]>(Array.isArray(value) ? value.map(String) : [])
  // Enter commits and the host then removes the input; a late blur must not commit a second time.
  const settled = useRef(false)

  if (def.type === 'boolean') {
    return (
      <span className="aiws-editor">
        <select aria-label={ariaLabel} autoFocus defaultValue={value === true ? 'true' : value === false ? 'false' : ''} onBlur={onCancel}
          onChange={(event) => onCommit(event.target.value === '' ? UNSET : event.target.value === 'true')}
          onKeyDown={(event) => { if (event.key === 'Escape') onCancel() }}>
          <option value="">（未设置）</option><option value="true">是</option><option value="false">否</option>
        </select>
      </span>
    )
  }
  if (def.type === 'select') {
    return (
      <span className="aiws-editor">
        <select aria-label={ariaLabel} autoFocus defaultValue={typeof value === 'string' ? value : ''} onBlur={onCancel}
          onChange={(event) => onCommit(event.target.value === '' ? UNSET : event.target.value)}
          onKeyDown={(event) => { if (event.key === 'Escape') onCancel() }}>
          <option value="">（未设置）</option>
          {(def.options ?? []).map((option) => <option key={option.option_id} value={option.option_id}>{option.label}</option>)}
        </select>
      </span>
    )
  }
  if (def.type === 'multi_select') {
    return (
      <span className="aiws-editor aiws-editor-multi" role="group" aria-label={ariaLabel}>
        {(def.options ?? []).map((option) => (
          <label key={option.option_id}>
            <input type="checkbox" checked={selected.includes(option.option_id)}
              onChange={(event) => setSelected(event.target.checked ? [...selected, option.option_id] : selected.filter((id) => id !== option.option_id))} />
            {option.label}
          </label>
        ))}
        <button type="button" onClick={() => onCommit(selected.length === 0 ? UNSET : selected)}>确定</button>
        <button type="button" onClick={onCancel}>取消</button>
      </span>
    )
  }
  if (def.type === 'object_ref') {
    return <span className="aiws-editor aiws-muted">对象引用暂不支持在此编辑 <button type="button" onClick={onCancel}>关闭</button></span>
  }
  const cancel = () => {
    if (settled.current) return
    settled.current = true
    onCancel()
  }
  const commit = () => {
    if (settled.current) return
    if (text === startText && initialText === undefined) { cancel(); return }
    const parsed = parseInput(def, text)
    if (!parsed.ok) { setError(parsed.error); return }
    settled.current = true
    onCommit(parsed.value)
  }
  return (
    <span className="aiws-editor">
      <input
        aria-label={ariaLabel}
        autoFocus
        type={def.type === 'date' ? 'date' : def.type === 'datetime' ? 'datetime-local' : 'text'}
        inputMode={def.type === 'number' || def.type === 'decimal' ? 'decimal' : undefined}
        value={text}
        onChange={(event) => { setText(event.target.value); setError(null) }}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === 'Enter') { event.preventDefault(); commit() }
          if (event.key === 'Escape') { event.preventDefault(); cancel() }
        }}
      />
      {error && <span className="aiws-error" role="alert">{error}</span>}
    </span>
  )
}
