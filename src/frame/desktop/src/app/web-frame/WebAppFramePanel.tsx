import { useEffect, useMemo, useRef } from 'react'
import { buckyos } from 'buckyos'
import { AppFrameHost } from 'buckyos/app-frame'
import { contentDescriptor, expandOpenPath, isOpenRequest, type OpenBinding, type OpenRequest } from 'buckyos/content'
import { desktopUIStore, useDesktopRuntime } from '../../models/DesktopUIDataModel'
import { useMobileBackHandler } from '../../desktop/windows/MobileNavContext'
import type { AppContentLoaderProps } from '../types'
import { appFrames, openContent } from '../content/open'
import { requestOpenWith } from '../content/OpenWithSheet'
import { openPreview } from '../preview/launch'
import { isMockRuntime } from '../../runtime'
import { useAppText } from '../../presentation/appText'
import { useAppLaunch } from './launch'
import { AppLaunchStatus } from './AppLaunchStatus'
export function WebAppFramePanel({ app, windowId, launch, themeMode, locale }: AppContentLoaderProps) {
  const iframe = useRef<HTMLIFrameElement>(null)
  const title = useAppText()(app).title
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
  const appLaunch = useAppLaunch(app.appInstanceId ?? app.id, src)
  const frameUrl = appLaunch.outcome?.kind === 'ready' ? appLaunch.outcome.url : undefined
  const unopenable = !src || appLaunch.outcome?.kind === 'invalid_target'
  const latest = useRef({ request, themeMode, locale })
  useEffect(() => { latest.current = { request, themeMode, locale } }, [request, themeMode, locale])
  useEffect(() => {
    if (unopenable && request) { desktopUIStore.setSnackbar(`Unable to open with ${title}`); openPreview({ source: request.source, session: request.session }) }
  }, [unopenable, request, title])
  useEffect(() => {
    if (!origin || !windowId || !frameUrl) return
    const { request, themeMode, locale } = latest.current
    const frame = new AppFrameHost({ target: () => iframe.current?.contentWindow ?? null, origin,
      init: { nonce, windowId, theme: { mode: themeMode }, locale, formFactor: matchMedia('(max-width: 900px)').matches ? 'mobile' : 'desktop', launch: request, shellCapabilities: ['content.open', 'beforeClose', 'back'] },
      onTitle: (title, dirty) => desktopUIStore.updateWindow(windowId, { title: `${dirty ? '● ' : ''}${title}` }),
      onRequestClose: () => desktopUIStore.closeWindow(windowId, true),
      onContentOpen: (next, target, { newWindow, hints }) => {
        const requester = app.appInstanceId ?? app.id
        const req = { source: next.source, session: next.session, origin: { app: requester, windowId }, newWindow }
        if (target === 'preview') openPreview(req)
        else if (target === 'chooser') requestOpenWith({ source: next.source, session: next.session, name: hints.name ?? contentDescriptor(next.source).name ?? '', mediaType: hints.mime, size: hints.size, exclude: requester, origin: req.origin })
        else void openContent(req, { name: hints.name, mime: hints.mime, size: hints.size })
      },
    })
    host.current = frame; appFrames.set(windowId, { host: frame, handlerKey: payload?.handlerKey })
    const unregister = desktopUIStore.registerCloseGuard(windowId, async (reason = 'user') => {
      try { return await frame.beforeClose(reason) === 'allow' }
      catch { return window.confirm(locale.startsWith('zh') ? '应用无响应，是否强制关闭？' : 'App is not responding. Force close?') }
    })
    return () => { unregister(); appFrames.delete(windowId); frame.dispose(); host.current = undefined }
  }, [origin, frameUrl, windowId, app.id, app.appInstanceId, nonce, payload?.handlerKey])
  useEffect(() => { host.current?.update({ theme: { mode: themeMode }, locale }); host.current?.send('frame.themeChanged', { mode: themeMode }); host.current?.send('frame.localeChanged', { locale }) }, [themeMode, locale])
  // A click inside the (cross-origin) iframe never reaches the shell document, so the window
  // container's pointer handling cannot raise this window. What the shell does observe is that
  // its own window loses focus while the iframe element becomes the active element.
  useEffect(() => {
    if (!windowId) return
    const onBlur = () => { if (document.activeElement === iframe.current) desktopUIStore.focusWindow(windowId) }
    window.addEventListener('blur', onBlur)
    return () => window.removeEventListener('blur', onBlur)
  }, [windowId])
  useEffect(() => {
    const top = [...runtime.windows].filter(w => w.state !== 'minimized').sort((a, b) => b.zIndex - a.zIndex)[0]
    host.current?.setFocused(top?.id === windowId)
  }, [runtime.windows, windowId, frameUrl])
  useMobileBackHandler(() => {
    void host.current?.back().then(handled => { if (!handled && windowId) desktopUIStore.closeWindow(windowId) }).catch(() => { if (windowId) desktopUIStore.closeWindow(windowId) })
    return true
  })
  if (!src) return <p>Unable to open {title}</p>
  if (!frameUrl) return <AppLaunchStatus outcome={appLaunch.outcome} title={title} onRetry={appLaunch.retry} />
  return <iframe ref={iframe} className="block h-full w-full border-0" data-testid="web-app-frame" src={frameUrl} title={title} allow="clipboard-read; clipboard-write; fullscreen" onError={() => { if (request) { desktopUIStore.setSnackbar(`Unable to open with ${title}`); openPreview({ source: request.source, session: request.session }) } }} />
}
