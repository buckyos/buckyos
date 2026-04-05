import { AICenterAppPanel } from './ai-center/AICenterAppPanel'
import { AppServiceAppPanel } from './app-service/AppServiceAppPanel'
import { CodeAssistantAppPanel } from './codeassistant/CodeAssistantAppPanel'
import { DemosAppPanel } from './demos/DemosAppPanel'
import { DiagnosticsAppPanel } from './diagnostics/DiagnosticsAppPanel'
import { HomeStationAppPanel } from './homestation/HomeStationAppPanel'
import { MarketAppPanel } from './market/MarketAppPanel'
import { MessageHubAppPanel } from './messagehub/MessageHubAppPanel'
import { SettingsAppPanel } from './settings/SettingsAppPanel'
import { StudioAppPanel } from './studio/StudioAppPanel'
import { TaskCenterAppPanel } from './task-center/TaskCenterAppPanel'
import { UsersAgentsAppPanel } from './users-agents/UsersAgentsAppPanel'
import { UnsupportedAppPanel } from './unsupported/UnsupportedAppPanel'
import {
  supportsFormFactor,
  type AppDefinition,
  type FormFactor,
} from '../models/ui'
import type { AppContentLoaderProps, DesktopAppItem } from './types'

const appLoaders = {
  'ai-center': AICenterAppPanel,
  'app-service': AppServiceAppPanel,
  settings: SettingsAppPanel,
  studio: StudioAppPanel,
  market: MarketAppPanel,
  diagnostics: DiagnosticsAppPanel,
  demos: DemosAppPanel,
  codeassistant: CodeAssistantAppPanel,
  messagehub: MessageHubAppPanel,
  homestation: HomeStationAppPanel,
  'task-center': TaskCenterAppPanel,
  'users-agents': UsersAgentsAppPanel,
} as const

export function resolveDesktopApps(
  apps: AppDefinition[],
  formFactor: FormFactor,
): DesktopAppItem[] {
  return apps
    .filter((app) => supportsFormFactor(app, formFactor))
    .map((app) => ({
      ...app,
      loader: appLoaders[app.id as keyof typeof appLoaders],
    }))
}

export function findDesktopAppById(
  apps: DesktopAppItem[],
  appId: string,
) {
  return apps.find((app) => app.id === appId)
}

export function AppContentRenderer(props: AppContentLoaderProps & { app: DesktopAppItem }) {
  const Loader = props.app.loader ?? UnsupportedAppPanel
  return <Loader {...props} />
}
