/* ── Users & Agents – main shell layout ── */

import { useCallback, useState } from 'react'
import { Alert, Snackbar, useMediaQuery } from '@mui/material'
import { Sidebar } from './Sidebar'
import { MobileHomeScreen } from './MobileHomeScreen'
import { EmptyPlaceholder } from '../detail/EmptyPlaceholder'
import { SelfDetailPage } from '../detail/SelfDetailPage'
import { AgentDetailPage } from '../detail/AgentDetailPage'
import { LocalUserDetailPage } from '../detail/LocalUserDetailPage'
import { EntityGroupDetailPage } from '../detail/EntityGroupDetailPage'
import { NewUserWizard } from '../shared/NewUserWizard'
import { useEntity } from '../../hooks/use-users-agents-store'
import { useMobileBackHandler } from '../../../../desktop/windows/MobileNavContext'
import type { SidebarSelection } from '../../datamodel/types'

function DetailRouter({ entityId, onRemoved }: { entityId: string; onRemoved?: () => void }) {
  const entity = useEntity(entityId)
  if (!entity) return <EmptyPlaceholder />

  switch (entity.kind) {
    case 'self':
      return <SelfDetailPage />
    case 'agent':
      return <AgentDetailPage agent={entity} />
    case 'local-user':
      return <LocalUserDetailPage user={entity} onRemoved={onRemoved} />
    case 'entity-group':
      return <EntityGroupDetailPage group={entity} />
    default:
      return <EmptyPlaceholder />
  }
}

export function UsersAgentsShell() {
  const [selection, setSelection] = useState<SidebarSelection | null>(null)
  const [showNewUser, setShowNewUser] = useState(false)
  const [agentNoticeOpen, setAgentNoticeOpen] = useState(false)
  const isMobile = useMediaQuery('(max-width: 767px)')

  const handleSelect = (sel: SidebarSelection) => {
    setSelection(sel)
    setShowNewUser(false)
  }

  const handleBack = useCallback(() => {
    setSelection(null)
    setShowNewUser(false)
  }, [])

  const handleAddAgent = () => {
    setAgentNoticeOpen(true)
  }

  const handleUserCreated = (userId: string) => {
    setShowNewUser(false)
    setSelection({ kind: 'entity', entityId: userId })
  }

  const handleEntityRemoved = () => {
    setSelection(null)
  }

  const agentNotice = (
    <Snackbar
      open={agentNoticeOpen}
      autoHideDuration={2600}
      onClose={() => setAgentNoticeOpen(false)}
      anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
    >
      <Alert severity="info" variant="filled" onClose={() => setAgentNoticeOpen(false)}>
        Agent creation is coming soon.
      </Alert>
    </Snackbar>
  )

  // Register back handler: any mobile sub-page that isn't the root sidebar
  const isOnSubPage = isMobile && (selection !== null || showNewUser)
  useMobileBackHandler(isOnSubPage ? handleBack : null)

  // ── Mobile layout ──
  if (isMobile) {
    // level 0: full-width mobile home with badge & server cards
    if (!selection && !showNewUser) {
      return (
        <>
          <MobileHomeScreen
            selection={selection}
            onSelect={handleSelect}
            onAddUser={() => setShowNewUser(true)}
            onAddAgent={handleAddAgent}
          />
          {agentNotice}
        </>
      )
    }

    // new user wizard (mobile)
    if (showNewUser) {
      return (
        <div className="flex flex-col h-full w-full" style={{ background: 'var(--cp-bg)' }}>
          <div className="flex-1 overflow-y-auto desktop-scrollbar px-4 py-4">
            <NewUserWizard onClose={() => setShowNewUser(false)} onCreated={handleUserCreated} />
          </div>
          {agentNotice}
        </div>
      )
    }

    return (
      <div className="flex flex-col h-full w-full" style={{ background: 'var(--cp-bg)' }}>
        <main className="flex-1 overflow-y-auto desktop-scrollbar">
          <div className="px-4 pb-5 pt-3">
            {selection?.kind === 'entity' ? <DetailRouter entityId={selection.entityId} onRemoved={handleEntityRemoved} /> : <EmptyPlaceholder />}
          </div>
        </main>
        {agentNotice}
      </div>
    )
  }

  // ── Desktop layout ──
  return (
    <div className="flex h-full w-full" style={{ background: 'var(--cp-bg)' }}>
      <Sidebar
        selection={selection}
        onSelect={handleSelect}
        onAddUser={() => setShowNewUser(true)}
        onAddAgent={handleAddAgent}
      />

      <main className="flex-1 overflow-y-auto desktop-scrollbar min-w-0">
        <div className="px-6 py-5 max-w-3xl">
          {showNewUser ? (
            <NewUserWizard onClose={() => setShowNewUser(false)} onCreated={handleUserCreated} />
          ) : selection?.kind === 'entity' ? (
            <DetailRouter entityId={selection.entityId} onRemoved={handleEntityRemoved} />
          ) : (
            <EmptyPlaceholder />
          )}
        </div>
      </main>
      {agentNotice}
    </div>
  )
}
