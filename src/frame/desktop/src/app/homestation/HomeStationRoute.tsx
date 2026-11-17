import { useI18n } from '../../i18n/provider'
import { useDocumentTitle } from '../../presentation/documentHead'
import { HomeStationView } from './HomeStationView'

export function HomeStationRoute() {
  const { t } = useI18n()
  useDocumentTitle(t('apps.homestation'))
  return (
    <main className="h-dvh w-full overflow-hidden">
      <HomeStationView />
    </main>
  )
}
