import type { ComponentType } from 'react'
import type {
  AppDefinition,
  LayoutState,
  SystemPreferencesInput,
  ThemeMode,
  WindowAppearancePreferences,
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
}

export type AppContentLoader = ComponentType<AppContentLoaderProps>

export interface DesktopAppItem extends AppDefinition {
  loader?: AppContentLoader
}
