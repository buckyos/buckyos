/* ── Users & Agents – left sidebar ── */

import { useMemo, useState } from 'react'
import { Bot, Search, UserPlus } from 'lucide-react'
import { Chip, IconButton, Tooltip } from '@mui/material'
import { EntityCard } from '../cards/EntityCard'
import { SearchFilterBar } from '../shared/SearchFilterBar'
import { entityFilterOptions, filterInternalEntities, getInternalEntities, type InternalEntityFilter } from '../shared/entityFilters'
import {
  useSelf,
  useAgents,
  useLocalUsers,
  useEntityGroups,
} from '../../hooks/use-users-agents-store'
import type { SidebarSelection } from '../../datamodel/types'
import { useI18n } from '../../../../i18n/provider'

interface SidebarProps {
  selection: SidebarSelection | null
  onSelect: (sel: SidebarSelection) => void
  onAddUser?: () => void
  onAddAgent?: () => void
  canCreateAgents?: boolean
}

export function Sidebar({ selection, onSelect, onAddUser, onAddAgent, canCreateAgents = true }: SidebarProps) {
  const { t } = useI18n()
  const [showSearch, setShowSearch] = useState(false)
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<InternalEntityFilter>('all')
  const self = useSelf()
  const agents = useAgents()
  const localUsers = useLocalUsers()
  const entityGroups = useEntityGroups()

  const isEntityActive = (id: string) =>
    selection?.kind === 'entity' ? selection.entityId === id : selection?.kind === 'self' && id === self.id

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
            {t('usersAgents.internalEntities')}
          </span>
          <div className="flex items-center gap-1">
            <Tooltip title={t('usersAgents.searchOrFilter')}>
              <IconButton size="small" onClick={() => setShowSearch((value) => !value)} aria-label={t('usersAgents.searchOrFilter')}>
                <Search size={14} />
              </IconButton>
            </Tooltip>
            {onAddAgent && (
              <Tooltip title={canCreateAgents ? t('usersAgents.addAgent') : t('usersAgents.addAgent.limited')}>
                <IconButton
                  size="small"
                  onClick={onAddAgent}
                  aria-label={t('usersAgents.addAgent')}
                  aria-disabled={!canCreateAgents || undefined}
                  data-testid="users-agents-add-agent"
                  style={canCreateAgents ? undefined : { opacity: 0.45 }}
                >
                  <Bot size={14} />
                </IconButton>
              </Tooltip>
            )}
            {onAddUser && (
              <Tooltip title={t('usersAgents.addUser')}>
                <IconButton size="small" onClick={onAddUser} aria-label={t('usersAgents.addUser')}>
                  <UserPlus size={14} />
                </IconButton>
              </Tooltip>
            )}
          </div>
        </div>

        {(showSearch || query || filter !== 'all') && (
          <>
            <SearchFilterBar
              query={query}
              onQueryChange={setQuery}
              placeholder={t('usersAgents.searchPlaceholder')}
            />
            <div className="mx-2 mb-2 mt-1 flex flex-wrap gap-1">
              {entityFilterOptions.map((item) => (
                <Chip
                  key={item.value}
                  label={t(item.labelKey)}
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
              {t('usersAgents.noMatch')}
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
