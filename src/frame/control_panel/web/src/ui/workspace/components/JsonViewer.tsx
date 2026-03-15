import { useState } from 'react'

import Icon from '../../icons'

type JsonViewerProps = {
  label: string
  data: unknown
  defaultOpen?: boolean
}

const JsonViewer = ({ label, data, defaultOpen = false }: JsonViewerProps) => {
  const [open, setOpen] = useState(defaultOpen)
  const formatted = JSON.stringify(data, null, 2)

  const handleCopy = () => {
    void navigator.clipboard.writeText(formatted)
  }

  return (
    <div className="rounded-xl border border-[var(--cp-border)]">
      <button
        type="button"
        onClick={() => setOpen((p) => !p)}
        className="flex w-full items-center gap-2 px-3 py-2 text-xs font-semibold text-[var(--cp-muted)] transition hover:bg-[var(--cp-surface-muted)]"
      >
        <Icon name={open ? 'chevron-down' : 'chevron-right'} className="size-3" />
        {label}
      </button>
      {open && (
        <div className="relative border-t border-[var(--cp-border)] bg-[var(--cp-surface-muted)]">
          <button
            type="button"
            onClick={handleCopy}
            className="absolute right-2 top-2 rounded-lg p-1 text-[var(--cp-muted)] transition hover:bg-white hover:text-[var(--cp-ink)]"
            title="Copy JSON"
          >
            <Icon name="copy" className="size-3.5" />
          </button>
          <pre className="max-h-64 overflow-auto p-3 font-mono text-[11px] leading-5 text-[var(--cp-ink)]">
            {formatted}
          </pre>
        </div>
      )}
    </div>
  )
}

export default JsonViewer
