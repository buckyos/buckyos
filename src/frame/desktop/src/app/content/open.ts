import { buckyos } from 'buckyos'
import { ContentRegistryClient, contentDescriptor, isTransferableRef, isTransferableSession, resolveContentHandlers, emptyDefaults, type OpenRequest, type HandlerPlan } from 'buckyos/content'
import type { AppFrameHost } from 'buckyos/app-frame'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import { openPreview } from '../preview/launch'
import type { PreviewOpenRequest } from '../preview/types'
import { isMockRuntime } from '../../runtime'
import { contentFixtureEnabled, mockContentRegistry } from '../../mock/content'
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
  if (!plan.appInstanceId) return 'Preview'
  const app = desktopUIStore.getSnapshot().apps.find(a => (a.appInstanceId ?? a.id) === plan.appInstanceId)
  return app?.labelKey ?? plan.appInstanceId.split('@')[0]
}
export async function openContent(request: PreviewOpenRequest, options: { handlerKey?: string; newWindow?: boolean; name?: string; mime?: string; size?: number } = {}): Promise<string | null> {
  if (!isTransferableRef(request.source) || request.session && !isTransferableSession(request.session)) return openPreview(request)
  const candidates = await contentHandlers(request, options)
  if (options.handlerKey && !candidates.some(p => p.handlerKey === options.handlerKey)) desktopUIStore.setSnackbar('The selected app is unavailable. Using the default app.')
  const selected = candidates.find(p => p.handlerKey === options.handlerKey) ?? candidates[0]
  if (!selected || selected.binding.entry.type === 'builtin') return openPreview({ ...request, newWindow: options.newWindow ?? request.newWindow })
  const open: OpenRequest = { requestId: crypto.randomUUID(), source: request.source, session: request.session,
    mode: selected.binding.modes?.includes('edit') ? 'edit' : 'view', origin: { appInstanceId: request.origin?.app, windowId: request.origin?.windowId, hostContext: request.origin?.hostContext } }
  const windows = desktopUIStore.getSnapshot().runtime.windows
  if (selected.binding.window === 'reuse' && !options.newWindow && !request.newWindow) {
    for (const window of [...windows].sort((a, b) => b.zIndex - a.zIndex)) {
      const frame = appFrames.get(window.id)
      if (frame?.handlerKey !== selected.handlerKey || !frame.host.ready) continue
      try {
        const result = await frame.host.open(open)
        if (result.accepted) { desktopUIStore.focusWindow(window.id); return window.id }
        if (result.fallback === 'preview') { desktopUIStore.setSnackbar(`Unable to open with ${handlerLabel(selected)}`); return openPreview(request) }
      } catch { continue }
    }
  }
  const app = desktopUIStore.getSnapshot().apps.find(a => (a.appInstanceId ?? a.id) === selected.appInstanceId)
  if (!app) return openPreview(request)
  return desktopUIStore.openAppWindow(app.id, { newInstance: true, title: options.name,
    launch: { requestId: open.requestId, payload: { kind: 'app-open', handlerKey: selected.handlerKey, request: open, binding: selected.binding } } }) ?? openPreview(request)
}

if (typeof window !== 'undefined') {
  window.addEventListener('focus', () => { void contentRegistry().catch(() => {}) })
  window.addEventListener('buckyos-apps-changed', () => { void refreshContentRegistry().catch(() => {}) })
}
