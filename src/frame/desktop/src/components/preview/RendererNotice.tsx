/* The notice a renderer shows when it cannot display the content itself (password protected, too large). */

import { Download, ExternalLink, Lock } from 'lucide-react'
import type { ReactNode } from 'react'
import type { TranslateFn } from './pinchZoom'

const CHIP =
  'inline-flex items-center gap-1.5 rounded-full border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] px-3 py-1.5 text-[12px] font-medium text-[color:var(--cp-text)] hover:border-[color:var(--cp-accent)] hover:text-[color:var(--cp-accent)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color:var(--cp-focus-ring)]'

/** The content cannot be shown here (password protected, too large): what it is, and the ways out. */
export function RendererNotice({
  title,
  body,
  icon,
  testId,
  onDownload,
  onOpenWith,
  t,
}: {
  title: string
  body: string
  icon?: ReactNode
  testId: string
  onDownload: () => void
  onOpenWith?: () => void
  t: TranslateFn
}) {
  return (
    <div className="absolute inset-0 flex items-center justify-center p-6" data-testid={testId}>
      <div className="flex max-w-md flex-col items-center gap-3 text-center">
        <div className="flex h-16 w-16 items-center justify-center rounded-3xl bg-[color:color-mix(in_srgb,var(--cp-surface-2)_86%,transparent)] text-[color:var(--cp-muted)]">
          {icon ?? <Lock size={28} />}
        </div>
        <div className="text-[15px] font-semibold">{title}</div>
        <div className="text-[12px] text-[color:var(--cp-muted)]">{body}</div>
        <div className="mt-1 flex gap-2">
          <button type="button" className={CHIP} onClick={onDownload}>
            <Download size={13} /> {t('preview.action.download', 'Download original')}
          </button>
          {onOpenWith ? (
            <button type="button" className={CHIP} onClick={onOpenWith}>
              <ExternalLink size={13} /> {t('preview.action.openWith', 'Open with…')}
            </button>
          ) : null}
        </div>
      </div>
    </div>
  )
}
