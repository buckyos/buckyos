/* ── `OpenAppRequest.path` ↔ the launch a window of an app carries ──
 *
 * The pages each system app opens on (the table in product/desktop/BuckyOS_Web_Desktop_需求文档.md §27.9 is the same
 * list). A path an app does not know opens it as usual. An installed web App's path is a page of its own site.
 */

import { expandOpenPath, isOpenRequest, parseSystemOpen, type OpenBinding } from 'buckyos/content'
import { isAppPath } from 'buckyos/open-app'
import type { AppDefinition, WindowLaunch } from '../models/ui'
import { desktopCatalogIdForLogicalApp } from './backend-apps'
import { agentBrowserLaunchParams, agentBrowserLaunchSchema } from './agent-browser/launch'
import { agentSetupLaunchSchema } from './agent-setup/launch'
import { aiCenterLaunchSchema } from './ai-center/launch'
import { appServiceLaunchSchema } from './app-service/launch'
import { filesOpenRequest, type FilesOpenPayload } from './filebrowser/launch'
import { homeStationLaunchSchema } from './homestation/launch'
import { MESSAGE_HUB_LINK_KIND, messageHubLaunchSchema, messageHubLinkParams, messageHubLinkSchema } from './messagehub/launch'
import { myContentLaunchSchema } from './my-content/launch'
import { myNetworkLaunchSchema } from './my-network/launch'
import { previewPath } from './preview/path'
import { isPreviewLaunchPayload, PREVIEW_LAUNCH_KIND, type PreviewLaunchPayload } from './preview/types'
import { settingsLaunchSchema } from './settings/launch'
import { appPath } from './standalone/address'
import { taskCenterLaunchSchema } from './task-center/launch'
import { usersAgentsLaunchSchema } from './users-agents/launch'
import { workflowLaunchSchema } from './workflow/launch'

interface AppPages {
  /** The launch payload of a page; undefined opens the app as usual. */
  payload: (name: string, params: URLSearchParams) => unknown
  /** The page of a launch payload; undefined when the launch cannot be written as an address. */
  path: (payload: unknown) => string | undefined
}

const PATH_LIMIT = 4096

/** Parameters an app's schema names one to one. */
const given = (params: URLSearchParams, names: string[]) =>
  Object.fromEntries(names.flatMap((name) => (params.get(name) ? [[name, params.get(name)]] : [])))

const pages: Readonly<Record<string, AppPages>> = {
  settings: {
    payload: (name) => ({ page: name }),
    path: (payload) => {
      const launch = settingsLaunchSchema.safeParse(payload)
      return launch.success ? appPath(launch.data.page) : undefined
    },
  },
  'ai-center': {
    payload: (name) => ({ page: name }),
    path: (payload) => {
      const launch = aiCenterLaunchSchema.safeParse(payload)
      return launch.success ? appPath(launch.data.page) : undefined
    },
  },
  files: {
    payload: (name, params) => (name === 'upload'
      ? { kind: 'files-open', upload: true } satisfies FilesOpenPayload
      : params.get('path') ? { kind: 'files-open', path: params.get('path')! } satisfies FilesOpenPayload : undefined),
    path: (payload) => {
      const open = filesOpenRequest({ requestId: '', payload })
      return open && (open.upload ? appPath('upload') : appPath('', { path: open.path }))
    },
  },
  'task-center': {
    payload: (_name, params) => ({ taskId: params.get('taskid') }),
    path: (payload) => {
      const launch = taskCenterLaunchSchema.safeParse(payload)
      return launch.success ? appPath('', { taskid: launch.data.taskId }) : undefined
    },
  },
  'app-service': {
    payload: (name, params) => (name === 'install' ? { page: 'install' } : { page: 'detail', serviceId: params.get('service') }),
    path: (payload) => {
      const launch = appServiceLaunchSchema.safeParse(payload)
      if (!launch.success) return undefined
      return launch.data.page === 'install' ? appPath('install') : appPath('', { service: launch.data.serviceId })
    },
  },
  'users-agents': {
    payload: (name, params) => (name === 'self' ? { view: 'self' } : { entityId: params.get('entity') }),
    path: (payload) => {
      const launch = usersAgentsLaunchSchema.safeParse(payload)
      if (!launch.success) return undefined
      return 'view' in launch.data ? appPath('self') : appPath('', { entity: launch.data.entityId })
    },
  },
  'my-network': {
    payload: (name, params) => (name === 'requests' ? { view: 'requests' } : params.get('entity') ? { entity: params.get('entity') } : { action: name, ...given(params, ['input']) }),
    path: (payload) => {
      const launch = myNetworkLaunchSchema.safeParse(payload)
      if (!launch.success) return undefined
      if ('view' in launch.data) return appPath('requests')
      return 'entity' in launch.data ? appPath('', { entity: launch.data.entity }) : appPath(launch.data.action, { input: launch.data.input })
    },
  },
  'my-content': {
    payload: (name, params) => ({ ...(name ? { tab: name } : {}), ...(params.get('share') ? { shareId: params.get('share') } : {}) }),
    path: (payload) => {
      const launch = myContentLaunchSchema.safeParse(payload)
      return launch.success ? appPath(launch.data.tab ?? '', { share: launch.data.shareId }) : undefined
    },
  },
  'agent-browser': {
    payload: (_name, params) => given(params, ['agent', ...agentBrowserLaunchParams]),
    path: (payload) => {
      const launch = agentBrowserLaunchSchema.safeParse(payload)
      return launch.success ? appPath('', launch.data) : undefined
    },
  },
  'agent-setup': {
    payload: (_name, params) => given(params, ['source', 'agent_id']),
    path: (payload) => {
      const launch = agentSetupLaunchSchema.safeParse(payload)
      return launch.success ? appPath('', { source: launch.data.source, agent_id: launch.data.agent_id }) : undefined
    },
  },
  workflow: {
    payload: (name, params) => {
      if (name === 'prompt') return { kind: 'ai_prompt' }
      return params.get('run') ? { kind: 'run', runId: params.get('run') } : { kind: 'definition', definitionId: params.get('definition') }
    },
    path: (payload) => {
      const launch = workflowLaunchSchema.safeParse(payload)
      if (!launch.success) return undefined
      if (launch.data.kind === 'ai_prompt') return appPath('prompt')
      return launch.data.kind === 'run' ? appPath('', { run: launch.data.runId }) : appPath('', { definition: launch.data.definitionId })
    },
  },
  homestation: {
    payload: (_name, params) => {
      if (params.get('new') === 'post') return { newPost: true }
      try { return { compose: JSON.parse(params.get('compose') ?? '') } } catch { return undefined }
    },
    path: (payload) => {
      const launch = homeStationLaunchSchema.safeParse(payload)
      if (!launch.success) return undefined
      return 'newPost' in launch.data ? appPath('', { new: 'post' }) : appPath('', { compose: JSON.stringify(launch.data.compose) })
    },
  },
  messagehub: {
    payload: (_name, params) => ({ kind: MESSAGE_HUB_LINK_KIND, search: params.toString() }),
    path: (payload) => {
      const link = messageHubLinkSchema.safeParse(payload)
      if (link.success) return `/?${link.data.search}`
      const launch = messageHubLaunchSchema.safeParse(payload)
      return launch.success ? `/?${messageHubLinkParams(launch.data)}` : undefined
    },
  },
  preview: {
    payload: (_name, params) => {
      const open = parseSystemOpen(params)
      return open && ({
        kind: PREVIEW_LAUNCH_KIND,
        requestId: crypto.randomUUID(),
        source: open.request.source,
        session: open.request.session,
        createdBy: 'manual',
      } satisfies PreviewLaunchPayload)
    },
    path: (payload) => (isPreviewLaunchPayload(payload) ? previewPath(payload) : undefined),
  },
}

/** A page of an installed web App, for a window whose launch is not a content request. */
export interface AppPathPayload { kind: 'app-path'; path: string }
const APP_PATH_KIND = 'app-path'

function pagesOf(app: AppDefinition): AppPages | undefined {
  return pages[desktopCatalogIdForLogicalApp(app.logicalAppId ?? app.id)]
}

export function launchOfPath(app: AppDefinition, path: string | undefined): WindowLaunch | undefined {
  if (!path || !isAppPath(path)) return undefined
  const known = pagesOf(app)
  const requestId = crypto.randomUUID()
  if (!known) return app.webHosts?.length ? { requestId, payload: { kind: APP_PATH_KIND, path } satisfies AppPathPayload } : undefined
  const url = new URL(path, 'https://zone/')
  let name: string
  try { name = decodeURIComponent(url.pathname.replace(/^\/+|\/+$/g, '')) } catch { return undefined }
  const payload = known.payload(name, url.searchParams)
  return payload === undefined || known.path(payload) === undefined ? undefined : { requestId, payload }
}

/** The page a web App's window shows: the one its launch names, or the entry of a content request. */
export function webAppPath(launch: WindowLaunch | undefined): string | undefined {
  const payload = launch?.payload as { kind?: string; path?: unknown; request?: unknown; binding?: OpenBinding } | undefined
  if (payload?.kind === APP_PATH_KIND) return isAppPath(payload.path) ? payload.path : undefined
  if (payload?.kind !== 'app-open' || !isOpenRequest(payload.request) || payload.binding?.entry.type !== 'web') return undefined
  try { return expandOpenPath(payload.binding.entry.path, payload.request, payload.binding.modes) } catch { return undefined }
}

export function pathOfLaunch(app: AppDefinition, launch: WindowLaunch | undefined): string | undefined {
  if (!launch) return undefined
  const known = pagesOf(app)
  const path = known ? known.path(launch.payload) : webAppPath(launch)
  return path && path.length <= PATH_LIMIT ? path : undefined
}
