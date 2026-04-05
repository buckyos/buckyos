import { ChevronLeft } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import {
  getSettingsPageDefinition,
  getSettingsPageGroup,
  type SettingsPage,
} from './navigation'

interface MobileDetailHeaderProps {
  page: SettingsPage
  onBack: () => void
}

export function MobileDetailHeader({ page, onBack }: MobileDetailHeaderProps) {
  const { t } = useI18n()
  const pageDefinition = getSettingsPageDefinition(page)
  const group = getSettingsPageGroup(pageDefinition.group)

  return (
    <div className="shrink-0 px-4 pb-2 pt-4">
      <div className="shell-subtle-panel flex items-center gap-3 px-3 py-3">
        <button
          type="button"
          onClick={onBack}
          className="inline-flex items-center gap-1.5 rounded-full px-3 py-2 text-sm font-medium transition-colors active:opacity-70"
          style={{
            color: 'var(--cp-accent)',
            background: 'color-mix(in srgb, var(--cp-accent-soft) 12%, transparent)',
          }}
        >
          <ChevronLeft size={18} />
          <span>{t('settings.mobile.back', 'Settings')}</span>
        </button>
        <div className="min-w-0">
          <p className="shell-kicker">{t(group.labelKey, group.label)}</p>
          <p
            className="truncate text-sm font-semibold"
            style={{ color: 'var(--cp-text)' }}
          >
            {t(pageDefinition.labelKey, pageDefinition.label)}
          </p>
        </div>
      </div>
    </div>
  )
}
