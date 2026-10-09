import clsx from 'clsx'
import { AlertTriangle, ChevronLeft, RefreshCw } from 'lucide-react'
import type { ReactNode } from 'react'
import { useI18n } from '../../../i18n/provider'
import type { IdentityView } from '../datamodel/types'

export function Avatar({ identity, size = 36, className }: { identity: Pick<IdentityView, 'name' | 'hue'>; size?: number; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={clsx('flex flex-shrink-0 items-center justify-center rounded-full font-semibold', className)}
      style={{
        width: size,
        height: size,
        fontSize: Math.round(size * 0.4),
        background: `color-mix(in srgb, hsl(${identity.hue} 60% 55%) 22%, var(--cp-surface))`,
        color: `color-mix(in srgb, hsl(${identity.hue} 60% 42%) 80%, var(--cp-text))`,
      }}
    >
      {identity.name.charAt(0).toUpperCase()}
    </span>
  )
}

export function ListSkeleton({ rows = 3 }: { rows?: number }) {
  const { t } = useI18n()
  return (
    <div role="status" aria-label={t('homestation.state.loading', 'Loading…')} data-testid="hs-loading">
      {Array.from({ length: rows }, (_, index) => (
        <div key={index} className="flex gap-3 px-4 py-4" style={{ borderBottom: '1px solid var(--hs-divider)' }}>
          <div className="hs-skeleton h-9 w-9 flex-shrink-0 rounded-full" />
          <div className="flex-1 space-y-2">
            <div className="hs-skeleton h-3 w-1/3" />
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
    <div className="flex flex-col items-center justify-center gap-2 px-6 py-14 text-center" data-testid="hs-empty">
      <span style={{ color: 'var(--cp-muted)' }}>{icon}</span>
      <p className="text-sm font-semibold">{title}</p>
      {body ? <p className="max-w-sm text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>{body}</p> : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  )
}

export function ErrorState({ onRetry, body }: { onRetry: () => void; body?: string }) {
  const { t } = useI18n()
  return (
    <div className="flex flex-col items-center justify-center gap-2 px-6 py-14 text-center" role="alert" data-testid="hs-error">
      <AlertTriangle size={32} strokeWidth={1.5} style={{ color: 'var(--cp-warning)' }} />
      <p className="text-sm font-semibold">{t('homestation.state.errorTitle', 'Could not load this view')}</p>
      <p className="max-w-sm text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>{body ?? t('homestation.state.errorBody', 'Your HomeStation did not answer in time. Nothing was changed.')}</p>
      <button type="button" className="hs-btn mt-2" onClick={onRetry}>
        <RefreshCw size={14} />
        {t('common.retry', 'Retry')}
      </button>
    </div>
  )
}

export function PageHeader({ title, subtitle, onBack, actions, backLabel }: { title: string; subtitle?: string; onBack?: () => void; actions?: ReactNode; backLabel?: string }) {
  return (
    <div className="flex items-center gap-2 px-3 py-2" style={{ borderBottom: '1px solid var(--hs-divider)' }}>
      {onBack ? (
        <button type="button" className="hs-icon-btn" aria-label={backLabel} onClick={onBack}>
          <ChevronLeft size={18} />
        </button>
      ) : null}
      <div className="min-w-0 flex-1 px-1">
        <h2 className="truncate text-sm font-semibold">{title}</h2>
        {subtitle ? <p className="truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>{subtitle}</p> : null}
      </div>
      {actions}
    </div>
  )
}
