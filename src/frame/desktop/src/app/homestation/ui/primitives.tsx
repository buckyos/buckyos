import clsx from 'clsx'
import { AlertTriangle, ArrowLeft, RefreshCw } from 'lucide-react'
import { forwardRef, type CSSProperties, type ReactNode } from 'react'
import { useI18n } from '../../../i18n/provider'
import type { IdentityView } from '../datamodel/types'
import { initialOf, usableImageUrl } from '../../../presentation/model'
import { ObjectImage } from '../../../presentation/ObjectIcon'
import { useDidPresentation } from '../../../presentation/useDidPresentation'

/** The identity's own picture, else the one its DID document names (shared presentation cache), else the initial on its hue. */
export function Avatar({ identity, size = 36, className }: { identity: Pick<IdentityView, 'name' | 'hue' | 'avatar'> & Partial<Pick<IdentityView, 'did' | 'kind'>>; size?: number; className?: string }) {
  const own = usableImageUrl(identity.avatar)
  const document = useDidPresentation(own ? undefined : identity.did, identity.kind === 'agent' ? 'agent' : 'user')
  return (
    <ObjectImage
      icons={[own, document?.icons[0]]}
      className={clsx('flex-shrink-0 rounded-full object-cover', className)}
      style={{ width: size, height: size, boxShadow: '0 0 0 1px var(--hs-line)' }}
      fallback={<InitialAvatar identity={identity} size={size} className={className} />}
    />
  )
}

function InitialAvatar({ identity, size, className }: { identity: Pick<IdentityView, 'name' | 'hue'>; size: number; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={clsx('flex flex-shrink-0 items-center justify-center rounded-full font-medium', className)}
      style={{
        width: size,
        height: size,
        fontSize: Math.round(size * 0.42),
        background: `color-mix(in srgb, hsl(${identity.hue} 45% 55%) 16%, var(--hs-column))`,
        color: `color-mix(in srgb, hsl(${identity.hue} 45% 38%) 70%, var(--cp-text))`,
      }}
    >
      {initialOf(identity.name)}
    </span>
  )
}

export function ListSkeleton({ rows = 3 }: { rows?: number }) {
  const { t } = useI18n()
  return (
    <div role="status" aria-label={t('homestation.state.loading', 'Loading…')} data-testid="hs-loading">
      {Array.from({ length: rows }, (_, index) => (
        <div key={index} className="hs-card hs-card-row">
          <div className="hs-skeleton h-9 w-9 flex-shrink-0 rounded-full" />
          <div className="flex-1 space-y-3 pt-1">
            <div className="hs-skeleton h-3 w-1/4" />
            <div className="hs-skeleton h-3 w-5/6" />
            <div className="hs-skeleton h-3 w-2/3" />
          </div>
        </div>
      ))}
    </div>
  )
}

export function EmptyState({ icon, title, body, action }: { icon: ReactNode; title: string; body?: string; action?: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 px-6 py-16 text-center" data-testid="hs-empty">
      <span className="hs-empty-icon mb-2" style={{ color: 'var(--cp-muted)' }}>{icon}</span>
      <p className="text-[15px] font-semibold">{title}</p>
      {body ? <p className="max-w-sm text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{body}</p> : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  )
}

export function ErrorState({ onRetry, body }: { onRetry: () => void; body?: string }) {
  const { t } = useI18n()
  return (
    <div className="flex flex-col items-center justify-center gap-2 px-6 py-16 text-center" role="alert" data-testid="hs-error">
      <span className="hs-empty-icon mb-2" style={{ color: 'var(--hs-warning-ink)' }}><AlertTriangle size={28} /></span>
      <p className="text-[15px] font-semibold">{t('homestation.state.errorTitle', 'Could not load this view')}</p>
      <p className="max-w-sm text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{body ?? t('homestation.state.errorBody', 'Your HomeStation did not answer in time. Nothing was changed.')}</p>
      <button type="button" className="hs-btn mt-2" onClick={onRetry}>
        <RefreshCw size={16} />
        {t('common.retry', 'Retry')}
      </button>
    </div>
  )
}

/** The header of a column: back on the left, the title in the middle, the column's own actions on the right. */
export function PageHeader({ title, subtitle, onBack, actions, backLabel, leading }: { title: ReactNode; subtitle?: string; onBack?: () => void; actions?: ReactNode; backLabel?: string; leading?: ReactNode }) {
  return (
    <header className="hs-column-head flex-shrink-0">
      <div className="flex min-w-0 items-center justify-start">
        {onBack ? (
          <button type="button" className="hs-icon-btn" aria-label={backLabel} onClick={onBack}>
            <ArrowLeft size={20} />
          </button>
        ) : leading}
      </div>
      <div className="hs-column-head-title">
        <h2 className="truncate text-[15px] font-semibold leading-5">{title}</h2>
        {subtitle ? <p className="truncate text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{subtitle}</p> : null}
      </div>
      <div className="flex min-w-0 items-center justify-end gap-1">{actions}</div>
    </header>
  )
}

/** The reading column under a header: a card on the page on desktop, the whole screen on a phone. It is the scroll container. */
export const ColumnBody = forwardRef<HTMLDivElement, { children: ReactNode; className?: string; style?: CSSProperties; testId?: string }>(function ColumnBody({ children, className, style, testId }, ref) {
  return (
    <div ref={ref} className={clsx('hs-column-body desktop-scrollbar relative min-h-0 flex-1 overflow-y-auto', className)} style={style} data-testid={testId}>
      {children}
    </div>
  )
})
