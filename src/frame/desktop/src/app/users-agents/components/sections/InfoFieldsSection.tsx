/* ── Key-value info fields section ── */

import { Edit3 } from 'lucide-react'
import { Button } from '@mui/material'

interface InfoFieldsSectionProps {
  title: string
  fields: Record<string, string>
  editable?: boolean
}

export function InfoFieldsSection({ title, fields, editable = true }: InfoFieldsSectionProps) {
  const entries = Object.entries(fields)

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
        {editable && (
          <Button size="small" startIcon={<Edit3 size={13} />} variant="text">
            Edit
          </Button>
        )}
      </div>

      {entries.length === 0 ? (
        <div className="text-sm" style={{ color: 'var(--cp-muted)' }}>
          No information configured.
        </div>
      ) : (
        <div className="space-y-2">
          {entries.map(([key, value]) => (
            <div key={key} className="flex items-baseline gap-3">
              <span
                className="text-[12px] font-medium capitalize shrink-0 w-24"
                style={{ color: 'var(--cp-muted)' }}
              >
                {key}
              </span>
              <span className="text-sm flex-1 min-w-0 break-words" style={{ color: 'var(--cp-text)' }}>
                {value}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
