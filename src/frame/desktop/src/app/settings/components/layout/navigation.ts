import { Code, FolderLock, Globe, HardDrive, KeyRound, Laptop, Network, Package, Palette, Settings, Shield } from 'lucide-react'

/** Stable id of the Agent Runtimes page; other apps open Settings on it (`openSettings`). */
export const AGENT_RUNTIMES_SETTINGS_PAGE = 'agent-runtimes'

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
    key: 'storage',
    group: 'system',
    icon: HardDrive,
    labelKey: 'settings.nav.storage',
    label: 'Storage',
  },
  {
    key: 'appearance',
    group: 'system',
    icon: Palette,
    labelKey: 'settings.nav.appearance',
    label: 'Appearance',
  },
  {
    key: 'zone',
    group: 'system',
    icon: Globe,
    labelKey: 'settings.nav.zone',
    label: 'Zone Config',
  },
  {
    key: 'cluster',
    group: 'network-security',
    icon: Network,
    labelKey: 'settings.nav.cluster',
    label: 'Cluster Manager',
  },
  {
    key: AGENT_RUNTIMES_SETTINGS_PAGE,
    group: 'network-security',
    icon: Laptop,
    labelKey: 'settings.nav.agentRuntimes',
    label: 'Agent Runtimes',
  },
  {
    key: 'privacy',
    group: 'network-security',
    icon: Shield,
    labelKey: 'settings.nav.privacy',
    label: 'Privacy',
  },
  {
    key: 'file-permissions',
    group: 'network-security',
    icon: FolderLock,
    labelKey: 'settings.nav.filePermissions',
    label: 'File Permissions',
  },
  {
    key: 'sso',
    group: 'network-security',
    icon: KeyRound,
    labelKey: 'settings.nav.sso',
    label: 'Sign-in & Sharing',
    adminOnly: true,
  },
  {
    key: 'ext-tools',
    group: 'advanced',
    icon: Package,
    labelKey: 'settings.nav.extTools',
    label: 'Extension Tools',
    adminOnly: true,
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

export function isSettingsPage(value: unknown): value is SettingsPage {
  return settingsPageDefinitions.some((item) => item.key === value)
}

export function getSettingsPageDefinition(page: SettingsPage) {
  return settingsPageDefinitions.find((item) => item.key === page) ?? settingsPageDefinitions[0]
}

export function getSettingsPageGroup(group: SettingsPageGroup) {
  return settingsPageGroups.find((item) => item.key === group) ?? settingsPageGroups[0]
}
