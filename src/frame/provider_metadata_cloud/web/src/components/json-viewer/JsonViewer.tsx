import { useMemo, useState } from 'react'
import { Check, Copy, Download } from 'lucide-react'
import { useI18n } from '../../i18n/provider'

export function JsonViewer({ value, filename = 'metadata.json' }: { value: unknown; filename?: string }) {
  const { t } = useI18n()
  const [copied, setCopied] = useState(false)
  const text = useMemo(() => JSON.stringify(value, null, 2), [value])

  async function handleCopy() {
    await navigator.clipboard.writeText(text)
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1200)
  }

  function handleDownload() {
    const blob = new Blob([text], { type: 'application/json;charset=utf-8' })
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = filename
    link.click()
    URL.revokeObjectURL(url)
  }

  return (
    <div className="overflow-hidden rounded-lg border border-[color:var(--cp-border)]">
      <div className="flex items-center justify-end gap-2 border-b border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 py-2">
        <button className="inline-flex h-8 items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-2 text-xs font-semibold" onClick={handleCopy} type="button">
          {copied ? <Check size={14} /> : <Copy size={14} />}
          {copied ? t('action.copied', 'Copied') : t('action.copy', 'Copy')}
        </button>
        <button className="inline-flex h-8 items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-2 text-xs font-semibold" onClick={handleDownload} type="button">
          <Download size={14} />
          {t('action.download', 'Download')}
        </button>
      </div>
      <pre className="shell-scrollbar max-h-[420px] overflow-auto bg-[color:color-mix(in_srgb,var(--cp-bg-strong)_74%,transparent)] p-3 text-xs leading-5 text-[color:var(--cp-text)]">
        {text}
      </pre>
    </div>
  )
}
