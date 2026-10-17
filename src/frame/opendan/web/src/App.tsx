import { useSyncExternalStore } from 'react'
import clsx from 'clsx'
import { ChevronLeft, RefreshCw } from 'lucide-react'
import { useSWRConfig } from 'swr'
import { t } from './i18n'
import { REFRESH_MS, useQuery } from './lib'
import { dataModel } from './model'
import { Mono } from './ui'
import AgentStatePage from './pages/AgentState'
import HomePage from './pages/Home'
import LoaderPage from './pages/Loader'
import SessionDetailPage from './pages/SessionDetail'
import SessionsPage from './pages/Sessions'

const subscribeHash = (notify: () => void) => {
  window.addEventListener('hashchange', notify)
  return () => window.removeEventListener('hashchange', notify)
}
const readHash = () => window.location.hash

const TABS = [
  { id: 'sessions', label: 'Sessions' },
  { id: 'agent', label: 'Agent State' },
  { id: 'loader', label: 'Loader' },
]

// Reloading is a navigation: the zone gateway sends it to the zone's login
// page, which comes back here once signed in.
function LoginLost() {
  return (
    <div className="mx-auto max-w-md px-4 py-16 text-center" data-testid="login-lost" role="alert">
      <h1 className="mb-2 text-base font-semibold">{t('loginLost')}</h1>
      <p className="mb-4 text-sm text-mute">{t('loginLostHint')}</p>
      <button className="btn mx-auto" onClick={() => window.location.reload()}>
        {t('signInAgain')}
      </button>
    </div>
  )
}

export default function App() {
  const hash = useSyncExternalStore(subscribeHash, readHash)
  const loginLost = useSyncExternalStore(dataModel.login.subscribe, dataModel.login.lost)
  const [page = '', arg] = hash.replace(/^#\/?/, '').split('/')
  const tab = page === 'session' ? 'sessions' : page
  const home = !TABS.some((x) => x.id === tab)
  const { mutate } = useSWRConfig()
  const status = useQuery(home || loginLost ? null : ['loader.status'], () => dataModel.loaderStatus())
  if (loginLost) return <LoginLost />
  const meta = (
    <>
      <span data-testid="data-source">data: {dataModel.source}</span>
      <span>auto {REFRESH_MS / 1000}s</span>
    </>
  )

  if (home) {
    return (
      <div className="mx-auto max-w-5xl px-3 pb-6 pt-3 sm:px-4">
        <HomePage />
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
    <div className="mx-auto max-w-[1400px] px-3 pb-8">
      <header className="mb-3 flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-line py-2">
        <a href="#/" className="flex items-center text-sm font-semibold" aria-label="Home">
          <ChevronLeft size={16} />
          OpenDAN Agent Loader
        </a>
        <nav className="flex gap-1">
          {TABS.map((x) => (
            <a
              key={x.id}
              href={`#/${x.id}`}
              className={clsx('rounded px-2 py-1 text-xs', tab === x.id ? 'bg-accent text-panel' : 'text-mute hover:text-fg')}
            >
              {x.label}
            </a>
          ))}
        </nav>
        <div className="ml-auto flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-mute">
          {status.data && <Mono className="hidden sm:inline">{status.data.agent_did}</Mono>}
          {meta}
          <button className="btn" onClick={() => mutate(() => true)} aria-label="Refresh">
            <RefreshCw size={12} />
            Refresh
          </button>
        </div>
      </header>
      {page === 'session' && arg ? (
        <SessionDetailPage key={arg} sid={decodeURIComponent(arg)} />
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
