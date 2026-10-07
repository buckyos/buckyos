import { createRoot } from 'react-dom/client'
import { buckyos } from 'buckyos'
import { NfspClient } from 'buckyos/nfsp'
import { AppFrameClient, type FrameInit } from 'buckyos/app-frame'
import { type OpenRequest } from 'buckyos/content'
import { App, handleBack } from './ui/App.tsx'
import { Workspace, dirty } from './model/workspace.ts'
import { APP_ID, initializeAppData } from './fs/appData.ts'
import { NfspDocumentStore } from './fs/documentStore.ts'
import { BufferStore } from './fs/bufferStore.ts'
import { RecoveryStore } from './fs/recoveryStore.ts'
import { LocalStore } from './fs/localStore.ts'
import { SettingsStore } from './platform/settings.ts'
import { parseLaunch } from './platform/launch.ts'
import { translate } from './i18n/index.ts'
import './ui/styles.css'
const root = createRoot(document.getElementById('root')!)
let workspace: Workspace | undefined
let initial: FrameInit | undefined
const pending: OpenRequest[] = []
const t = (key: string) => translate(navigator.language, key)
const shellOrigin = import.meta.env.DEV ? (import.meta.env.VITE_SHELL_ORIGIN || location.origin) : `${location.protocol}//sys.${location.hostname.split('.').slice(1).join('.')}${location.port ? `:${location.port}` : ''}`
const frame = new AppFrameClient({ shellOrigin,
  onInit: init => { initial = init; if (workspace) { applyFrame(init); if (init.launch) workspace.run(open(init.launch)) } },
  onOpen: async request => { if (!workspace) pending.push(request); else await open(request); return { accepted: true } },
  onTheme: theme => { if (workspace) { workspace.theme = theme.mode; workspace.emit() } },
  onLocale: locale => { if (workspace) { workspace.locale = locale; workspace.emit() } },
  onBeforeClose: async reason => {
    if (!workspace) return 'allow'
    if (reason === 'logout') return await workspace.prepareClose(undefined, reason) ? 'allow' : 'deny'
    if (![...workspace.documents.values()].some(dirty)) return 'allow'
    workspace.run(workspace.prepareClose(undefined, reason).then(allow => { if (allow) frame.requestClose() }))
    return 'pending'
  },
  onBack: () => workspace ? handleBack(workspace) : false,
  onFocus: focused => { if (focused && workspace) workspace.run(workspace.checkDisk()) },
})
function applyFrame(init: FrameInit): void {
  if (!workspace) return
  workspace.locale = init.locale; workspace.theme = init.theme.mode; workspace.origin.windowId = init.windowId
  if (init.formFactor === 'mobile') workspace.sidebar = false
  workspace.emit()
}
async function open(request: OpenRequest): Promise<void> {
  try { await workspace!.open(request) }
  catch (error) { workspace!.notify(String(error instanceof Error ? error.message : error)); throw error }
}
// The SDK renews the browser session by itself (`getAccountInfo()` refreshes the token when it is
// about to expire and keeps a renew timer). The app only has to notice when that fails for good:
// the gateway then clears the SSO cookies, `getAccountInfo()` turns null and nothing short of a new
// sign-in brings the session back.
let leavingForLogin = false
function setSessionLost(lost: boolean): void {
  if (!workspace || workspace.sessionLost === lost) return
  workspace.sessionLost = lost
  if (!lost) workspace.sessionLostMuted = false
  workspace.emit()
}
async function sessionToken(test: boolean): Promise<string | null> {
  if (test) return 'test-token'
  const token = (await buckyos.getAccountInfo())?.session_token ?? null
  setSessionLost(!token)
  return token
}
async function relogin(): Promise<void> {
  if (workspace) for (const doc of workspace.documents.values()) await workspace.persistLocal(doc)
  leavingForLogin = true
  await buckyos.login()
}
function renderLogin(): void {
  root.render(<div className="welcome"><h1>{t('title')}</h1><p>{t('loginNeeded')}</p><button onClick={() => { void buckyos.login().catch(showError) }}>{t('login')}</button><button onClick={() => window.open(location.href, '_blank', 'noopener')}>{t('loginPopup')}</button></div>)
}
async function start(): Promise<void> {
  const test = import.meta.env.MODE === 'test'
  // Do not show the sign-in screen before the session has actually been checked: the SDK may still be
  // refreshing a valid session, and a "Sign in" click at that moment forces a needless full SSO round trip.
  root.render(<div className="welcome"><h1>{t('title')}</h1><p className="muted">{t('connecting')}</p></div>)
  if (!test) await buckyos.initBuckyOS(APP_ID)
  const account = test ? { user_id: 'test-user', session_token: 'test-token' } : await buckyos.getAccountInfo()
  if (!account?.user_id) { if (window.parent !== window) await buckyos.login(); else renderLogin(); return }
  const client = new NfspClient({ baseUrl: location.origin, uploadChunkSize: 4 * 1024 * 1024, sessionToken: () => sessionToken(test) })
  const store = new NfspDocumentStore(client)
  const data = await initializeAppData(store, account.user_id)
  const local = new LocalStore(account.user_id)
  const config = test ? { get: async (key: string) => ({ value: localStorage.getItem(key) ?? '{}' }), set: async (key: string, value: string) => { localStorage.setItem(key, value) } } : buckyos.getSystemConfigClient()
  workspace = new Workspace(store, new BufferStore(data, store, local), new RecoveryStore(data, store), local, new SettingsStore(config, account.user_id))
  workspace.onLogin = relogin
  root.render(<App workspace={workspace} />)
  workspace.onExternalOpen = (source, target) => {
    if (frame.init) frame.openContent({ source, session: workspace?.session }, target)
    else window.open(`${shellOrigin}/?open=${encodeURIComponent(source.kind === 'cyfs-path' ? source.path : `obj://${source.objectId}`)}&intent=${target}`, '_blank', 'noopener')
  }
  workspace.subscribe(() => {
    const title = `${workspace!.active?.name ?? t('title')} — Text Editor`
    const changed = [...workspace!.documents.values()].some(dirty)
    document.title = (changed ? '● ' : '') + title; frame.setTitle(title, changed)
  })
  await workspace.start()
  if (initial) applyFrame(initial)
  if (window.parent !== window && !initial) await new Promise(resolve => setTimeout(resolve, 1500))
  const request = initial?.launch ?? parseLaunch(location.href)
  if (request) workspace.run(open(request))
  for (const request of pending.splice(0)) workspace.run(open(request))
  workspace.run(workspace.recovery.cleanup(workspace.settings.recoveryRetentionDays))
  const recheck = () => { if (document.visibilityState !== 'visible') return; workspace!.run(workspace!.checkDisk()); if (!test) void sessionToken(test).catch(() => {}) }
  document.addEventListener('visibilitychange', recheck)
  window.addEventListener('focus', recheck)
  window.addEventListener('online', () => { for (const doc of workspace!.documents.values()) workspace!.run(workspace!.flush(doc)) })
  window.addEventListener('beforeunload', event => {
    if (!leavingForLogin && [...workspace!.documents.values()].some(dirty)) { for (const doc of workspace!.documents.values()) workspace!.run(workspace!.persistLocal(doc)); event.preventDefault(); event.returnValue = '' }
  })
  window.addEventListener('pagehide', () => { for (const doc of workspace!.documents.values()) workspace!.run(workspace!.persistLocal(doc)) })
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', event => { if (!frame.init) { workspace!.theme = event.matches ? 'dark' : 'light'; workspace!.emit() } })
  setInterval(recheck, 15000)
  let watch: ReturnType<NfspClient['watch']> | undefined; let watchKey = ''; let timer: ReturnType<typeof setTimeout>
  const updateWatch = () => {
    clearTimeout(timer)
    timer = setTimeout(() => workspace!.run((async () => {
      const paths = [...new Set([...workspace!.documents.values()].flatMap(d => d.source?.kind === 'cyfs-path' ? [d.source.path.slice(0, d.source.path.lastIndexOf('/'))] : []))].sort()
      if (paths.join('|') === watchKey) return
      watchKey = paths.join('|'); watch?.close()
      const tokens = (await Promise.all(paths.map(path => client.list(path, { limit: 1 })))).flatMap(l => l.watch_token ? [l.watch_token] : [])
      if (!tokens.length) return
      watch = client.watch({ tokens })
      try { for await (const _event of watch) await workspace!.checkDisk() }
      catch { watchKey = ''; setTimeout(updateWatch, 3000) }
    })()), 500)
  }
  workspace.subscribe(updateWatch); updateWatch()
  if (test) Object.assign(window, { editorWorkspace: workspace })
}
function showError(error: unknown): void {
  if (workspace) workspace.notify(error instanceof Error ? error.message : String(error))
  else root.render(<div className="welcome"><h1>{t('title')}</h1><p role="alert">{String(error)}</p><button onClick={() => { void start().catch(showError) }}>{t('retry')}</button></div>)
}
void start().catch(showError)
