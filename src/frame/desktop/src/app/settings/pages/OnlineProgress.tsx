import type { ExtToolsOnlineStatus } from '../../../api/settings'
import { useI18n } from '../../../i18n/provider'
import { formatBytes } from '../components/shared/format'
import { onlinePercent } from './extToolsOnline'

/** How far a download of an online item got: a bar, and the bytes under it. */
export function OnlineProgress({ status, testId }: { status: ExtToolsOnlineStatus; testId?: string }) {
  const { t } = useI18n()
  const percent = onlinePercent(status)
  return (
    <div className="flex w-full min-w-0 flex-col gap-1" data-testid={testId}>
      <div className="h-1.5 w-full overflow-hidden rounded-full bg-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)]" role="progressbar" aria-valuenow={percent ?? undefined} aria-valuemin={0} aria-valuemax={100}>
        <div className="h-full rounded-full bg-[color:var(--cp-accent)]" style={{ width: `${percent ?? 0}%`, transition: 'width 300ms ease' }} />
      </div>
      {status.size_bytes > 0 ? (
        <span className="text-xs text-[color:var(--cp-muted)]">
          {t('settings.extTools.online.bytes', '{{done}} of {{total}}', { done: formatBytes(status.downloaded_bytes), total: formatBytes(status.size_bytes) })}
        </span>
      ) : null}
    </div>
  )
}
