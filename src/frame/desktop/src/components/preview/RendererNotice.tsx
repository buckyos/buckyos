/* The notice a renderer shows when it cannot display the content itself (password protected, too large). */

import { Download, ExternalLink, Lock } from 'lucide-react'
import type { ReactNode } from 'react'
import type { TranslateFn } from './pinchZoom'

const CHIP = 'files-btn'

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
        <div className="text-[color:var(--cp-muted)]">
          {icon ?? <Lock size={32} strokeWidth={1.5} />}
        </div>
        <div className="mt-1 text-[16px] font-medium">{title}</div>
        <div className="text-[13px] leading-5 text-[color:var(--cp-muted)]">{body}</div>
        <div className="mt-1 flex gap-2">
          <button type="button" className={CHIP} onClick={onDownload}>
            <Download size={16} /> {t('preview.action.download', 'Download original')}
          </button>
          {onOpenWith ? (
            <button type="button" className={CHIP} onClick={onOpenWith}>
              <ExternalLink size={16} /> {t('preview.action.openWith', 'Open with…')}
            </button>
          ) : null}
        </div>
      </div>
    </div>
  )
}
