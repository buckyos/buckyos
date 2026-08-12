import { AppIcon, TierBadge } from '../../components/DesktopVisuals'
import { PanelIntro } from '../../components/AppPanelPrimitives'
import { useI18n } from '../../i18n/provider'
import type { AppContentLoaderProps } from '../types'

export function MarketAppPanel(_: AppContentLoaderProps) {
  const { t } = useI18n()
  const cards = [
    {
      id: 'settings',
      labelKey: 'apps.settings',
      bodyKey: 'market.card.settings.body',
      iconKey: 'settings',
      accent: 'var(--cp-accent)',
      tier: 'system' as const,
    },
    {
      id: 'files',
      labelKey: 'apps.files',
      bodyKey: 'market.card.files.body',
      iconKey: 'files',
      accent: 'var(--cp-success)',
      tier: 'sdk' as const,
    },
    {
      id: 'studio',
      labelKey: 'apps.studio',
      bodyKey: 'market.card.studio.body',
      iconKey: 'studio',
      accent: 'var(--cp-warning)',
      tier: 'sdk' as const,
    },
    {
      id: 'docs',
      labelKey: 'apps.docs',
      bodyKey: 'market.card.docs.body',
      iconKey: 'docs',
      accent: 'var(--cp-accent-soft)',
      tier: 'external' as const,
    },
    {
      id: 'demos',
      labelKey: 'apps.demos',
      bodyKey: 'market.card.demos.body',
      iconKey: 'demos',
      accent: 'var(--cp-accent-soft)',
      tier: 'sdk' as const,
    },
  ]

  return (
    <div className="space-y-4">
      <PanelIntro
        kicker="Launcher"
        title={t('market.title')}
        body={t('market.body')}
      />
      <div className="grid gap-3 sm:grid-cols-2">
        {cards.map((app) => (
          <div key={app.id} className="shell-subtle-panel p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="flex items-start gap-3">
                <span
                  className="flex h-12 w-12 items-center justify-center rounded-[18px] border shadow-[0_14px_28px_color-mix(in_srgb,var(--cp-shadow)_12%,transparent)]"
                  style={{
                    borderColor: `color-mix(in srgb, ${app.accent} 26%, white)`,
                    background: `linear-gradient(165deg, color-mix(in srgb, ${app.accent} 78%, white), color-mix(in srgb, ${app.accent} 24%, var(--cp-bg)))`,
                  }}
                >
                  <AppIcon iconKey={app.iconKey} className="text-white" />
                </span>
                <div>
                  <p className="font-display text-lg font-semibold">{t(app.labelKey)}</p>
                  <p className="mt-1 text-sm leading-6 text-[color:var(--cp-muted)]">
                    {t(app.bodyKey)}
                  </p>
                </div>
              </div>
              <TierBadge tier={app.tier} />
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
