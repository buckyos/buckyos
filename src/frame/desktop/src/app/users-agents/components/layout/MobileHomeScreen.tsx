/* ── Users & Agents – mobile first screen (full-width card layout) ── */

import { Plus, UserPlus, Server, MessageSquare, Crown, User } from 'lucide-react'
import { IconButton } from '@mui/material'
import { EntityAvatar } from '../shared/EntityAvatar'
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
import type { AnyEntity, LocalUserEntity, AgentEntity, SelfEntity, EntityGroupEntity } from '../../mock/types'

/* ── Role icons & colors ── */

const roleConfig: Record<string, { icon: typeof Crown; label: string; color: string }> = {
  admin: { icon: Crown, label: 'Admin', color: 'var(--cp-warning)' },
  member: { icon: User, label: 'Member', color: 'var(--cp-accent)' },
  guest: { icon: User, label: 'Guest', color: 'var(--cp-muted)' },
}

/* ── ID Badge Card (for self / local-user / agent) ── */

function BadgeCard({
  entity,
  isActive,
  onClick,
}: {
  entity: AnyEntity
  isActive: boolean
  onClick: () => void
}) {
  const isSelf = entity.kind === 'self'
  const isAgent = entity.kind === 'agent'
  const isLocalUser = entity.kind === 'local-user'

  const isOnline =
    isLocalUser ? (entity as LocalUserEntity).isOnline :
    isAgent ? (entity as AgentEntity).status === 'running' :
    isSelf ? true :
    undefined

  // Top stripe color
  const stripeColor =
    isSelf ? 'var(--cp-accent)' :
    isAgent ? 'var(--cp-success)' :
    'var(--cp-accent)'

  // Badge label
  const badgeLabel =
    isSelf ? 'Owner' :
    isAgent ? (entity as AgentEntity).agentType :
    roleConfig[(entity as LocalUserEntity).role]?.label ?? 'User'

  const badgeColor =
    isSelf ? 'var(--cp-accent)' :
    isAgent ? 'var(--cp-success)' :
    roleConfig[(entity as LocalUserEntity).role]?.color ?? 'var(--cp-accent)'

  // Subtitle
  const subtitle =
    isSelf ? (entity as SelfEntity).bio ?? '' :
    isAgent ? `v${(entity as AgentEntity).version} · ${(entity as AgentEntity).status}` :
    `${(entity as LocalUserEntity).storageUsed} / ${(entity as LocalUserEntity).storageQuota}`

  return (
    <button
      type="button"
      onClick={onClick}
      className="flex flex-col items-center text-center rounded-[16px] overflow-hidden transition-all duration-150"
      style={{
        background: isActive
          ? 'color-mix(in srgb, var(--cp-accent) 10%, var(--cp-surface))'
          : 'var(--cp-surface)',
        border: isActive
          ? '1px solid color-mix(in srgb, var(--cp-accent) 30%, transparent)'
          : '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
        minWidth: 0,
      }}
    >
      {/* colored top stripe (like an ID card header) */}
      <div
        className="w-full relative"
        style={{
          height: 6,
          background: `linear-gradient(90deg, ${stripeColor}, color-mix(in srgb, ${stripeColor} 60%, transparent))`,
        }}
      />

      {/* badge body */}
      <div className="flex flex-col items-center w-full px-3 pt-3 pb-3 gap-1.5">
        {/* avatar */}
        <EntityAvatar
          name={entity.displayName}
          kind={entity.kind}
          avatarUrl={entity.avatarUrl}
          size="lg"
          isOnline={isOnline}
        />

        {/* name */}
        <div
          className="truncate text-sm font-semibold w-full"
          style={{ color: 'var(--cp-text)' }}
        >
          {entity.displayName}
        </div>

        {/* role badge */}
        <span
          className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-semibold"
          style={{
            background: `color-mix(in srgb, ${badgeColor} 14%, transparent)`,
            color: badgeColor,
          }}
        >
          {badgeLabel}
        </span>

        {/* DID (like employee ID) */}
        {entity.did && (
          <div
            className="truncate text-[10px] w-full font-mono"
            style={{ color: 'var(--cp-muted)' }}
          >
            {entity.did}
          </div>
        )}

        {/* subtitle info */}
        {subtitle && (
          <div
            className="truncate text-[11px] w-full"
            style={{ color: 'var(--cp-muted)' }}
          >
            {subtitle}
          </div>
        )}
      </div>
    </button>
  )
}

/* ── Server Card (for hosted entity groups) ── */

function ServerCard({
  group,
  isActive,
  onClick,
}: {
  group: EntityGroupEntity
  isActive: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex items-center gap-3 w-full rounded-[16px] overflow-hidden transition-all duration-150 text-left"
      style={{
        background: isActive
          ? 'color-mix(in srgb, var(--cp-accent) 10%, var(--cp-surface))'
          : 'var(--cp-surface)',
        border: isActive
          ? '1px solid color-mix(in srgb, var(--cp-accent) 30%, transparent)'
          : '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
      }}
    >
      {/* server icon area */}
      <div
        className="shrink-0 flex flex-col items-center justify-center self-stretch px-3"
        style={{
          width: 56,
          background: 'color-mix(in srgb, var(--cp-warning) 10%, var(--cp-surface-2, var(--cp-surface)))',
        }}
      >
        <Server
          size={22}
          style={{ color: 'var(--cp-warning)' }}
        />
        {/* status LEDs */}
        <div className="flex gap-1 mt-1.5">
          <span
            className="block rounded-full"
            style={{
              width: 5, height: 5,
              background: 'var(--cp-success)',
            }}
          />
          <span
            className="block rounded-full"
            style={{
              width: 5, height: 5,
              background: 'var(--cp-success)',
            }}
          />
          <span
            className="block rounded-full"
            style={{
              width: 5, height: 5,
              background: group.canMessage ? 'var(--cp-accent)' : 'var(--cp-muted)',
            }}
          />
        </div>
      </div>

      {/* info */}
      <div className="flex-1 min-w-0 py-3 pr-3">
        <div
          className="truncate text-sm font-semibold"
          style={{ color: 'var(--cp-text)' }}
        >
          {group.displayName}
        </div>

        {group.description && (
          <div
            className="truncate text-[11px] mt-0.5"
            style={{ color: 'var(--cp-muted)' }}
          >
            {group.description}
          </div>
        )}

        <div className="flex items-center gap-3 mt-1.5">
          {/* member count */}
          <span
            className="text-[10px] font-medium"
            style={{ color: 'var(--cp-muted)' }}
          >
            {group.memberCount} members
          </span>

          {/* badges */}
          <div className="flex items-center gap-1.5">
            {group.isHostedBySelf && (
              <span
                className="inline-flex items-center gap-0.5 rounded-full px-1.5 py-0.5 text-[9px] font-semibold"
                style={{
                  background: 'color-mix(in srgb, var(--cp-success) 14%, transparent)',
                  color: 'var(--cp-success)',
                }}
              >
                Hosted
              </span>
            )}
            {group.canMessage && (
              <MessageSquare
                size={11}
                style={{ color: 'var(--cp-accent)' }}
              />
            )}
          </div>
        </div>
      </div>
    </button>
  )
}

/* ── Main mobile home screen ── */

interface MobileHomeScreenProps {
  selection: SidebarSelection | null
  onSelect: (sel: SidebarSelection) => void
  onAddUser?: () => void
  onRenameCollection?: (id: string, currentName: string) => void
  onDeleteCollection?: (id: string) => void
}

export function MobileHomeScreen({
  selection,
  onSelect,
  onAddUser,
  onRenameCollection,
  onDeleteCollection,
}: MobileHomeScreenProps) {
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

  const hostedGroups = entityGroups.filter((g) => g.isHostedBySelf)

  return (
    <div
      className="flex flex-col h-full w-full overflow-y-auto desktop-scrollbar"
      style={{ background: 'var(--cp-bg)' }}
    >
      <div className="px-4 pt-4 pb-6 space-y-5">
        {/* ── Section: Entities (ID badge grid) ── */}
        <div>
          <div className="flex items-center justify-between mb-2.5">
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

          {/* badge card grid – 3 columns, fill width */}
          <div className="grid grid-cols-3 gap-2.5">
            <BadgeCard
              entity={self}
              isActive={isEntityActive(self.id)}
              onClick={() => onSelect({ kind: 'entity', entityId: self.id })}
            />
            <BadgeCard
              entity={agent}
              isActive={isEntityActive(agent.id)}
              onClick={() => onSelect({ kind: 'entity', entityId: agent.id })}
            />
            {localUsers.map((u) => (
              <BadgeCard
                key={u.id}
                entity={u}
                isActive={isEntityActive(u.id)}
                onClick={() => onSelect({ kind: 'entity', entityId: u.id })}
              />
            ))}
          </div>
        </div>

        {/* ── Section: Hosted Servers ── */}
        {hostedGroups.length > 0 && (
          <div>
            <div className="flex items-center justify-between mb-2.5">
              <span
                className="text-[11px] font-semibold uppercase tracking-[0.18em]"
                style={{ color: 'var(--cp-muted)' }}
              >
                Hosted Servers
              </span>
            </div>

            <div className="space-y-2">
              {hostedGroups.map((g) => (
                <ServerCard
                  key={g.id}
                  group={g}
                  isActive={isEntityActive(g.id)}
                  onClick={() => onSelect({ kind: 'entity', entityId: g.id })}
                />
              ))}
            </div>
          </div>
        )}

        {/* divider */}
        <div
          style={{
            height: 1,
            background: 'color-mix(in srgb, var(--cp-border) 50%, transparent)',
          }}
        />

        {/* ── Section: Collections ── */}
        <div>
          <div className="flex items-center justify-between mb-2.5">
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
    </div>
  )
}
