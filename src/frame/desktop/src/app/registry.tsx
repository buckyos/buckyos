import { AICenterAppPanel } from './ai-center/AICenterAppPanel'
import { AppServiceAppPanel } from './app-service/AppServiceAppPanel'
import { CanvasAppPanel } from './canvas/CanvasAppPanel'
import { CodeAssistantAppPanel } from './codeassistant/CodeAssistantAppPanel'
import { DemosAppPanel } from './demos/DemosAppPanel'
import { DiagnosticsAppPanel } from './diagnostics/DiagnosticsAppPanel'
import { FileBrowserAppPanel } from './filebrowser/FileBrowserAppPanel'
import { HomeStationAppPanel } from './homestation/HomeStationAppPanel'
import { MarketAppPanel } from './market/MarketAppPanel'
import { MessageHubAppPanel } from './messagehub/MessageHubAppPanel'
import { MyNetworkAppPanel } from './my-network/MyNetworkAppPanel'
import { SettingsAppPanel } from './settings/SettingsAppPanel'
import { StudioAppPanel } from './studio/StudioAppPanel'
import { SystestAppPanel } from './systest/SystestAppPanel'
import { TaskCenterAppPanel } from './task-center/TaskCenterAppPanel'
import { UsersAgentsAppPanel } from './users-agents/UsersAgentsAppPanel'
import { WorkflowAppPanel } from './workflow/WorkflowAppPanel'
import { UnsupportedAppPanel } from './unsupported/UnsupportedAppPanel'
import {
  supportsFormFactor,
  type AppDefinition,
  type FormFactor,
} from '../models/ui'
import type { AppContentLoaderProps, DesktopAppItem } from './types'
import { desktopCatalogIdForLogicalApp } from './backend-apps'

const appLoaders = {
  'ai-center': AICenterAppPanel,
  'app-service': AppServiceAppPanel,
  canvas: CanvasAppPanel,
  settings: SettingsAppPanel,
  studio: StudioAppPanel,
  market: MarketAppPanel,
  diagnostics: DiagnosticsAppPanel,
  demos: DemosAppPanel,
  files: FileBrowserAppPanel,
  codeassistant: CodeAssistantAppPanel,
  messagehub: MessageHubAppPanel,
  'my-network': MyNetworkAppPanel,
  homestation: HomeStationAppPanel,
  systest: SystestAppPanel,
  'task-center': TaskCenterAppPanel,
  'users-agents': UsersAgentsAppPanel,
  workflow: WorkflowAppPanel,
} as const

export function resolveDesktopApps(
  apps: AppDefinition[],
  formFactor: FormFactor,
): DesktopAppItem[] {
  return apps
    .filter((app) => supportsFormFactor(app, formFactor))
    .map((app) => {
      const catalogId = desktopCatalogIdForLogicalApp(app.logicalAppId ?? app.id)
      const loader = appLoaders[catalogId as keyof typeof appLoaders]
        ?? (app.webHosts?.length ? SystestAppPanel : undefined)
      return { ...app, loader }
    })
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
