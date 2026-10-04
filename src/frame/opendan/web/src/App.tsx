import { useSyncExternalStore } from 'react'
import clsx from 'clsx'
import { RefreshCw } from 'lucide-react'
import { useSWRConfig } from 'swr'
import { REFRESH_MS, useQuery } from './lib'
import { dataModel } from './model'
import { Mono } from './ui'
import AgentStatePage from './pages/AgentState'
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

export default function App() {
  const hash = useSyncExternalStore(subscribeHash, readHash)
  const [page = 'sessions', arg] = hash.replace(/^#\/?/, '').split('/')
  const tab = page === 'session' ? 'sessions' : page
  const { mutate } = useSWRConfig()
  const status = useQuery(['loader.status'], () => dataModel.loaderStatus())

  return (
    <div className="mx-auto max-w-[1400px] px-3 pb-8">
      <header className="mb-3 flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-line py-2">
        <h1 className="text-sm font-semibold">OpenDAN Agent Loader</h1>
        <nav className="flex gap-1">
          {TABS.map((t) => (
            <a
              key={t.id}
              href={`#/${t.id}`}
              className={clsx('rounded px-2 py-1 text-xs', tab === t.id ? 'bg-accent text-panel' : 'text-mute hover:text-fg')}
            >
              {t.label}
            </a>
          ))}
        </nav>
        <div className="ml-auto flex items-center gap-3 text-xs text-mute">
          {status.data && <Mono>{status.data.agent_did}</Mono>}
          <span data-testid="data-source">data: {dataModel.source}</span>
          <span>auto {REFRESH_MS / 1000}s</span>
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
