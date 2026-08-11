import { PanelIntro } from '../../components/AppPanelPrimitives'
import { useI18n } from '../../i18n/provider'
import type { AppContentLoaderProps } from '../types'

export function StudioAppPanel(_: AppContentLoaderProps) {
  const { t } = useI18n()

  return (
    <div className="space-y-4">
      <PanelIntro
        kicker="Manifest"
        title={t('studio.title')}
        body={t('studio.body')}
      />
      <div className="grid gap-3 md:grid-cols-2">
        {[
          'studio.point1',
          'studio.point2',
          'studio.point3',
          'studio.point4',
        ].map((key, index) => (
          <div key={key} className="shell-subtle-panel p-4 text-sm leading-6 text-[color:var(--cp-text)]">
            <div className="mb-3 flex items-center gap-3">
              <span className="flex h-8 w-8 items-center justify-center rounded-full bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_16%,var(--cp-surface))] font-display text-sm font-semibold text-[color:var(--cp-accent)]">
                {index + 1}
              </span>
              <span className="shell-kicker">Rule</span>
            </div>
            {t(key)}
          </div>
        ))}
      </div>
    </div>
  )
}
