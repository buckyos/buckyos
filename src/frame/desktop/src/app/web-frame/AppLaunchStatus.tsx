import { LoadingOrb } from '../../components/LoadingOrb'
import { useI18n } from '../../i18n/provider'
import type { AppLaunchOutcome } from './launch'

/** What a web App window shows instead of its iframe until the launch is ready. */
export function AppLaunchStatus({ outcome, title, onRetry }: { outcome: AppLaunchOutcome | undefined; title: string; onRetry: () => void }) {
  const { t } = useI18n()
  if (!outcome || outcome.kind === 'ready' || outcome.kind === 'signed_out') {
    return (
      <div aria-busy="true" data-testid="app-launch-pending" className="flex h-full min-h-[160px] w-full items-center justify-center">
        <LoadingOrb size={44} label={t('appLaunch.opening', undefined, { title })} />
      </div>
    )
  }
  const retryable = outcome.kind === 'session_inactive' || outcome.kind === 'failed'
  const message = {
    access_denied: t('appLaunch.accessDenied', undefined, { title }),
    invalid_target: t('appLaunch.invalidTarget', undefined, { title }),
    session_inactive: t('appLaunch.sessionInactive', undefined, { title }),
    failed: t('appLaunch.failed', undefined, { title }),
  }[outcome.kind]
  return (
    <div role="alert" data-testid="app-launch-error" data-launch-status={outcome.kind} className="flex h-full min-h-[160px] w-full flex-col items-center justify-center gap-2 p-6 text-center text-sm" style={{ color: 'var(--cp-muted)' }}>
      <p>{message}</p>
      {retryable && <button type="button" className="min-h-11 px-3 underline" style={{ color: 'var(--cp-text)' }} onClick={onRetry}>{t('common.retry')}</button>}
    </div>
  )
}
