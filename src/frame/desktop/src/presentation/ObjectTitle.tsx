/* ── Object title / summary line: one truncation, tooltip and DID-copy rule for lists, cards and details ── */

import clsx from 'clsx'
import { Check, Copy } from 'lucide-react'
import { useState } from 'react'
import { useI18n } from '../i18n/provider'

export function ObjectTitle({
  title,
  description,
  did,
  showDid = false,
  size = 'md',
  className,
}: {
  title: string
  description?: string
  did?: string
  /** Show the full DID under the title with a copy button. */
  showDid?: boolean
  size?: 'sm' | 'md' | 'lg'
  className?: string
}) {
  const { t } = useI18n()
  const [copied, setCopied] = useState(false)
  const copy = () => {
    if (!did) return
    void navigator.clipboard?.writeText(did).then(() => {
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    }).catch(() => undefined)
  }
  const titleClass = size === 'lg' ? 'text-xl' : size === 'sm' ? 'text-[13px]' : 'text-sm'
  return (
    <div className={clsx('min-w-0', className)}>
      <div className={clsx('truncate font-display font-semibold text-[color:var(--cp-text)]', titleClass)} title={title}>
        {title}
      </div>
      {description ? (
        <div className="line-clamp-2 text-xs text-[color:var(--cp-muted)]" title={description}>
          {description}
        </div>
      ) : null}
      {showDid && did ? (
        <div className="mt-1 flex min-w-0 items-center gap-1">
          <code className="truncate rounded-[8px] px-2 py-0.5 text-[12px] text-[color:var(--cp-accent)] [background:color-mix(in_srgb,var(--cp-accent-soft)_14%,var(--cp-surface))]" title={did}>
            {did}
          </code>
          <button
            type="button"
            onClick={copy}
            aria-label={t('presentation.copyDid', 'Copy DID')}
            className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]"
          >
            {copied ? <Check size={12} /> : <Copy size={12} />}
          </button>
        </div>
      ) : null}
    </div>
  )
}
