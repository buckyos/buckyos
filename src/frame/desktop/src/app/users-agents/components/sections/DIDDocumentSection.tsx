/* ── DID Document viewer section ── */

import { FileText, AlertTriangle } from 'lucide-react'
import { Button } from '@mui/material'

interface DIDDocumentSectionProps {
  document?: Record<string, unknown>
}

export function DIDDocumentSection({ document }: DIDDocumentSectionProps) {
  return (
    <div
      className="rounded-[22px] px-5 py-4"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
      }}
    >
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-2">
          <FileText size={16} style={{ color: 'var(--cp-accent)' }} />
          <h3
            className="font-display text-sm font-semibold"
            style={{ color: 'var(--cp-text)' }}
          >
            DID Document
          </h3>
        </div>
        <Button size="small" variant="text">
          Modify
        </Button>
      </div>

      {/* warning about modification cost */}
      <div
        className="flex items-start gap-2 px-3 py-2 rounded-[12px] mb-3"
        style={{
          background: 'color-mix(in srgb, var(--cp-warning) 10%, var(--cp-surface))',
          border: '1px solid color-mix(in srgb, var(--cp-warning) 20%, transparent)',
        }}
      >
        <AlertTriangle size={14} className="shrink-0 mt-0.5" style={{ color: 'var(--cp-warning)' }} />
        <span className="text-[12px]" style={{ color: 'var(--cp-text)' }}>
          DID Document contains trusted identity data. Modifications may require additional confirmation or cost.
        </span>
      </div>

      {document ? (
        <pre
          className="text-[11px] leading-5 overflow-x-auto rounded-[12px] px-3 py-2 desktop-scrollbar"
          style={{
            background: 'color-mix(in srgb, var(--cp-surface) 80%, transparent)',
            color: 'var(--cp-text)',
            maxHeight: 200,
          }}
        >
          {JSON.stringify(document, null, 2)}
        </pre>
      ) : (
        <div className="text-sm" style={{ color: 'var(--cp-muted)' }}>
          No DID Document configured.
        </div>
      )}
    </div>
  )
}
