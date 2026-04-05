import { useI18n } from '../../i18n/provider'
import type { AppContentLoaderProps } from '../types'

export function UnsupportedAppPanel({ app }: AppContentLoaderProps) {
  const { t } = useI18n()

  return (
    <div className="shell-subtle-panel p-4">
      <p>{t('common.unsupportedPanel', app.id)}</p>
    </div>
  )
}
