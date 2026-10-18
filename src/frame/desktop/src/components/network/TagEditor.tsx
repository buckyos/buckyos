/* ── TagEditor: the user tags (Collections) of one person; system and app tags are never editable here ── */

import { Button } from '@mui/material'
import { Plus, X } from 'lucide-react'
import { useState } from 'react'
import { useI18n } from '../../i18n/provider'
import { userTagSchema } from './relation'

export function TagEditor({
  value,
  onChange,
  suggestions = [],
  disabled = false,
  label,
}: {
  value: string[]
  onChange: (tags: string[]) => void
  suggestions?: string[]
  disabled?: boolean
  label?: string
}) {
  const { t } = useI18n()
  const [draft, setDraft] = useState('')
  const [error, setError] = useState<string | null>(null)
  const offered = suggestions.filter(tag => !value.includes(tag)).slice(0, 8)

  const add = (raw: string) => {
    const parsed = userTagSchema.safeParse(raw)
    if (!parsed.success) {
      setError(t(parsed.error.issues[0]?.message ?? 'network.tag.error.required'))
      return
    }
    setError(null)
    setDraft('')
    if (!value.includes(parsed.data)) onChange([...value, parsed.data])
  }

  return (
    <div className="space-y-2" data-testid="tag-editor">
      <div className="text-[12px] font-medium text-[color:var(--cp-muted)]">{label ?? t('network.tag.collections')}</div>
      {value.length > 0 ? (
        <ul className="flex flex-wrap gap-1.5">
          {value.map(tag => (
            <li key={tag}>
              <button
                type="button"
                disabled={disabled}
                className="flex min-h-8 items-center gap-1 rounded-full py-1 pl-3 pr-2 text-[13px]"
                style={{ background: 'color-mix(in srgb, var(--cp-accent-soft) 20%, var(--cp-surface))', color: 'var(--cp-text)' }}
                aria-label={t('network.tag.remove', undefined, { tag })}
                onClick={() => onChange(value.filter(item => item !== tag))}
              >
                <span className="max-w-[12rem] truncate">{tag}</span><X size={13} aria-hidden />
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      <div className="flex gap-2">
        <input
          value={draft}
          disabled={disabled}
          maxLength={80}
          onChange={(event) => { setDraft(event.target.value); setError(null) }}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault()
              add(draft)
            }
          }}
          placeholder={t('network.tag.placeholder')}
          aria-label={t('network.tag.new')}
          aria-invalid={Boolean(error)}
          className="min-h-9 min-w-0 flex-1 rounded-lg border border-[color:var(--cp-border)] bg-[color:var(--cp-bg)] px-3 text-sm text-[color:var(--cp-text)]"
        />
        <Button size="small" variant="outlined" startIcon={<Plus size={13} />} disabled={disabled || !draft.trim()} onClick={() => add(draft)}>
          {t('network.tag.add')}
        </Button>
      </div>
      {error ? <p role="alert" className="text-xs text-[color:var(--cp-danger)]">{error}</p> : null}
      {offered.length > 0 ? (
        <div className="flex flex-wrap gap-1.5">
          {offered.map(tag => (
            <button
              key={tag}
              type="button"
              disabled={disabled}
              onClick={() => add(tag)}
              className="min-h-7 rounded-full border border-dashed border-[color:var(--cp-border)] px-2.5 text-[12px] text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]"
            >
              + {tag}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}
