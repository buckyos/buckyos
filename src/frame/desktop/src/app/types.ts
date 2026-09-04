import type { ComponentType } from 'react'
import type {
  AppDefinition,
  LayoutState,
  SystemPreferencesInput,
  ThemeMode,
  WindowAppearancePreferences,
  WindowLaunch,
} from '../models/ui'

export interface AppContentLoaderProps {
  activityLog: string[]
  app: AppDefinition
  layoutState: LayoutState
  locale: string
  onSaveSettings: (values: SystemPreferencesInput) => void
  runtimeContainer: string
  themeMode: ThemeMode
  windowAppearance: WindowAppearancePreferences
  /** Hosting window id (absent for standalone routes). */
  windowId?: string
  /** Launch request the window was opened / re-targeted with. */
  launch?: WindowLaunch
}

export type AppContentLoader = ComponentType<AppContentLoaderProps>

export interface DesktopAppItem extends AppDefinition {
  loader?: AppContentLoader
}
