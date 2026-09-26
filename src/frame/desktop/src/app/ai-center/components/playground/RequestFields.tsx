import { useEffect, useId, useRef, useState } from 'react'
import { Plus, Trash2 } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { fieldDefault, fieldsDefault, record, type JsonObject, type PlaygroundField } from '../../datamodel/playground'

import { controlClass, controlStyle } from './styles'

const MAX_FILE_BYTES = 20 * 1024 * 1024

function JsonInput({ value, onChange, label }: { value: unknown; onChange: (value: unknown) => void; label: string }) {
  const [draft, setDraft] = useState<string | null>(null)
  const [invalid, setInvalid] = useState(false)
  const { t } = useI18n()
  return <div>
    <textarea aria-label={label} className={`${controlClass} font-mono`} style={controlStyle} rows={5} value={draft ?? JSON.stringify(value, null, 2)} onChange={(event) => {
      setDraft(event.target.value)
      try { onChange(JSON.parse(event.target.value)); setInvalid(false) } catch { setInvalid(true) }
      event.target.setCustomValidity((() => { try { JSON.parse(event.target.value); return '' } catch { return t('aiCenter.playground.invalidJson') } })())
    }} />
    {invalid && <p role="alert" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.playground.invalidJson')}</p>}
  </div>
}

function ResourceInput({ field, value, onChange, label }: { field: PlaygroundField; value: unknown; onChange: (value: unknown) => void; label: string }) {
  const { t } = useI18n()
  const ref = record(value)
  const [upload, setUpload] = useState(false)
  const [reading, setReading] = useState(false)
  const [error, setError] = useState('')
  const generation = useRef(0)
  const latestOnChange = useRef(onChange)
  useEffect(() => { latestOnChange.current = onChange }, [onChange])
  useEffect(() => () => { generation.current++ }, [])
  const mode = upload ? 'file' : String(ref.kind ?? 'url')
  const mimes = field.accept?.startsWith('audio') ? ['audio/mpeg', 'audio/wav', 'audio/ogg', 'audio/flac'] : field.accept?.startsWith('video') ? ['video/mp4', 'video/webm'] : ['image/png', 'image/jpeg', 'image/webp', 'application/pdf', 'text/plain']
  return <div className="space-y-2 min-w-0">
    <select aria-label={`${label} ${t('aiCenter.playground.inputMode')}`} className={controlClass} style={controlStyle} value={mode} onChange={(event) => {
      generation.current++; setReading(false); setError(''); setUpload(event.target.value === 'file')
      onChange(event.target.value === 'url' ? { kind: 'url', url: '' } : event.target.value === 'named_object' ? { kind: 'named_object', obj_id: '' } : { kind: 'base64', mime: mimes[0], data_base64: '' })
    }}>
      <option value="url">URL</option><option value="base64">Base64</option><option value="file">{t('aiCenter.playground.upload')}</option><option value="named_object">{t('aiCenter.playground.namedObject')}</option>
    </select>
    {mode === 'url' && <input aria-label={`${label} URL`} type="url" className={controlClass} style={controlStyle} placeholder="https://" value={String(ref.url ?? '')} onChange={(event) => onChange({ ...ref, url: event.target.value })} />}
    {mode === 'named_object' && <input aria-label={`${label} Object ID`} className={controlClass} style={controlStyle} value={String(ref.obj_id ?? '')} onChange={(event) => onChange({ ...ref, obj_id: event.target.value })} />}
    {(mode === 'base64' || mode === 'file') && <select aria-label={`${label} MIME`} className={controlClass} style={controlStyle} value={String(ref.mime ?? mimes[0])} onChange={(event) => onChange({ ...ref, mime: event.target.value })}>
      {[...new Set([...mimes, ...(ref.mime ? [String(ref.mime)] : [])])].map((mime) => <option key={mime}>{mime}</option>)}
    </select>}
    {mode === 'base64' && <textarea aria-label={`${label} Base64`} className={`${controlClass} font-mono`} style={controlStyle} rows={3} value={String(ref.data_base64 ?? '')} onChange={(event) => {
      const raw = event.target.value.trim()
      const data = /^data:([^;,]+);base64,([\s\S]*)$/.exec(raw)
      onChange({ kind: 'base64', mime: data?.[1] ?? ref.mime, data_base64: (data?.[2] ?? raw).replace(/\s/g, '') })
    }} />}
    {mode === 'file' && <>
      <input aria-label={`${label} ${t('aiCenter.playground.upload')}`} type="file" accept={field.accept} className={controlClass} style={controlStyle} onChange={async (event) => {
        const input = event.currentTarget
        const file = input.files?.[0]
        if (!file) return
        const current = ++generation.current
        setError(''); setReading(true)
        onChange({ kind: 'base64', mime: file.type || mimes[0], data_base64: '' })
        try {
          if (file.size === 0 || file.size > MAX_FILE_BYTES) throw new Error(t('aiCenter.playground.fileSize'))
          const data = await new Promise<string>((resolve, reject) => {
            const reader = new FileReader()
            reader.onload = () => resolve(String(reader.result).split(',')[1])
            reader.onerror = () => reject(new Error(t('aiCenter.playground.fileRead')))
            reader.readAsDataURL(file)
          })
          if (current === generation.current) { latestOnChange.current({ kind: 'base64', mime: file.type || mimes[0], data_base64: data }); input.setCustomValidity('') }
        } catch (error) {
          if (current === generation.current) { const message = String(error instanceof Error ? error.message : error); setError(message); input.setCustomValidity(message) }
        } finally { if (current === generation.current) setReading(false) }
      }} />
      <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.playground.fileHint')}</p>
    </>}
    {reading && <p role="status">{t('aiCenter.playground.reading')}</p>}
    {error && <p role="alert" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
  </div>
}

export function RequestFields({ fields, value, onChange, path = '' }: { fields: PlaygroundField[]; value: JsonObject; onChange: (value: JsonObject) => void; path?: string }) {
  return <div className="space-y-4">{fields.map((field) => <RequestField key={field.key} field={field} value={value[field.key]} path={path ? `${path}.${field.key}` : field.key} onChange={(next) => {
    const copy = { ...value }
    if (next === undefined) delete copy[field.key]
    else copy[field.key] = next
    onChange(copy)
  }} />)}</div>
}

function RequestField({ field, value, onChange, path }: { field: PlaygroundField; value: unknown; onChange: (value: unknown) => void; path: string }) {
  const { t } = useI18n()
  const id = useId()
  if (field.required && value === undefined) value = fieldDefault(field)
  const label = t(`aiCenter.playground.field.${field.key}`, field.key || path)
  const present = value !== undefined && (value !== null || field.nullable)
  const nested = Boolean(field.key) && ['object', 'resource'].includes(field.kind)
  let editor: React.ReactNode
  if (field.nullable && value === null) editor = <JsonInput value={value} onChange={onChange} label={path} />
  else if (field.kind === 'resource') editor = <ResourceInput field={field} value={value} onChange={onChange} label={path} />
  else if (field.kind === 'json' || (field.kind === 'text' && typeof value === 'object' && value !== null)) editor = <JsonInput value={value} onChange={onChange} label={path} />
  else if (field.kind === 'object') editor = <RequestFields fields={field.fields ?? []} value={record(value)} onChange={onChange} path={path} />
  else if (field.kind === 'union') {
    const obj = record(value)
    const variant = String(obj[field.discriminator!])
    editor = <div className="space-y-3">
      <select aria-label={`${path}.type`} className={controlClass} style={controlStyle} value={variant} onChange={(event) => onChange({ [field.discriminator!]: event.target.value, ...fieldsDefault(field.variants![event.target.value]) })}>
        {[...new Set([...Object.keys(field.variants!), variant])].map((key) => <option key={key}>{key}</option>)}
      </select>
      {field.variants && Object.hasOwn(field.variants, variant) ? <RequestFields key={variant} fields={field.variants[variant]} value={obj} onChange={onChange} path={path} /> : <JsonInput value={value} onChange={onChange} label={path} />}
    </div>
  } else if (field.kind === 'array') {
    editor = <ArrayInput field={field} value={value} onChange={onChange} path={path} label={label} />
  } else if (field.kind === 'checks') {
    const selected = Array.isArray(value) ? value as string[] : []
    editor = <div className="flex flex-wrap gap-3">{[...new Set([...(field.options ?? []), ...selected])].map((option) => <label key={option} className="flex items-center gap-2 text-sm"><input type="checkbox" checked={selected.includes(option)} onChange={(event) => onChange(event.target.checked ? [...selected, option] : selected.filter((item) => item !== option))} />{option}</label>)}</div>
  } else if (field.kind === 'select') editor = <select id={id} aria-label={path} className={controlClass} style={controlStyle} value={String(value ?? '')} onChange={(event) => onChange(event.target.value)}>
    {[...new Set([...(field.options ?? []), ...(present ? [String(value)] : [])])].map((option) => <option key={option}>{option}</option>)}
  </select>
  else if (field.kind === 'boolean') editor = <label className="flex items-center gap-2"><input id={id} aria-label={path} type="checkbox" checked={value === true} onChange={(event) => onChange(event.target.checked)} />{t(value ? 'common.on' : 'common.off')}</label>
  else if (field.kind === 'number') editor = <input id={id} aria-label={path} type="number" className={controlClass} style={controlStyle} min={field.min} max={field.max} step={field.step} value={typeof value === 'number' ? value : ''} required={present} onChange={(event) => onChange(event.target.value === '' ? '' : Number(event.target.value))} />
  else editor = <textarea id={id} aria-label={path} className={controlClass} style={controlStyle} rows={field.key === 'id' || field.key.endsWith('_id') ? 1 : 3} value={String(value ?? '')} onChange={(event) => onChange(event.target.value)} />
  return <div className={`min-w-0 ${nested && present ? 'rounded-xl p-3' : ''}`} style={nested && present ? { background: 'var(--cp-surface-2)' } : undefined}>
    {field.key && <div className="flex items-center gap-2 mb-2 text-sm font-medium">
      {!field.required && <input type="checkbox" aria-label={`${t('aiCenter.playground.enable')} ${path}`} checked={present} onChange={(event) => onChange(event.target.checked ? fieldDefault(field) : undefined)} />}
      <label htmlFor={id}>{label}{field.required ? ' *' : ''}</label>
    </div>}
    {present && editor}
  </div>
}

function ArrayInput({ field, value, onChange, path, label }: { field: PlaygroundField; value: unknown; onChange: (value: unknown) => void; path: string; label: string }) {
  const { t } = useI18n()
  const items = Array.isArray(value) ? value : []
  const nextKey = useRef(items.length)
  const [keys, setKeys] = useState(() => items.map((_, index) => index))
  return <div className="space-y-3">{items.map((item, index) => <div key={keys[index]} className="rounded-lg border p-3 space-y-2" style={{ borderColor: 'var(--cp-border)' }}>
    <div className="flex items-center justify-between text-xs"><span>{index + 1}</span><button type="button" aria-label={`${t('aiCenter.playground.remove')} ${path}[${index}]`} onClick={() => {
      setKeys(keys.filter((_, i) => i !== index))
      onChange(items.filter((_, i) => i !== index))
    }}><Trash2 size={16} /></button></div>
    <RequestField field={{ ...field.item!, required: true }} value={item} path={`${path}[${index}]`} onChange={(next) => onChange(items.map((old, i) => i === index ? next : old))} />
  </div>)}<button type="button" className="inline-flex items-center gap-2 text-sm" style={{ color: 'var(--cp-accent)' }} onClick={() => {
    setKeys([...keys, nextKey.current++])
    onChange([...items, fieldDefault(field.item!)])
  }}><Plus size={16} />{t('aiCenter.playground.add')} {label}</button></div>
}
