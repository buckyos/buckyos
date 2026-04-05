import { Code, Network, Palette, Settings, Shield } from 'lucide-react'

export const settingsPageGroups = [
  {
    key: 'system',
    labelKey: 'settings.mobile.system',
    label: 'System',
  },
  {
    key: 'network-security',
    labelKey: 'settings.mobile.networkSecurity',
    label: 'Network & Security',
  },
  {
    key: 'advanced',
    labelKey: 'settings.mobile.advanced',
    label: 'Advanced',
  },
] as const

export const settingsPageDefinitions = [
  {
    key: 'general',
    group: 'system',
    icon: Settings,
    labelKey: 'settings.nav.general',
    label: 'General',
  },
  {
    key: 'appearance',
    group: 'system',
    icon: Palette,
    labelKey: 'settings.nav.appearance',
    label: 'Appearance',
  },
  {
    key: 'cluster',
    group: 'network-security',
    icon: Network,
    labelKey: 'settings.nav.cluster',
    label: 'Cluster Manager',
  },
  {
    key: 'privacy',
    group: 'network-security',
    icon: Shield,
    labelKey: 'settings.nav.privacy',
    label: 'Privacy',
  },
  {
    key: 'developer',
    group: 'advanced',
    icon: Code,
    labelKey: 'settings.nav.developer',
    label: 'Developer Mode',
  },
] as const

export type SettingsPage = (typeof settingsPageDefinitions)[number]['key']
export type SettingsPageGroup = (typeof settingsPageGroups)[number]['key']

export function getSettingsPageDefinition(page: SettingsPage) {
  return settingsPageDefinitions.find((item) => item.key === page) ?? settingsPageDefinitions[0]
}

export function getSettingsPageGroup(group: SettingsPageGroup) {
  return settingsPageGroups.find((item) => item.key === group) ?? settingsPageGroups[0]
}
