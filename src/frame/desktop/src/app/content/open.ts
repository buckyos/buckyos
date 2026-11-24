import { buckyos } from 'buckyos'
import { ContentRegistryClient, contentDescriptor, handlerOfApp, isTransferableRef, isTransferableSession, resolveContentHandlers, emptyDefaults, type OpenRequest, type HandlerPlan } from 'buckyos/content'
import type { AppFrameHost } from 'buckyos/app-frame'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import { openPreview } from '../preview/launch'
import type { PreviewOpenRequest } from '../preview/types'
import { isMockRuntime } from '../../runtime'
import { contentFixtureEnabled, mockContentRegistry } from '../../mock/content'
import { translateNow } from '../../i18n/translate'
import { appTitleNow } from '../../presentation/appText'
import { standaloneFrame } from '../standalone/frame'
export const appFrames = new Map<string, { host: AppFrameHost; handlerKey?: string }>()
let registry: ContentRegistryClient | undefined
let userId: string | undefined
let refresh: Promise<void> | undefined
export async function contentRegistry(): Promise<ContentRegistryClient | undefined> {
  if (isMockRuntime()) {
    if (!contentFixtureEnabled()) return undefined
    if (!mockContentRegistry.updatedAt) await mockContentRegistry.refresh()
    return mockContentRegistry
  }
  const user = (await buckyos.getAccountInfo())?.user_id
  if (!user) return undefined
  if (!registry || userId !== user) { userId = user; registry = new ContentRegistryClient(buckyos.getSystemConfigClient(), user) }
  if (Date.now() - registry.updatedAt > 60000) {
    refresh ??= Promise.all([registry.refresh(), desktopUIStore.refreshApps()]).then(() => {}).finally(() => { refresh = undefined })
    await refresh
  }
  return registry
}
export async function refreshContentRegistry(): Promise<void> { if (registry) registry.updatedAt = 0; await contentRegistry() }
export async function contentHandlers(request: PreviewOpenRequest, hints: { name?: string; mime?: string; size?: number } = {}): Promise<HandlerPlan[]> {
  if (!isTransferableRef(request.source) || request.session && !isTransferableSession(request.session)) return []
  const client = await contentRegistry().catch(() => undefined)
  const snapshot = desktopUIStore.getSnapshot()
  return resolveContentHandlers(client?.registry ?? { schema_version: 1, handlers: {} }, client?.defaults ?? emptyDefaults(), contentDescriptor(request.source, hints), 'open', {}, snapshot.apps.map(app => app.appInstanceId ?? app.id))
}
export function handlerLabel(plan: HandlerPlan): string {
  if (!plan.appInstanceId) return translateNow('apps.preview', 'Preview')
  const app = desktopUIStore.getSnapshot().apps.find(a => (a.appInstanceId ?? a.id) === plan.appInstanceId)
  return app ? appTitleNow(app) : plan.appInstanceId.split('@')[0]
}
/** `exclude`: an App that asked the system to open this content elsewhere; none of its handlers is chosen for it. */
export async function openContent(request: PreviewOpenRequest, options: { handlerKey?: string; newWindow?: boolean; name?: string; mime?: string; size?: number; exclude?: string } = {}): Promise<string | null> {
  if (!isTransferableRef(request.source) || request.session && !isTransferableSession(request.session)) return openPreview(request)
  // In a window of the BuckyOS client the client knows which windows are open.
  if (!options.handlerKey && desktopUIStore.isStandalone() && standaloneFrame.openContent({ source: request.source, session: request.session }, { newWindow: options.newWindow ?? request.newWindow, hints: { name: options.name, mime: options.mime, size: options.size } })) return null
  const candidates = (await contentHandlers(request, options)).filter(plan => !options.exclude || !handlerOfApp(plan, options.exclude))
  if (options.handlerKey && !candidates.some(p => p.handlerKey === options.handlerKey)) desktopUIStore.setSnackbar(translateNow('webFrame.useDefault'))
  const selected = candidates.find(p => p.handlerKey === options.handlerKey) ?? candidates[0]
  if (!selected || selected.binding.entry.type === 'builtin') return openPreview({ ...request, newWindow: options.newWindow ?? request.newWindow })
  const open: OpenRequest = { requestId: crypto.randomUUID(), source: request.source, session: request.session,
    mode: selected.binding.modes?.includes('edit') ? 'edit' : 'view', origin: { appInstanceId: request.origin?.app, windowId: request.origin?.windowId, hostContext: request.origin?.hostContext } }
  const app = desktopUIStore.getSnapshot().apps.find(a => (a.appInstanceId ?? a.id) === selected.appInstanceId)
  if (!app) return openPreview(request)
  // An App that declares how many windows it has is held to it; otherwise its handler decides for the content it opens.
  const { instance } = app.manifest
  const newWindow = (instance ?? 'multi') === 'multi' && (selected.binding.window !== 'reuse' || options.newWindow === true || request.newWindow === true)
  const windows = desktopUIStore.getSnapshot().runtime.windows
  if (!newWindow) {
    for (const window of windows.filter(w => w.appId === app.id).sort((a, b) => b.lastActive - a.lastActive)) {
      const frame = appFrames.get(window.id)
      if (!frame?.host.ready || (instance !== 'single' && frame.handlerKey !== selected.handlerKey)) continue
      try {
        const result = await frame.host.open(open)
        if (result.accepted) { desktopUIStore.focusWindow(window.id); return window.id }
        if (result.fallback === 'preview') { desktopUIStore.setSnackbar(translateNow('webFrame.unableToOpenWith').replace('{{app}}', () => handlerLabel(selected))); return openPreview(request) }
      } catch { continue }
    }
  }
  return desktopUIStore.openAppWindow(app.id, { mode: 'new', multi: instance === undefined, title: options.name,
    launch: { requestId: open.requestId, payload: { kind: 'app-open', handlerKey: selected.handlerKey, request: open, binding: selected.binding } } }) ?? openPreview(request)
}

if (typeof window !== 'undefined') {
  window.addEventListener('focus', () => { void contentRegistry().catch(() => {}) })
  window.addEventListener('buckyos-apps-changed', () => { void refreshContentRegistry().catch(() => {}) })
}
