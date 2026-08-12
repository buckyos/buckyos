/* ── Users & Agents – app panel entry point ── */

import { useState } from 'react'
import { UsersAgentsStoreContext } from './hooks/use-users-agents-store'
import { UsersAgentsStore } from './datamodel/store'
import { UsersAgentsShell } from './components/layout/UsersAgentsShell'

export function UsersAgentsAppPanel() {
  const [store] = useState(() => new UsersAgentsStore())

  return (
    <UsersAgentsStoreContext.Provider value={store}>
      <UsersAgentsShell />
    </UsersAgentsStoreContext.Provider>
  )
}
