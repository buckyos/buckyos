import { useEffect, useMemo, useRef, useState } from 'react'
import { buckyos, getRuntimeEnvironment } from 'buckyos'
import { appFrameName, AppFrameHost, TRANSFER_CAPABILITY } from 'buckyos/app-frame'
import { contentDescriptor, isOpenRequest, type OpenRequest } from 'buckyos/content'
import { desktopUIStore, useDesktopRuntime } from '../../models/DesktopUIDataModel'
import { useZoneAccent } from '../../desktop/appearance/accent'
import { useMobileBackHandler } from '../../desktop/windows/MobileNavContext'
import type { AppContentLoaderProps } from '../types'
import { appFrames, openContent } from '../content/open'
import { webAppPath } from '../launchPath'
import { openAppRequest } from '../openApp'
import { requestOpenWith } from '../content/OpenWithSheet'
import { openPreview } from '../preview/launch'
import { isMockRuntime } from '../../runtime'
import { useI18n } from '../../i18n/provider'
import { useAppText } from '../../presentation/appText'
import { currentZoneHost, useZonePresentation } from '../../presentation/zone'
import { useAppLaunch } from './launch'
import { AppLaunchStatus } from './AppLaunchStatus'
import { frameTransfer } from './transfer'
import { SELECT_DIALOG_PATH, useSystemDialog, type SelectDialogParams } from '../../sysdlg'

/** System dialogs an App may ask the shell to show over its window (`sysdlg.open`). */
const APP_DIALOGS: ReadonlySet<string> = new Set([SELECT_DIALOG_PATH])
export function WebAppFramePanel({ app, windowId, launch: requested, themeMode, locale }: AppContentLoaderProps) {
  const [launch, setLaunch] = useState(requested)
  const iframe = useRef<HTMLIFrameElement>(null)
  const { t } = useI18n()
  const title = useAppText()(app).title
  const host = useRef<AppFrameHost | undefined>(undefined)
  const nonce = useMemo(() => crypto.randomUUID(), [])
  const runtime = useDesktopRuntime()
  const payload = launch?.payload as { kind?: string; handlerKey?: string; request?: OpenRequest } | undefined
  const request = payload?.kind === 'app-open' && isOpenRequest(payload.request) ? payload.request : undefined
  const origin = useMemo(() => {
    const hostname = app.webHosts?.[0]
    if (!hostname || !/^[a-z0-9][a-z0-9.-]*$/i.test(hostname)) return undefined
    const zone = !isMockRuntime() ? buckyos.getZoneHostName() : 'localhost'
    return `${location.protocol}//${hostname}.${zone}${location.port ? `:${location.port}` : ''}`
  }, [app.webHosts])
  const src = useMemo(() => {
    if (!origin) return undefined
    const path = webAppPath(launch) ?? (request ? undefined : '/')
    if (!path) return undefined
    try {
      const url = new URL(path, origin)
      if (url.origin !== origin) return undefined
      // Older Apps read the nonce from the fragment when their entry has none of its own.
      if (!url.href.includes('#')) url.hash = `bfp=${nonce}`
      return url.href
    } catch { return undefined }
  }, [origin, nonce, request, launch])
  const appLaunch = useAppLaunch(app.appInstanceId ?? app.id, src, `${nonce}/${launch?.requestId ?? ''}`)
  const frameUrl = appLaunch.outcome?.kind === 'ready' ? appLaunch.outcome.url : undefined
  const unopenable = !src || appLaunch.outcome?.kind === 'invalid_target'
  const systemDialog = useSystemDialog()
  const zoneDid = useZonePresentation().did
  const accent = useZoneAccent()
  const latest = useRef({ request, themeMode, accent, locale, title, systemDialog, t, zoneDid })
  useEffect(() => { latest.current = { request, themeMode, accent, locale, title, systemDialog, t, zoneDid } }, [request, themeMode, accent, locale, title, systemDialog, t, zoneDid])
  // Another page in this window: the App is asked first, as when the window is closed.
  useEffect(() => {
    if (requested === launch) return
    let live = true
    const leave = async () => {
      if (!host.current?.ready) return true
      try { return await host.current.beforeClose('user') === 'allow' }
      catch { return window.confirm(latest.current.t('webFrame.forceClose')) }
    }
    void leave().then(allowed => { if (live && allowed) setLaunch(requested) })
    return () => { live = false }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once for each launch the window is given
  }, [requested])
  useEffect(() => {
    if (unopenable && request) { desktopUIStore.setSnackbar(t('webFrame.unableToOpenWith', undefined, { app: title })); openPreview({ source: request.source, session: request.session }) }
  }, [unopenable, request, title, t])
  useEffect(() => {
    if (!origin || !windowId || !frameUrl) return
    const { request, themeMode, accent, locale } = latest.current
    const zoneHost = currentZoneHost()
    const transfer = frameTransfer({ windowId, app: app.appInstanceId ?? app.id, frame: () => iframe.current })
    const frame = new AppFrameHost({ target: () => iframe.current?.contentWindow ?? null, origin,
      init: { nonce, windowId, theme: { mode: themeMode, accent }, locale, formFactor: matchMedia('(max-width: 900px)').matches ? 'mobile' : 'desktop', launch: request, presentation: 'window', container: getRuntimeEnvironment().container,
        shellCapabilities: ['content.open', 'app.open', 'beforeClose', 'back', SELECT_DIALOG_PATH, TRANSFER_CAPABILITY] },
      ...transfer.callbacks,
      onTitle: (title, dirty) => desktopUIStore.updateWindow(windowId, { title: `${dirty ? '● ' : ''}${title}` }),
      onRequestClose: () => desktopUIStore.closeWindow(windowId, true),
      onContentOpen: (next, target, { newWindow, hints }) => {
        const requester = app.appInstanceId ?? app.id
        const req = { source: next.source, session: next.session, origin: { app: requester, windowId }, newWindow }
        if (target === 'preview') openPreview(req)
        else if (target === 'chooser') requestOpenWith({ source: next.source, session: next.session, name: hints.name ?? contentDescriptor(next.source).name ?? '', mediaType: hints.mime, size: hints.size, exclude: requester, origin: req.origin })
        else void openContent(req, { name: hints.name, mime: hints.mime, size: hints.size })
      },
      onAppOpen: ({ zone, ...open }) => {
        const own = !zone || zone === latest.current.zoneDid || zone === zoneHost || zone === `did:web:${zoneHost}`
        return own ? openAppRequest(open) : { ok: false, code: 'unsupported' }
      },
      // The App is untrusted: the path must be on the list, the parameters must pass the dialog's schema, and the
      // name shown as the requester is the system's own. One dialog at a time per window is enforced by the host.
      onSystemDialog: async (path, params) => {
        if (!APP_DIALOGS.has(path)) return { ok: false, code: 'unsupported' }
        let pending
        try { pending = latest.current.systemDialog.open(SELECT_DIALOG_PATH, params as SelectDialogParams, { id: app.appInstanceId ?? app.id, name: latest.current.title }) }
        catch { return { ok: false, code: 'invalid_params' } }
        try { return { ok: true, result: await pending } }
        finally { iframe.current?.focus() }
      },
    })
    transfer.bind(frame)
    host.current = frame; appFrames.set(windowId, { host: frame, handlerKey: payload?.handlerKey })
    const unregister = desktopUIStore.registerCloseGuard(windowId, async (reason = 'user') => {
      try { return await frame.beforeClose(reason) === 'allow' }
      catch { return window.confirm(latest.current.t('webFrame.forceClose')) }
    })
    return () => { unregister(); appFrames.delete(windowId); frame.dispose(); transfer.dispose(); host.current = undefined }
  }, [origin, frameUrl, windowId, app.id, app.appInstanceId, nonce, payload?.handlerKey])
  useEffect(() => { host.current?.update({ theme: { mode: themeMode, accent }, locale }); host.current?.send('frame.themeChanged', { mode: themeMode, accent }); host.current?.send('frame.localeChanged', { locale }) }, [themeMode, accent, locale])
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
  if (!src) return <p>{t('webFrame.unableToOpen', undefined, { app: title })}</p>
  if (!frameUrl) return <AppLaunchStatus outcome={appLaunch.outcome} title={title} onRetry={appLaunch.retry} />
  return <iframe ref={iframe} name={appFrameName(nonce)} className="block h-full w-full border-0" data-testid="web-app-frame" src={frameUrl} title={title} allow="clipboard-read; clipboard-write; fullscreen" onError={() => { if (request) { desktopUIStore.setSnackbar(t('webFrame.unableToOpenWith', undefined, { app: title })); openPreview({ source: request.source, session: request.session }) } }} />
}
