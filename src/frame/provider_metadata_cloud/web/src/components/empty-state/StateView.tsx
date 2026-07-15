import { AlertTriangle, Loader2 } from 'lucide-react'
import { useI18n } from '../../i18n/provider'

export function LoadingView() {
  const { t } = useI18n()
  return (
    <div className="flex min-h-[260px] items-center justify-center gap-3 text-sm text-[color:var(--cp-muted)]">
      <Loader2 className="animate-spin" size={18} />
      {t('state.loading', 'Loading metadata workspace')}
    </div>
  )
}

export function ErrorView({ retry }: { retry: () => void }) {
  const { t } = useI18n()
  return (
    <div className="shell-card flex min-h-[260px] flex-col items-center justify-center gap-3 p-6 text-center">
      <AlertTriangle size={22} className="text-[color:var(--cp-danger)]" />
      <p className="text-sm font-semibold">{t('state.error', 'Unable to load mock metadata')}</p>
      <button className="rounded-md border border-[color:var(--cp-border)] px-3 py-2 text-sm" onClick={retry}>
        {t('action.retry', 'Retry')}
      </button>
    </div>
  )
}

export function EmptyView() {
  const { t } = useI18n()
  return (
    <div className="flex min-h-[160px] items-center justify-center rounded-md border border-dashed border-[color:var(--cp-border)] text-sm text-[color:var(--cp-muted)]">
      {t('state.empty', 'No records match the current filters')}
    </div>
  )
}
