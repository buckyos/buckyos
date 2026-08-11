import { PanelIntro } from '../../../../components/AppPanelPrimitives'
import { useI18n } from '../../../../i18n/provider'
import {
  getSettingsPageDefinition,
  getSettingsPageGroup,
  type SettingsPage,
} from '../layout/navigation'

interface SettingsPageIntroProps {
  page: SettingsPage
  title: string
  description: string
}

export function SettingsPageIntro({
  page,
  title,
  description,
}: SettingsPageIntroProps) {
  const { t } = useI18n()
  const pageDefinition = getSettingsPageDefinition(page)
  const group = getSettingsPageGroup(pageDefinition.group)

  return (
    <PanelIntro
      kicker={t(group.labelKey, group.label)}
      title={title}
      body={description}
    />
  )
}
