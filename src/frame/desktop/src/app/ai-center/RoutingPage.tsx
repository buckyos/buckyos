import { useMediaQuery } from '@mui/material'
import { useI18n } from '../../i18n/provider'
import { useLogicalModels } from './hooks/use-mock-store'
import { StatusBadge } from './components/shared/StatusBadge'

export function RoutingPage() {
  const { t } = useI18n()
  const models = useLogicalModels()
  const isMobile = useMediaQuery('(max-width: 767px)')

  if (models.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>
          {t('aiCenter.routing.notConfigured', 'Not configured')}
        </p>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-6">
      <h2 className="text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
        {t('aiCenter.routing.title', 'AI Routing Configuration')}
      </h2>

      {isMobile ? (
        // Mobile: card list
        <div className="flex flex-col gap-3">
          {models.map((lm) => {
            const configured = !!lm.resolved_model
            return (
              <div
                key={lm.name}
                className="rounded-xl p-4"
                style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
              >
                <div className="flex justify-between items-center mb-2">
                  <span className="text-sm font-mono font-medium" style={{ color: 'var(--cp-text)' }}>
                    {lm.name}
                  </span>
                  <StatusBadge
                    status={configured ? 'ok' : 'warning'}
                  />
                </div>
                <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                  {lm.resolved_model ?? t('aiCenter.routing.notConfigured', 'Not configured')}
                </div>
              </div>
            )
          })}
        </div>
      ) : (
        // Desktop: table
        <div
          className="rounded-xl overflow-hidden"
          style={{ border: '1px solid var(--cp-border)' }}
        >
          <table className="w-full">
            <thead>
              <tr style={{ background: 'var(--cp-surface)' }}>
                <th className="text-left text-xs font-medium px-4 py-3" style={{ color: 'var(--cp-muted)' }}>
                  {t('aiCenter.routing.logicalModel', 'Logical Model')}
                </th>
                <th className="text-left text-xs font-medium px-4 py-3" style={{ color: 'var(--cp-muted)' }}>
                  {t('aiCenter.routing.currentModel', 'Current Model')}
                </th>
                <th className="text-left text-xs font-medium px-4 py-3" style={{ color: 'var(--cp-muted)' }}>
                  {t('aiCenter.routing.status', 'Status')}
                </th>
              </tr>
            </thead>
            <tbody>
              {models.map((lm) => {
                const configured = !!lm.resolved_model
                return (
                  <tr
                    key={lm.name}
                    style={{ borderTop: '1px solid var(--cp-border)' }}
                  >
                    <td className="px-4 py-3 text-sm font-mono" style={{ color: 'var(--cp-text)' }}>
                      {lm.name}
                    </td>
                    <td className="px-4 py-3 text-sm" style={{ color: configured ? 'var(--cp-text)' : 'var(--cp-muted)' }}>
                      {lm.resolved_model ?? t('aiCenter.routing.notConfigured', 'Not configured')}
                    </td>
                    <td className="px-4 py-3">
                      <StatusBadge status={configured ? 'ok' : 'warning'} />
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      )}

      <div className="text-sm" style={{ color: 'var(--cp-muted)' }}>
        {t('aiCenter.routing.advancedHint', 'Want to customize routing strategy?')}{' '}
        <button
          type="button"
          disabled
          className="font-medium opacity-50 cursor-not-allowed"
          style={{ color: 'var(--cp-accent)' }}
        >
          {t('aiCenter.routing.advancedCta', 'Enter Advanced Mode')}
        </button>
      </div>
    </div>
  )
}
