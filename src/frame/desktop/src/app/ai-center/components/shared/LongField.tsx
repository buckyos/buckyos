import { useState } from 'react'
import { Check, ChevronDown, ChevronUp, Copy } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'

interface LongFieldProps {
  value?: string | null
  fallback?: string
  title?: string
  className?: string
  mono?: boolean
  copyable?: boolean
  expandable?: boolean
  tone?: 'default' | 'muted' | 'warning' | 'danger' | 'accent'
}

function toneColor(tone: LongFieldProps['tone']): string {
  if (tone === 'muted') return 'var(--cp-muted)'
  if (tone === 'warning') return 'var(--cp-warning)'
  if (tone === 'danger') return 'var(--cp-danger)'
  if (tone === 'accent') return 'var(--cp-accent)'
  return 'var(--cp-text)'
}

async function writeClipboard(value: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(value)
    return
  } catch {
    const textarea = document.createElement('textarea')
    textarea.value = value
    textarea.setAttribute('readonly', '')
    textarea.style.position = 'fixed'
    textarea.style.left = '-9999px'
    document.body.appendChild(textarea)
    textarea.select()
    try {
      document.execCommand('copy')
    } finally {
      document.body.removeChild(textarea)
    }
  }
}

export function LongField({
  value,
  fallback = '-',
  title,
  className = '',
  mono = false,
  copyable = true,
  expandable = false,
  tone = 'default',
}: LongFieldProps) {
  const { t } = useI18n()
  const [copied, setCopied] = useState(false)
  const [expanded, setExpanded] = useState(false)
  const displayValue = value && value.length > 0 ? value : fallback
  const fullTitle = title ?? displayValue
  const canCopy = copyable && Boolean(value)
  const canExpand = expandable && displayValue.length > 36

  const copy = async () => {
    if (!value) return
    try {
      await writeClipboard(value)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    } catch {
      setCopied(false)
    }
  }

  return (
    <span className={`inline-flex max-w-full min-w-0 items-center gap-1 ${className}`}>
      <span
        title={fullTitle}
        className={`${mono ? 'font-mono' : ''} ${expanded ? 'whitespace-normal break-words' : 'truncate'} min-w-0`}
        style={{ color: toneColor(tone) }}
      >
        {displayValue}
      </span>
      {canCopy && (
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation()
            void copy()
          }}
          title={copied ? t('common.copied', 'Copied') : t('common.copy', 'Copy')}
          className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-md opacity-70 hover:opacity-100 focus-visible:opacity-100"
          style={{ color: copied ? 'var(--cp-success)' : 'var(--cp-muted)' }}
        >
          {copied ? <Check size={12} /> : <Copy size={12} />}
        </button>
      )}
      {canExpand && (
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation()
            setExpanded((value) => !value)
          }}
          title={expanded ? t('common.collapse', 'Collapse') : t('common.expand', 'Expand')}
          className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-md opacity-70 hover:opacity-100 focus-visible:opacity-100"
          style={{ color: 'var(--cp-muted)' }}
        >
          {expanded ? <ChevronUp size={12} /> : <ChevronDown size={12} />}
        </button>
      )}
    </span>
  )
}
