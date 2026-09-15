/* eslint-disable react-refresh/only-export-components */
import { lazy, memo, Suspense, type ComponentType } from 'react'
import {
  supportsFormFactor,
  type AppDefinition,
  type FormFactor,
} from '../models/ui'
import type { AppContentLoader, AppContentLoaderProps, DesktopAppItem } from './types'
import { desktopCatalogIdForLogicalApp } from './backend-apps'
import { UnsupportedAppPanel } from './unsupported/UnsupportedAppPanel'
import { LoadingOrb } from '../components/LoadingOrb'

// Every app panel is code-split: the desktop shell only needs the loader
// table at boot, and the panel module is fetched the first time a window for
// that app is opened. Statically importing all panels here pulled the whole
// application (~3.4 MB of JS) into the entry chunk.
function lazyPanel<TModule>(
  load: () => Promise<TModule>,
  pick: (module: TModule) => ComponentType<AppContentLoaderProps>,
): AppContentLoader {
  return lazy(() => load().then((module) => ({ default: pick(module) })))
}

const SystestAppPanel = lazyPanel(
  () => import('./systest/SystestAppPanel'),
  (m) => m.SystestAppPanel,
)

const appLoaders: Record<string, AppContentLoader> = {
  'ai-center': lazyPanel(() => import('./ai-center/AICenterAppPanel'), (m) => m.AICenterAppPanel),
  'app-service': lazyPanel(() => import('./app-service/AppServiceAppPanel'), (m) => m.AppServiceAppPanel),
  canvas: lazyPanel(() => import('./canvas/CanvasAppPanel'), (m) => m.CanvasAppPanel),
  settings: lazyPanel(() => import('./settings/SettingsAppPanel'), (m) => m.SettingsAppPanel),
  studio: lazyPanel(() => import('./studio/StudioAppPanel'), (m) => m.StudioAppPanel),
  market: lazyPanel(() => import('./market/MarketAppPanel'), (m) => m.MarketAppPanel),
  diagnostics: lazyPanel(() => import('./diagnostics/DiagnosticsAppPanel'), (m) => m.DiagnosticsAppPanel),
  demos: lazyPanel(() => import('./demos/DemosAppPanel'), (m) => m.DemosAppPanel),
  files: lazyPanel(() => import('./filebrowser/FileBrowserAppPanel'), (m) => m.FileBrowserAppPanel),
  codeassistant: lazyPanel(() => import('./codeassistant/CodeAssistantAppPanel'), (m) => m.CodeAssistantAppPanel),
  messagehub: lazyPanel(() => import('./messagehub/MessageHubAppPanel'), (m) => m.MessageHubAppPanel),
  'my-network': lazyPanel(() => import('./my-network/MyNetworkAppPanel'), (m) => m.MyNetworkAppPanel),
  preview: lazyPanel(() => import('./preview/PreviewAppPanel'), (m) => m.PreviewAppPanel),
  homestation: lazyPanel(() => import('./homestation/HomeStationAppPanel'), (m) => m.HomeStationAppPanel),
  systest: SystestAppPanel,
  'task-center': lazyPanel(() => import('./task-center/TaskCenterAppPanel'), (m) => m.TaskCenterAppPanel),
  'users-agents': lazyPanel(() => import('./users-agents/UsersAgentsAppPanel'), (m) => m.UsersAgentsAppPanel),
  workflow: lazyPanel(() => import('./workflow/WorkflowAppPanel'), (m) => m.WorkflowAppPanel),
}

export function resolveDesktopApps(
  apps: AppDefinition[],
  formFactor: FormFactor,
): DesktopAppItem[] {
  return apps
    .filter((app) => supportsFormFactor(app, formFactor))
    .map((app) => {
      const catalogId = desktopCatalogIdForLogicalApp(app.logicalAppId ?? app.id)
      const loader = appLoaders[catalogId]
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

function AppContentFallback() {
  return (
    <div
      aria-busy="true"
      className="flex h-full min-h-[160px] w-full items-center justify-center"
    >
      <LoadingOrb size={44} />
    </div>
  )
}

/**
 * Memoised so that shell-level re-renders (window drag/resize, status bar
 * ticks, sidebar toggles) do not re-render every open app panel. All props
 * are either primitives or objects whose identity the store keeps stable.
 */
export const AppContentRenderer = memo(function AppContentRenderer(
  props: AppContentLoaderProps & { app: DesktopAppItem },
) {
  const Loader = props.app.loader ?? UnsupportedAppPanel
  return (
    <Suspense fallback={<AppContentFallback />}>
      <Loader {...props} />
    </Suspense>
  )
})
