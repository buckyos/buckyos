import type { SettingsPage } from './navigation'
import { SettingsNavigationPanel } from './SettingsNavigationPanel'

interface MobileSettingsListProps {
  onNavigate: (page: SettingsPage) => void
}

export function MobileSettingsList({ onNavigate }: MobileSettingsListProps) {
  return (
    <div className="px-4 py-4">
      <SettingsNavigationPanel onNavigate={onNavigate} />
    </div>
  )
}
