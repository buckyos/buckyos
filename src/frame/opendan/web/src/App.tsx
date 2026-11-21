import { useSyncExternalStore } from 'react'
import clsx from 'clsx'
import { ChevronLeft, RefreshCw } from 'lucide-react'
import { useSWRConfig } from 'swr'
import { currentLanguage, subscribeLanguage, t } from './i18n'
import { REFRESH_MS, useQuery } from './lib'
import { dataModel } from './model'
import { Dot, Mono } from './ui'
import AgentStatePage from './pages/AgentState'
import GoalDetailPage from './pages/GoalDetail'
import HomePage from './pages/Home'
import LoaderPage from './pages/Loader'
import MemoryPage from './pages/Memory'
import RuntimeDetailPage from './pages/Runtimes'
import SessionDetailPage from './pages/SessionDetail'
import SessionsPage from './pages/Sessions'
import SkillsPage from './pages/Skills'
import WorkspacesPage from './pages/Workspaces'

const subscribeHash = (notify: () => void) => {
  window.addEventListener('hashchange', notify)
  return () => window.removeEventListener('hashchange', notify)
}
const readHash = () => window.location.hash

const TABS = [
  { id: 'sessions', label: 'sessions' },
  { id: 'skills', label: 'skills' },
  { id: 'agent', label: 'observeAgentState' },
  { id: 'loader', label: 'observeLoader' },
] as const

// Reloading is a navigation: the zone gateway sends it to the zone's login
// page, which comes back here once signed in.
function LoginLost() {
  return (
    <div className="mx-auto max-w-md px-4 py-16 text-center" data-testid="login-lost" role="alert">
      <h1 className="mb-2 text-base font-medium">{t('loginLost')}</h1>
      <p className="mb-4 text-sm text-mute">{t('loginLostHint')}</p>
      <button className="btn btn-primary mx-auto" onClick={() => window.location.reload()}>
        {t('signInAgain')}
      </button>
    </div>
  )
}

export default function App() {
  useSyncExternalStore(subscribeLanguage, currentLanguage)
  const hash = useSyncExternalStore(subscribeHash, readHash)
  const loginLost = useSyncExternalStore(dataModel.login.subscribe, dataModel.login.lost)
  const [page = '', arg] = hash.replace(/^#\/?/, '').split('/')
  // Detail pages (`#/session/<sid>`, `#/goal/<goal_id>`) share the tabbed
  // layout; a goal has no tab of its own and its back link goes home.
  const detail = (page === 'session' || page === 'goal') && Boolean(arg)
  const tab = page === 'session' ? 'sessions' : page
  // Detail pages in the home page's style (`#/memory`, `#/workspaces`, `#/runtime/<id>`).
  const shell =
    page === 'memory' ? <MemoryPage /> : page === 'workspaces' ? <WorkspacesPage /> : page === 'runtime' && arg ? <RuntimeDetailPage key={arg} runtimeId={decodeURIComponent(arg)} /> : null
  const home = !detail && !TABS.some((x) => x.id === tab)
  const { mutate } = useSWRConfig()
  const status = useQuery(home || loginLost ? null : ['loader.status'], () => dataModel.loaderStatus())
  if (loginLost) return <LoginLost />
  const meta = (
    <>
      <span data-testid="data-source">{t('observeMetaSource', { source: dataModel.source })}</span>
      <span aria-hidden>·</span>
      <span>{t('observeAutoRefresh', { seconds: REFRESH_MS / 1000 })}</span>
    </>
  )

  if (home) {
    return (
      <div className="mx-auto max-w-5xl px-3 pb-6 pt-3 sm:px-4">
        {shell ?? <HomePage />}
        <footer className="mt-4 flex flex-wrap items-center justify-center gap-x-4 gap-y-1 text-xs text-mute">
          <button className="tap px-2 text-xs font-normal" onClick={() => mutate(() => true)}>
            <RefreshCw size={12} />
            {t('refresh')}
          </button>
          <a className="tap px-2 text-xs font-normal" href="#/sessions">
            {t('advanced')}
          </a>
          {meta}
        </footer>
      </div>
    )
  }

  return (
    <div className="mx-auto max-w-[1440px] px-4 pb-12 sm:px-6">
      <header className="mb-6 flex min-h-14 flex-wrap items-center gap-x-6 gap-y-2 border-b border-line py-2">
        <a href="#/" className="-ml-1 flex h-8 items-center gap-1 rounded-lg pl-0.5 pr-1.5 text-sm font-medium transition-colors hover:text-accent" aria-label={t('backHome')}>
          <ChevronLeft size={16} className="text-mute" />
          {t('observeOpendanAgentLoader')}
        </a>
        <nav className="flex gap-1">
          {TABS.map((x) => (
            <a
              key={x.id}
              href={`#/${x.id}`}
              aria-current={tab === x.id ? 'page' : undefined}
              className={clsx(
                'flex h-7 items-center rounded-lg px-2.5 text-xs transition-colors',
                tab === x.id ? 'bg-panel text-fg shadow-[inset_0_0_0_1px_var(--line)]' : 'text-mute hover:bg-hover hover:text-fg',
              )}
            >
              {t(x.label)}
            </a>
          ))}
        </nav>
        <div className="ml-auto flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-mute">
          {status.data && <Mono className="hidden lg:inline">{status.data.agent_did}</Mono>}
          <span className="flex items-center gap-2 font-mono" title={status.error ? status.error.message : undefined}>
            <Dot tone={status.error ? 'bad' : status.data ? 'ok' : 'mute'} />
            {meta}
          </span>
          <button className="btn" onClick={() => mutate(() => true)} aria-label={t('refresh')}>
            <RefreshCw size={12} />
            {t('refresh')}
          </button>
        </div>
      </header>
      {page === 'session' && arg ? (
        <SessionDetailPage key={arg} sid={decodeURIComponent(arg)} />
      ) : page === 'goal' && arg ? (
        <GoalDetailPage key={arg} goalId={decodeURIComponent(arg)} />
      ) : tab === 'skills' ? (
        <SkillsPage />
      ) : tab === 'agent' ? (
        <AgentStatePage />
      ) : tab === 'loader' ? (
        <LoaderPage />
      ) : (
        <SessionsPage />
      )}
    </div>
  )
}
