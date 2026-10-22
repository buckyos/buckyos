import { BarChart3, Eye, Gauge, Home, Plug, HardDrive, GitFork, FlaskConical } from 'lucide-react'
import { useViewerIsAdmin } from '../../../settings/hooks/use-viewer-is-admin'

export const navItems = [
  { key: 'home', icon: Home, labelKey: 'aiCenter.nav.home', label: 'Home', adminOnly: false },
  { key: 'providers', icon: Plug, labelKey: 'aiCenter.nav.providers', label: 'Providers', adminOnly: false },
  { key: 'models', icon: HardDrive, labelKey: 'aiCenter.nav.models', label: 'Models', adminOnly: false },
  { key: 'routing', icon: GitFork, labelKey: 'aiCenter.nav.routing', label: 'Routing', adminOnly: false },
  { key: 'playground', icon: FlaskConical, labelKey: 'aiCenter.playground.title', label: 'Playground', adminOnly: false },
  { key: 'usage', icon: BarChart3, labelKey: 'aiCenter.nav.usage', label: 'Usage', adminOnly: false },
  { key: 'quota', icon: Gauge, labelKey: 'aiCenter.nav.quota', label: 'Quota', adminOnly: false },
  { key: 'visibility', icon: Eye, labelKey: 'aiCenter.nav.visibility', label: 'Visibility', adminOnly: true },
] as const

export type AICenterPage = (typeof navItems)[number]['key'] | 'providers/add'

/** Navigation entries the viewer may open (Visibility is for administrators). */
export function useNavItems() {
  const isAdmin = useViewerIsAdmin() === true
  return navItems.filter((item) => !item.adminOnly || isAdmin)
}
