export type DesktopShortcutDefinition = {
  id: string
  label: string
  icon: IconName
  tile: string
  windowId: DesktopWindowId
}

export type DesktopWindowId =
  | 'chat'
  | 'monitor'
  | 'network'
  | 'containers'
  | 'files'
  | 'storage'
  | 'logs'
  | 'apps'
  | 'settings'
  | 'users'

export const DESKTOP_SHORTCUTS: DesktopShortcutDefinition[] = [
  {
    id: 'chat',
    label: 'Chat',
    icon: 'message',
    tile: 'bg-emerald-600',
    windowId: 'chat',
  },
  {
    id: 'monitor',
    label: 'Monitor',
    icon: 'dashboard',
    tile: 'bg-blue-500',
    windowId: 'monitor',
  },
  {
    id: 'network',
    label: 'Network',
    icon: 'network',
    tile: 'bg-indigo-500',
    windowId: 'network',
  },
  {
    id: 'containers',
    label: 'Containers',
    icon: 'container',
    tile: 'bg-cyan-600',
    windowId: 'containers',
  },
  {
    id: 'files',
    label: 'Files',
    icon: 'drive',
    tile: 'bg-emerald-500',
    windowId: 'files',
  },
  {
    id: 'storage',
    label: 'Storage',
    icon: 'storage',
    tile: 'bg-teal-500',
    windowId: 'storage',
  },
  {
    id: 'logs',
    label: 'System Logs',
    icon: 'chart',
    tile: 'bg-orange-500',
    windowId: 'logs',
  },
  {
    id: 'apps',
    label: 'Apps',
    icon: 'apps',
    tile: 'bg-sky-500',
    windowId: 'apps',
  },
  {
    id: 'settings',
    label: 'Settings',
    icon: 'settings',
    tile: 'bg-gray-600',
    windowId: 'settings',
  },
  {
    id: 'users',
    label: 'Users',
    icon: 'users',
    tile: 'bg-green-500',
    windowId: 'users',
  },
]
