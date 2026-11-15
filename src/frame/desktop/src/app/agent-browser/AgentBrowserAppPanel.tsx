/* ── Agent Browser – app panel entry point ── */

import { useEffect, useState } from 'react'
import type { AppContentLoaderProps } from '../types'
import { agentBrowserLaunchSchema, type AgentBrowserLaunch } from './launch'
import { AgentBrowserShell } from './shell/AgentBrowserShell'
import { AgentBrowserStoreContext } from './store/context'
import { AgentBrowserStore } from './store/store'
import './agent-browser.css'

function AgentBrowserApp({ launch }: { launch?: AgentBrowserLaunch }) {
  const [store] = useState(() => new AgentBrowserStore())
  const [initial] = useState(launch)
  useEffect(() => {
    void store.start(initial)
    return () => store.dispose()
  }, [store, initial])
  return (
    <AgentBrowserStoreContext.Provider value={store}>
      <AgentBrowserShell />
    </AgentBrowserStoreContext.Provider>
  )
}

export function AgentBrowserAppPanel({ launch }: AppContentLoaderProps) {
  const parsed = agentBrowserLaunchSchema.safeParse(launch?.payload)
  return <AgentBrowserApp key={launch?.requestId ?? 'default'} launch={parsed.success ? parsed.data : undefined} />
}
