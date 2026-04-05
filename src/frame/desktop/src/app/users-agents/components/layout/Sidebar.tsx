/* ── Users & Agents – left sidebar ── */

import { Plus, UserPlus } from 'lucide-react'
import { IconButton } from '@mui/material'
import { EntityCard } from '../cards/EntityCard'
import { CollectionCard } from '../cards/CollectionCard'
import {
  useSelf,
  useAgent,
  useLocalUsers,
  useEntityGroups,
  useCollections,
  useUsersAgentsStore,
} from '../../hooks/use-users-agents-store'
import type { SidebarSelection } from '../../mock/types'

interface SidebarProps {
  selection: SidebarSelection | null
  onSelect: (sel: SidebarSelection) => void
  onAddUser?: () => void
  onRenameCollection?: (id: string, currentName: string) => void
  onDeleteCollection?: (id: string) => void
}

export function Sidebar({ selection, onSelect, onAddUser, onRenameCollection, onDeleteCollection }: SidebarProps) {
  const self = useSelf()
  const agent = useAgent()
  const localUsers = useLocalUsers()
  const entityGroups = useEntityGroups()
  const collections = useCollections()
  const store = useUsersAgentsStore()

  const isEntityActive = (id: string) =>
    selection?.kind === 'entity' && selection.entityId === id

  const isCollectionActive = (id: string) =>
    selection?.kind === 'collection' && selection.collectionId === id

  const handleNewCollection = () => {
    const col = store.addCollection('New Collection')
    onSelect({ kind: 'collection', collectionId: col.id })
  }

  // entity groups hosted by self shown in entity area
  const hostedGroups = entityGroups.filter((g) => g.isHostedBySelf)

  return (
    <div
      className="flex flex-col h-full w-60 shrink-0 overflow-y-auto desktop-scrollbar"
      style={{
        borderRight: '1px solid color-mix(in srgb, var(--cp-border) 60%, transparent)',
      }}
    >
      {/* ── Entity cards area ── */}
      <div className="px-2 pt-3 pb-1">
        <div className="flex items-center justify-between px-2 pb-2">
          <span
            className="text-[11px] font-semibold uppercase tracking-[0.18em]"
            style={{ color: 'var(--cp-muted)' }}
          >
            Entities
          </span>
          {onAddUser && (
            <IconButton size="small" onClick={onAddUser} aria-label="Add user">
              <UserPlus size={14} />
            </IconButton>
          )}
        </div>

        <div className="space-y-0.5">
          <EntityCard
            entity={self}
            isActive={isEntityActive(self.id)}
            onClick={() => onSelect({ kind: 'entity', entityId: self.id })}
          />
          <EntityCard
            entity={agent}
            isActive={isEntityActive(agent.id)}
            onClick={() => onSelect({ kind: 'entity', entityId: agent.id })}
          />

          {localUsers.map((u) => (
            <EntityCard
              key={u.id}
              entity={u}
              isActive={isEntityActive(u.id)}
              onClick={() => onSelect({ kind: 'entity', entityId: u.id })}
            />
          ))}

          {hostedGroups.map((g) => (
            <EntityCard
              key={g.id}
              entity={g}
              isActive={isEntityActive(g.id)}
              onClick={() => onSelect({ kind: 'entity', entityId: g.id })}
            />
          ))}
        </div>
      </div>

      {/* divider */}
      <div
        className="mx-4 my-1"
        style={{
          height: 1,
          background: 'color-mix(in srgb, var(--cp-border) 50%, transparent)',
        }}
      />

      {/* ── Collection cards area ── */}
      <div className="px-2 pt-1 pb-3 flex-1">
        <div className="flex items-center justify-between px-2 pb-2">
          <span
            className="text-[11px] font-semibold uppercase tracking-[0.18em]"
            style={{ color: 'var(--cp-muted)' }}
          >
            Collections
          </span>
          <IconButton size="small" onClick={handleNewCollection} aria-label="New collection">
            <Plus size={14} />
          </IconButton>
        </div>

        <div className="space-y-0.5">
          {collections.map((col) => (
            <CollectionCard
              key={col.id}
              collection={col}
              isActive={isCollectionActive(col.id)}
              onClick={() => onSelect({ kind: 'collection', collectionId: col.id })}
              onRename={!col.isBuiltIn && onRenameCollection ? () => onRenameCollection(col.id, col.name) : undefined}
              onDelete={!col.isBuiltIn && onDeleteCollection ? () => onDeleteCollection(col.id) : undefined}
            />
          ))}
        </div>
      </div>
    </div>
  )
}
