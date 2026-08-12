/* ── Users & Agents – left sidebar ── */

import { useMemo, useState } from 'react'
import { Bot, Search, UserPlus } from 'lucide-react'
import { Chip, IconButton, Tooltip } from '@mui/material'
import { EntityCard } from '../cards/EntityCard'
import { SearchFilterBar } from '../shared/SearchFilterBar'
import { filterInternalEntities, getInternalEntities, type InternalEntityFilter } from '../shared/entityFilters'
import {
  useSelf,
  useAgents,
  useLocalUsers,
  useEntityGroups,
} from '../../hooks/use-users-agents-store'
import type { SidebarSelection } from '../../datamodel/types'

interface SidebarProps {
  selection: SidebarSelection | null
  onSelect: (sel: SidebarSelection) => void
  onAddUser?: () => void
  onAddAgent?: () => void
}

const filters: Array<{ value: InternalEntityFilter; label: string }> = [
  { value: 'all', label: 'All' },
  { value: 'users', label: 'User' },
  { value: 'agents', label: 'Agent' },
  { value: 'groups', label: 'Group' },
  { value: 'online', label: 'Online' },
]

export function Sidebar({ selection, onSelect, onAddUser, onAddAgent }: SidebarProps) {
  const [showSearch, setShowSearch] = useState(false)
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<InternalEntityFilter>('all')
  const self = useSelf()
  const agents = useAgents()
  const localUsers = useLocalUsers()
  const entityGroups = useEntityGroups()

  const isEntityActive = (id: string) =>
    selection?.kind === 'entity' && selection.entityId === id

  const entities = useMemo(
    () => filterInternalEntities(getInternalEntities(self, agents, localUsers, entityGroups), query, filter),
    [agents, entityGroups, filter, localUsers, query, self],
  )

  return (
    <div
      className="flex flex-col h-full w-60 shrink-0 overflow-y-auto desktop-scrollbar"
      style={{
        borderRight: '1px solid color-mix(in srgb, var(--cp-border) 60%, transparent)',
      }}
    >
      <div className="px-2 pt-3 pb-1">
        <div className="flex items-center justify-between px-2 pb-2">
          <span
            className="text-[11px] font-semibold uppercase tracking-[0.18em]"
            style={{ color: 'var(--cp-muted)' }}
          >
            Internal Entities
          </span>
          <div className="flex items-center gap-1">
            <Tooltip title="Search or filter">
              <IconButton size="small" onClick={() => setShowSearch((value) => !value)} aria-label="Search or filter">
                <Search size={14} />
              </IconButton>
            </Tooltip>
            {onAddAgent && (
              <Tooltip title="Add Agent">
                <IconButton size="small" onClick={onAddAgent} aria-label="Add Agent">
                  <Bot size={14} />
                </IconButton>
              </Tooltip>
            )}
            {onAddUser && (
              <Tooltip title="Add User">
                <IconButton size="small" onClick={onAddUser} aria-label="Add User">
                  <UserPlus size={14} />
                </IconButton>
              </Tooltip>
            )}
          </div>
        </div>

        {(showSearch || query) && (
          <>
            <SearchFilterBar
              query={query}
              onQueryChange={setQuery}
              placeholder="Search name, DID, role, tag, status..."
            />
            <div className="mx-2 mb-2 mt-1 flex flex-wrap gap-1">
              {filters.map((item) => (
                <Chip
                  key={item.value}
                  label={item.label}
                  size="small"
                  variant={filter === item.value ? 'filled' : 'outlined'}
                  onClick={() => setFilter(item.value)}
                />
              ))}
            </div>
          </>
        )}

        <div className="space-y-0.5">
          {entities.length === 0 && (
            <div className="px-3 py-8 text-center text-sm" style={{ color: 'var(--cp-muted)' }}>
              No internal entities match.
            </div>
          )}

          {entities.map((entity) => (
            <EntityCard
              key={entity.id}
              entity={entity}
              isActive={isEntityActive(entity.id)}
              onClick={() => onSelect({ kind: 'entity', entityId: entity.id })}
            />
          ))}
        </div>
      </div>
    </div>
  )
}
