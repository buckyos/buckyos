import { useI18n } from '../../../i18n/provider'
import { runtimeEvidenceValid } from '../mock/store'
import type { AppServiceItem } from '../types'

export function RuntimeSummary({ service }: { service: AppServiceItem }) {
  const { t } = useI18n()
  const runtime = service.runtime
  const valid = runtimeEvidenceValid(runtime)
  const status =
    runtime.status === 'running' && !valid ? 'unknown' : runtime.status
  const rows: Array<[string, string]> =
    runtime.type === 'docker'
      ? [
          [
            'appService.detail.dockerEngine',
            valid ? (runtime.docker?.engine ?? 'unknown') : 'unknown',
          ],
          [
            'appService.detail.image',
            valid ? (runtime.docker?.image ?? 'unknown') : 'unknown',
          ],
          [
            'appService.detail.container',
            valid ? (runtime.docker?.container ?? 'unknown') : 'unknown',
          ],
        ]
      : runtime.type === 'script'
        ? [['app22.processHealth', valid ? runtime.process_health : 'UNKNOWN']]
        : runtime.type === 'web'
          ? [
              [
                'app22.webAccessible',
                valid ? runtime.web_accessible : 'UNKNOWN',
              ],
            ]
          : runtime.type === 'agent'
            ? [
                [
                  'app22.agentEnvironment',
                  valid ? runtime.environment_ready : 'UNKNOWN',
                ],
              ]
            : [['app22.diagnostics', 'UNKNOWN']]
  return (
    <section
      data-testid="app-service-runtime-summary"
      className="rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface)] p-4"
    >
      <h3 className="text-sm font-semibold">{t('app22.runtimeResult')}</h3>
      <p className="mt-3 text-sm" data-testid="app-service-runtime-status">
        {t(`app22.status.${status}`, t('app22.unknown'))}
      </p>
      <p className="mt-1 text-xs text-[var(--cp-muted)]">
        {t(`app22.runtimeType.${runtime.type}`, t('app22.unknown'))}
      </p>
      <dl className="mt-3 space-y-3 text-xs">
        {rows.map(([key, value]) => (
          <div key={key} className="flex flex-wrap justify-between gap-2">
            <dt>{t(key)}</dt>
            <dd>
              {t(
                `app22.evidence.${value}`,
                t(`appService.stateLabel.${value}`, t('app22.unknown')),
              )}
            </dd>
          </div>
        ))}
      </dl>
      {runtime.type === 'agent' && (
        <p className="mt-3 text-xs leading-5">
          {runtime.agent_bindings === 0
            ? t('app22.noBinding')
            : t('app22.bindingCount', undefined, {
                count: runtime.agent_bindings ?? 0,
              })}
        </p>
      )}
      {runtime.reason && (
        <p className="mt-3 text-xs leading-5">
          {t(`app22.error.${runtime.reason}`, t('app22.unknown'))}
        </p>
      )}
      {service.operation_error && (
        <p role="alert" className="mt-3 text-sm text-[var(--cp-danger)]">
          {t(`app22.error.${service.operation_error}`)}
        </p>
      )}
    </section>
  )
}
