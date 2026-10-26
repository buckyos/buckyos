/* ── Users & Agents – main shell layout ── */

import { useCallback, useEffect, useRef, useState } from 'react'
import { Alert, Snackbar, useMediaQuery } from '@mui/material'
import { Sidebar } from './Sidebar'
import { MobileHomeScreen } from './MobileHomeScreen'
import { EmptyPlaceholder } from '../detail/EmptyPlaceholder'
import { SelfDetailPage } from '../detail/SelfDetailPage'
import { AgentDetailPage } from '../detail/AgentDetailPage'
import { LocalUserDetailPage } from '../detail/LocalUserDetailPage'
import { DeletedUsersPage } from '../detail/DeletedUsersPage'
import { EntityGroupDetailPage } from '../detail/EntityGroupDetailPage'
import { NewUserWizard } from '../shared/NewUserWizard'
import { UserGroupsPage } from '../groups/UserGroupsPage'
import { ExclusiveInvitesPage } from '../invites/ExclusiveInvitesPage'
import { useCanCreateAgents, useEntity, useUsersAgentsStore } from '../../hooks/use-users-agents-store'
import { useMobileBackHandler } from '../../../../desktop/windows/MobileNavContext'
import { useI18n } from '../../../../i18n/provider'
import { openAgentSetup } from '../../../agent-setup/launch'
import type { SidebarSelection } from '../../datamodel/types'
import type { UsersAgentsLaunch } from '../../launch'
import type { UserCreateResponse } from '../../../../api/user_mgr'

function DetailRouter({ entityId, onRemoved, onNotice }: { entityId: string; onRemoved?: () => void; onNotice: (message: string) => void }) {
  const entity = useEntity(entityId)
  const store = useUsersAgentsStore()
  const reloaded = useRef<string | null>(null)

  // A launch can point at an entity created after this window loaded its list.
  useEffect(() => {
    if (entity || reloaded.current === entityId) return
    reloaded.current = entityId
    void store.reload().catch(() => undefined)
  }, [entity, entityId, store])

  if (!entity) return <EmptyPlaceholder />

  switch (entity.kind) {
    case 'self':
      return <SelfDetailPage />
    case 'agent':
      return <AgentDetailPage agent={entity} onRemoved={onRemoved} />
    case 'local-user':
      return <LocalUserDetailPage key={entity.id} user={entity} onRemoved={onRemoved} onNotice={onNotice} />
    case 'entity-group':
      return <EntityGroupDetailPage group={entity} />
    default:
      return <EmptyPlaceholder />
  }
}

function SelectionRouter({ selection, onRemoved, onNotice }: { selection: SidebarSelection | null; onRemoved: () => void; onNotice: (message: string) => void }) {
  if (!selection) return <EmptyPlaceholder />
  if (selection.kind === 'self') return <SelfDetailPage />
  if (selection.kind === 'user-groups') return <UserGroupsPage />
  if (selection.kind === 'exclusive-invites') return <ExclusiveInvitesPage />
  if (selection.kind === 'deleted-users') return <DeletedUsersPage onNotice={onNotice} />
  return <DetailRouter entityId={selection.entityId} onRemoved={onRemoved} onNotice={onNotice} />
}

function initialSelection(launch?: UsersAgentsLaunch): SidebarSelection | null {
  if (!launch) return null
  if ('view' in launch) return { kind: 'self' }
  return { kind: 'entity', entityId: launch.entityId }
}

export function UsersAgentsShell({ launch }: { launch?: UsersAgentsLaunch }) {
  const { t } = useI18n()
  const [selection, setSelection] = useState<SidebarSelection | null>(() => initialSelection(launch))
  const [showNewUser, setShowNewUser] = useState(false)
  const [agentNoticeOpen, setAgentNoticeOpen] = useState(false)
  const [userNotice, setUserNotice] = useState<string | null>(null)
  const [lifecycleNotice, setLifecycleNotice] = useState<string | null>(null)
  const canCreateAgents = useCanCreateAgents()
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
    if (!canCreateAgents) {
      setAgentNoticeOpen(true)
      return
    }
    openAgentSetup({ source: 'users_agents' })
  }

  const handleUserCreated = (userId: string, result: UserCreateResponse) => {
    setShowNewUser(false)
    setSelection({ kind: 'entity', entityId: userId })
    if (!result.rbac_refreshed) {
      setUserNotice(result.warning ?? t('usersAgents.newUser.rbacPending'))
    }
  }

  const handleEntityRemoved = () => {
    setSelection(null)
  }

  const lifecycleAlert = lifecycleNotice ? (
    <Alert severity="warning" className="mb-4" onClose={() => setLifecycleNotice(null)} data-testid="user-lifecycle-notice">{lifecycleNotice}</Alert>
  ) : null

  const notices = (
    <>
      <Snackbar
        open={agentNoticeOpen}
        autoHideDuration={4000}
        onClose={() => setAgentNoticeOpen(false)}
        anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
      >
        <Alert severity="info" variant="filled" onClose={() => setAgentNoticeOpen(false)}>
          {t('usersAgents.addAgent.limited')}
        </Alert>
      </Snackbar>
      <Snackbar
        open={Boolean(userNotice)}
        autoHideDuration={6000}
        onClose={() => setUserNotice(null)}
        anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
      >
        <Alert severity="warning" variant="filled" onClose={() => setUserNotice(null)}>
          {userNotice}
        </Alert>
      </Snackbar>
    </>
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
          {lifecycleAlert}
          <MobileHomeScreen
            selection={selection}
            onSelect={handleSelect}
            onAddUser={() => setShowNewUser(true)}
            onAddAgent={handleAddAgent}
            canCreateAgents={canCreateAgents}
          />
          {notices}
        </>
      )
    }

    // new user wizard (mobile)
    if (showNewUser) {
      return (
        <div className="flex flex-col h-full w-full" style={{ background: 'var(--cp-bg)' }}>
          <div className="flex-1 overflow-y-auto desktop-scrollbar px-4 py-4">
            {lifecycleAlert}
            <NewUserWizard onClose={() => setShowNewUser(false)} onCreated={handleUserCreated} onOpenDeletedUsers={() => handleSelect({ kind: 'deleted-users' })} />
          </div>
          {notices}
        </div>
      )
    }

    return (
      <div className="flex flex-col h-full w-full" style={{ background: 'var(--cp-bg)' }}>
        <main className="flex-1 overflow-y-auto desktop-scrollbar">
          <div className="px-4 pb-5 pt-3">
            {lifecycleAlert}
            <SelectionRouter selection={selection} onRemoved={handleEntityRemoved} onNotice={setLifecycleNotice} />
          </div>
        </main>
        {notices}
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
        canCreateAgents={canCreateAgents}
      />

      <main className="flex-1 overflow-y-auto desktop-scrollbar min-w-0">
        <div className="px-6 py-5 max-w-3xl">
          {lifecycleAlert}
          {showNewUser ? (
            <NewUserWizard onClose={() => setShowNewUser(false)} onCreated={handleUserCreated} onOpenDeletedUsers={() => handleSelect({ kind: 'deleted-users' })} />
          ) : (
            <SelectionRouter selection={selection} onRemoved={handleEntityRemoved} onNotice={setLifecycleNotice} />
          )}
        </div>
      </main>
      {notices}
    </div>
  )
}
