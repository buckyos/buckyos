/* ── Users & Agents – app panel entry point ── */

import { useEffect, useState } from 'react'
import type { AppContentLoaderProps } from '../types'
import { onAgentsChanged } from '../agent-setup/events'
import { UsersAgentsStoreContext } from './hooks/use-users-agents-store'
import { UsersAgentsStore } from './datamodel/store'
import { UsersAgentsShell } from './components/layout/UsersAgentsShell'
import { usersAgentsLaunchSchema } from './launch'

export function UsersAgentsAppPanel({ launch }: AppContentLoaderProps) {
  const [store] = useState(() => new UsersAgentsStore())
  const parsed = usersAgentsLaunchSchema.safeParse(launch?.payload)

  useEffect(() => onAgentsChanged(() => {
    void store.reload().catch(() => undefined)
  }), [store])

  return (
    <UsersAgentsStoreContext.Provider value={store}>
      <UsersAgentsShell key={launch?.requestId ?? 'default'} launch={parsed.success ? parsed.data : undefined} />
    </UsersAgentsStoreContext.Provider>
  )
}
