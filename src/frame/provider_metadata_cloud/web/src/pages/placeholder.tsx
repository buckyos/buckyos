import { StatusBadge } from '../components/status/StatusBadge'
import { useI18n } from '../i18n/provider'

export function PlaceholderPage({ titleKey }: { titleKey: string }) {
  const { t } = useI18n()
  return (
    <div className="shell-card p-6">
      <div className="flex items-center justify-between gap-3">
        <h1 className="text-2xl font-bold">{t(titleKey, titleKey)}</h1>
        <StatusBadge tone="warning">Round 2+</StatusBadge>
      </div>
    </div>
  )
}
