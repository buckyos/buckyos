import { MetricCard, PanelIntro } from '../../components/AppPanelPrimitives'
import { useI18n } from '../../i18n/provider'
import type { AppContentLoaderProps } from '../types'

export function DiagnosticsAppPanel({
  activityLog,
  layoutState,
  locale,
}: AppContentLoaderProps) {
  const { t } = useI18n()
  const totalItems = layoutState.pages.reduce((sum, page) => sum + page.items.length, 0)

  return (
    <div className="space-y-4">
      <PanelIntro
        kicker="Telemetry"
        title={t('diagnostics.title')}
        body={t('diagnostics.body')}
      />
      <div className="grid gap-3 md:grid-cols-3">
        <MetricCard label={t('diagnostics.locale')} tone="accent" value={locale} />
        <MetricCard label={t('files.pages')} tone="neutral" value={layoutState.pages.length} />
        <MetricCard label={t('files.items')} tone="success" value={totalItems} />
      </div>
      <div className="shell-subtle-panel p-4">
        <p className="shell-kicker">{t('shell.activity')}</p>
        <div className="mt-4 space-y-2">
          {activityLog.length > 0 ? (
            activityLog.map((entry) => (
              <div
                key={entry}
                className="flex items-start gap-3 rounded-[20px] bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] px-3 py-3 text-sm"
              >
                <span className="mt-1 h-2.5 w-2.5 rounded-full bg-[color:var(--cp-success)]" />
                <span>{entry}</span>
              </div>
            ))
          ) : (
            <div className="rounded-[20px] bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] px-4 py-4 text-sm text-[color:var(--cp-muted)]">
              {t('shell.noRunningApps')}
            </div>
          )}
        </div>
      </div>
      <pre className="shell-scrollbar overflow-x-auto rounded-[24px] bg-[color:var(--cp-bg-strong)] p-4 text-xs text-[color:var(--cp-text)]">
        {JSON.stringify(layoutState, null, 2)}
      </pre>
    </div>
  )
}
