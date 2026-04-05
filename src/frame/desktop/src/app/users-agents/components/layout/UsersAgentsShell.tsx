/* ── Users & Agents – main shell layout ── */

import { useCallback, useState } from 'react'
import { useMediaQuery } from '@mui/material'
import { Sidebar } from './Sidebar'
import { CollectionList } from './CollectionList'
import { MobileHomeScreen } from './MobileHomeScreen'
import { EmptyPlaceholder } from '../detail/EmptyPlaceholder'
import { SelfDetailPage } from '../detail/SelfDetailPage'
import { AgentDetailPage } from '../detail/AgentDetailPage'
import { LocalUserDetailPage } from '../detail/LocalUserDetailPage'
import { ContactDetailPage } from '../detail/ContactDetailPage'
import { EntityGroupDetailPage } from '../detail/EntityGroupDetailPage'
import { NewUserWizard } from '../shared/NewUserWizard'
import { useEntity, useUsersAgentsStore } from '../../hooks/use-users-agents-store'
import { useMobileBackHandler } from '../../../../desktop/windows/MobileNavContext'
import type { SidebarSelection } from '../../mock/types'

function DetailRouter({ entityId, onRemoved }: { entityId: string; onRemoved?: () => void }) {
  const entity = useEntity(entityId)
  if (!entity) return <EmptyPlaceholder />

  switch (entity.kind) {
    case 'self':
      return <SelfDetailPage />
    case 'agent':
      return <AgentDetailPage />
    case 'local-user':
      return <LocalUserDetailPage user={entity} onRemoved={onRemoved} />
    case 'contact':
      return <ContactDetailPage contact={entity} onRemoved={onRemoved} />
    case 'entity-group':
      return <EntityGroupDetailPage group={entity} />
    default:
      return <EmptyPlaceholder />
  }
}

export function UsersAgentsShell() {
  const [selection, setSelection] = useState<SidebarSelection | null>(null)
  const [collectionElementId, setCollectionElementId] = useState<string | null>(null)
  const [showNewUser, setShowNewUser] = useState(false)
  const isMobile = useMediaQuery('(max-width: 767px)')
  const store = useUsersAgentsStore()

  const handleSelect = (sel: SidebarSelection) => {
    setSelection(sel)
    setCollectionElementId(null)
    setShowNewUser(false)
  }

  const handleBack = useCallback(() => {
    if (collectionElementId) {
      setCollectionElementId(null)
    } else {
      setSelection(null)
      setShowNewUser(false)
    }
  }, [collectionElementId])

  const handleRenameCollection = (id: string, currentName: string) => {
    const newName = window.prompt('Rename collection:', currentName)
    if (newName && newName.trim() && newName !== currentName) {
      store.renameCollection(id, newName.trim())
    }
  }

  const handleDeleteCollection = (id: string) => {
    if (window.confirm('Delete this collection?')) {
      store.removeCollection(id)
      if (selection?.kind === 'collection' && selection.collectionId === id) {
        setSelection(null)
        setCollectionElementId(null)
      }
    }
  }

  const handleUserCreated = (userId: string) => {
    setShowNewUser(false)
    setSelection({ kind: 'entity', entityId: userId })
  }

  const handleEntityRemoved = () => {
    setSelection(null)
    setCollectionElementId(null)
  }

  // Register back handler: any mobile sub-page that isn't the root sidebar
  const isOnSubPage = isMobile && (selection !== null || showNewUser)
  useMobileBackHandler(isOnSubPage ? handleBack : null)

  // ── Mobile layout ──
  if (isMobile) {
    // level 0: full-width mobile home with badge & server cards
    if (!selection && !showNewUser) {
      return (
        <MobileHomeScreen
          selection={selection}
          onSelect={handleSelect}
          onAddUser={() => setShowNewUser(true)}
          onRenameCollection={handleRenameCollection}
          onDeleteCollection={handleDeleteCollection}
        />
      )
    }

    // new user wizard (mobile)
    if (showNewUser) {
      return (
        <div className="flex flex-col h-full w-full" style={{ background: 'var(--cp-bg)' }}>
          <div className="flex-1 overflow-y-auto desktop-scrollbar px-4 py-4">
            <NewUserWizard onClose={() => setShowNewUser(false)} onCreated={handleUserCreated} />
          </div>
        </div>
      )
    }

    // level 1 (collection): show list
    if (selection?.kind === 'collection' && !collectionElementId) {
      return (
        <div className="flex flex-col h-full w-full" style={{ background: 'var(--cp-bg)' }}>
          <div className="flex-1 overflow-hidden">
            <CollectionList
              collectionId={selection.collectionId}
              selectedElementId={null}
              onSelectElement={setCollectionElementId}
            />
          </div>
        </div>
      )
    }

    // level 2: detail
    const detailEntityId =
      selection?.kind === 'entity'
        ? selection.entityId
        : collectionElementId

    return (
      <div className="flex flex-col h-full w-full" style={{ background: 'var(--cp-bg)' }}>
        <main className="flex-1 overflow-y-auto desktop-scrollbar">
          <div className="px-4 pb-5 pt-3">
            {detailEntityId ? <DetailRouter entityId={detailEntityId} onRemoved={handleEntityRemoved} /> : <EmptyPlaceholder />}
          </div>
        </main>
      </div>
    )
  }

  // ── Desktop layout ──
  const isCollectionMode = selection?.kind === 'collection'

  return (
    <div className="flex h-full w-full" style={{ background: 'var(--cp-bg)' }}>
      <Sidebar
        selection={selection}
        onSelect={handleSelect}
        onAddUser={() => setShowNewUser(true)}
        onRenameCollection={handleRenameCollection}
        onDeleteCollection={handleDeleteCollection}
      />

      {isCollectionMode && (
        <CollectionList
          collectionId={selection.collectionId}
          selectedElementId={collectionElementId}
          onSelectElement={setCollectionElementId}
        />
      )}

      <main className="flex-1 overflow-y-auto desktop-scrollbar min-w-0">
        <div className="px-6 py-5 max-w-3xl">
          {showNewUser ? (
            <NewUserWizard onClose={() => setShowNewUser(false)} onCreated={handleUserCreated} />
          ) : selection?.kind === 'entity' ? (
            <DetailRouter entityId={selection.entityId} onRemoved={handleEntityRemoved} />
          ) : collectionElementId ? (
            <DetailRouter entityId={collectionElementId} onRemoved={handleEntityRemoved} />
          ) : (
            <EmptyPlaceholder />
          )}
        </div>
      </main>
    </div>
  )
}
