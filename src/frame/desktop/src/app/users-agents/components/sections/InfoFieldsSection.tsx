/* ── Key-value info fields section ── */

import { Edit3 } from 'lucide-react'
import { IconButton } from '@mui/material'

interface InfoFieldsSectionProps {
  title: string
  fields: Record<string, string>
  editable?: boolean
  onFieldChange?: (key: string, value: string) => void
}

export function InfoFieldsSection({ title, fields, editable = true, onFieldChange }: InfoFieldsSectionProps) {
  const entries = Object.entries(fields)
  const canEdit = editable && Boolean(onFieldChange)

  const editField = (key: string, value: string) => {
    const nextValue = window.prompt(key, value)
    if (nextValue !== null) {
      onFieldChange?.(key, nextValue.trim())
    }
  }

  return (
    <div
      className="rounded-[22px] px-5 py-4"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
      }}
    >
      <div className="flex items-center justify-between mb-3">
        <h3
          className="font-display text-sm font-semibold"
          style={{ color: 'var(--cp-text)' }}
        >
          {title}
        </h3>
      </div>

      {entries.length === 0 ? (
        <div className="text-sm" style={{ color: 'var(--cp-muted)' }}>
          No information configured.
        </div>
      ) : (
        <div className="space-y-2">
          {entries.map(([key, value]) => (
            <div key={key} className="flex items-start gap-3">
              <span
                className="text-[12px] font-medium capitalize shrink-0 w-24 pt-0.5"
                style={{ color: 'var(--cp-muted)' }}
              >
                {key}
              </span>
              <span className="text-sm flex-1 min-w-0 break-words" style={{ color: 'var(--cp-text)' }}>
                {value}
              </span>
              {canEdit && (
                <IconButton
                  size="small"
                  aria-label={`Edit ${key}`}
                  onClick={() => editField(key, value)}
                >
                  <Edit3 size={13} />
                </IconButton>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
