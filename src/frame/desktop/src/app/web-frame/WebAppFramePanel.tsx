import { useEffect, useMemo, useRef } from 'react'
import { buckyos } from 'buckyos'
import { AppFrameHost } from 'buckyos/app-frame'
import { expandOpenPath, isOpenRequest, type OpenBinding, type OpenRequest } from 'buckyos/content'
import { desktopUIStore, useDesktopRuntime } from '../../models/DesktopUIDataModel'
import { useMobileBackHandler } from '../../desktop/windows/MobileNavContext'
import type { AppContentLoaderProps } from '../types'
import { appFrames, openContent } from '../content/open'
import { openPreview } from '../preview/launch'
import { isMockRuntime } from '../../runtime'
export function WebAppFramePanel({ app, windowId, launch, themeMode, locale }: AppContentLoaderProps) {
  const iframe = useRef<HTMLIFrameElement>(null)
  const host = useRef<AppFrameHost | undefined>(undefined)
  const nonce = useMemo(() => crypto.randomUUID(), [])
  const runtime = useDesktopRuntime()
  const payload = launch?.payload as { kind?: string; handlerKey?: string; request?: OpenRequest; binding?: OpenBinding } | undefined
  const request = payload?.kind === 'app-open' && isOpenRequest(payload.request) ? payload.request : undefined
  const origin = useMemo(() => {
    const hostname = app.webHosts?.[0]
    if (!hostname || !/^[a-z0-9][a-z0-9.-]*$/i.test(hostname)) return undefined
    const zone = !isMockRuntime() ? buckyos.getZoneHostName() : 'localhost'
    return `${location.protocol}//${hostname}.${zone}${location.port ? `:${location.port}` : ''}`
  }, [app.webHosts])
  const src = useMemo(() => {
    if (!origin) return undefined
    try {
      const path = request && payload?.binding?.entry.type === 'web' ? expandOpenPath(payload.binding.entry.path, request, payload.binding.modes) : '/'
      const url = new URL(path, origin)
      if (url.origin !== origin) return undefined
      url.hash = `bfp=${nonce}`; return url.href
    } catch { return undefined }
  }, [origin, nonce, request, payload])
  const latest = useRef({ request, themeMode, locale })
  useEffect(() => { latest.current = { request, themeMode, locale } }, [request, themeMode, locale])
  useEffect(() => {
    if (!src && request) { desktopUIStore.setSnackbar(`Unable to open with ${app.labelKey}`); openPreview({ source: request.source, session: request.session }) }
  }, [src, request, app.labelKey])
  useEffect(() => {
    if (!origin || !windowId || !src) return
    const { request, themeMode, locale } = latest.current
    const frame = new AppFrameHost({ target: () => iframe.current?.contentWindow ?? null, origin,
      init: { nonce, windowId, theme: { mode: themeMode }, locale, formFactor: matchMedia('(max-width: 900px)').matches ? 'mobile' : 'desktop', launch: request, shellCapabilities: ['content.open', 'beforeClose', 'back'] },
      onTitle: (title, dirty) => desktopUIStore.updateWindow(windowId, { title: `${dirty ? '● ' : ''}${title}` }),
      onRequestClose: () => desktopUIStore.closeWindow(windowId, true),
      onContentOpen: (next, target, newWindow) => {
        const req = { source: next.source, session: next.session, origin: { app: app.appInstanceId ?? app.id, windowId }, newWindow }
        if (target === 'preview') openPreview(req); else void openContent(req)
      },
    })
    host.current = frame; appFrames.set(windowId, { host: frame, handlerKey: payload?.handlerKey })
    const unregister = desktopUIStore.registerCloseGuard(windowId, async (reason = 'user') => {
      try { return await frame.beforeClose(reason) === 'allow' }
      catch { return window.confirm(locale.startsWith('zh') ? '应用无响应，是否强制关闭？' : 'App is not responding. Force close?') }
    })
    return () => { unregister(); appFrames.delete(windowId); frame.dispose(); host.current = undefined }
  }, [origin, src, windowId, app.id, app.appInstanceId, nonce, payload?.handlerKey])
  useEffect(() => { host.current?.update({ theme: { mode: themeMode }, locale }); host.current?.send('frame.themeChanged', { mode: themeMode }); host.current?.send('frame.localeChanged', { locale }) }, [themeMode, locale])
  useEffect(() => {
    const top = [...runtime.windows].filter(w => w.state !== 'minimized').sort((a, b) => b.zIndex - a.zIndex)[0]
    host.current?.send('frame.focusChanged', { focused: top?.id === windowId })
  }, [runtime.windows, windowId])
  useMobileBackHandler(() => {
    void host.current?.back().then(handled => { if (!handled && windowId) desktopUIStore.closeWindow(windowId) }).catch(() => { if (windowId) desktopUIStore.closeWindow(windowId) })
    return true
  })
  if (!src) return <p>Unable to open {app.labelKey}</p>
  return <iframe ref={iframe} className="block h-full w-full border-0" data-testid="web-app-frame" src={src} title={app.labelKey} allow="clipboard-read; clipboard-write; fullscreen" onError={() => { if (request) { desktopUIStore.setSnackbar(`Unable to open with ${app.labelKey}`); openPreview({ source: request.source, session: request.session }) } }} />
}
