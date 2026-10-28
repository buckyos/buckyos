import { useState } from 'react'
import { useI18n } from '../../../../../i18n/provider'

interface RuleMappingEditorProps {
  label: string
  ariaLabel: string
  value: string
  routes?: boolean
  onChange: (value: string) => void
}

export function RuleMappingEditor({ label, ariaLabel, value, routes = false, onChange }: RuleMappingEditorProps) {
  const { t } = useI18n()
  const [mode, setMode] = useState<'list' | 'json'>('list')
  const parse = (source: string): { source: string; rows: [string, string][]; error: boolean } => {
    try {
      const parsed: unknown = JSON.parse(source)
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error()
      const entries = Object.entries(parsed)
      if (entries.some(([, item]) => routes
        ? !Array.isArray(item) || item.some((task) => typeof task !== 'string')
        : typeof item !== 'string')) throw new Error()
      return { source, rows: entries.map(([key, item]) => [key, routes ? (item as string[]).join(', ') : item as string]), error: false }
    } catch {
      return { source, rows: [], error: true }
    }
  }
  const [mapping, setMapping] = useState(() => parse(value))
  if (mapping.source !== value) setMapping(parse(value))
  const { rows, error } = mapping

  const updateRows = (next: [string, string][]) => {
    const duplicate = new Set(next.map(([key]) => key)).size !== next.length
    const json = duplicate ? '' : JSON.stringify(Object.fromEntries(next.map(([key, item]) => [
      key, routes ? item.split(',').map((task) => task.trim()).filter(Boolean) : item,
    ])), null, 2)
    const source = duplicate ? '{' : json
    setMapping({ source, rows: next, error: duplicate })
    onChange(source)
  }

  return (
    <fieldset className="flex min-w-0 flex-col gap-2 rounded-lg border p-3" style={{ borderColor: 'var(--cp-border)' }}>
      <legend className="px-1 text-xs">{label}</legend>
      <div className="flex gap-2" role="group" aria-label={ariaLabel}>
        {(['list', 'json'] as const).map((option) => (
          <button key={option} type="button" aria-pressed={mode === option} disabled={option === 'list' && mode === 'json' && error} onClick={() => setMode(option)}
            className="rounded border px-3 py-1 text-xs" style={{ color: mode === option ? 'var(--cp-accent)' : 'var(--cp-muted)' }}>
            {option === 'json' ? 'JSON' : t('aiCenter.wizard.listEditor', 'List')}
          </button>
        ))}
      </div>
      {mode === 'json' ? (
        <textarea aria-label={ariaLabel} className="block w-full rounded border p-2 font-mono text-xs" rows={6}
          value={value} onChange={(event) => onChange(event.target.value)} />
      ) : (
        <>
          {rows.length === 0 && <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.wizard.emptyRules', 'No entries configured.')}</p>}
          {rows.map(([key, item], index) => (
            <div key={index} className="flex flex-wrap gap-2">
              <input aria-label={`${ariaLabel}: model ${index + 1}`} placeholder={t('aiCenter.wizard.endpointModel', 'Endpoint model')}
                className="min-w-0 flex-1 rounded border p-2 text-xs" value={key}
                onChange={(event) => updateRows(rows.map((row, i) => i === index ? [event.target.value, row[1]] : row))} />
              <input aria-label={`${ariaLabel}: value ${index + 1}`}
                placeholder={routes ? 'llm.chat, llm.reason' : 'manufacturer/model'}
                className="min-w-0 flex-1 rounded border p-2 text-xs" value={item}
                onChange={(event) => updateRows(rows.map((row, i) => i === index ? [row[0], event.target.value] : row))} />
              <button type="button" className="text-xs" onClick={() => updateRows(rows.filter((_, i) => i !== index))}>{t('common.remove', 'Remove')}</button>
            </div>
          ))}
          <button type="button" className="self-start text-xs" onClick={() => updateRows([...rows, ['', '']])}>
            {t('aiCenter.wizard.addRule', 'Add entry')}
          </button>
        </>
      )}
      {error && <p role="alert" className="text-xs" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.wizard.invalidRuleMapping', 'Enter a valid JSON mapping with unique model IDs before using the list editor.')}</p>}
    </fieldset>
  )
}
