import { useMemo, useState, type ReactNode } from 'react'
import {
  ArrowLeft,
  BellOff,
  Bot,
  ChevronRight,
  Pin,
  Search,
  SlidersHorizontal,
  User,
  Users,
} from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import type {
  Entity,
  EntityChildrenSection,
  EntityFilter,
  EntityType,
} from './types'

interface EntityListProps {
  entities: Entity[]
  selectedEntityId: string | null
  filter: EntityFilter
  searchQuery: string
  headerActions?: ReactNode
  enableDrilldownNavigation?: boolean
  useCompactInlineChildren?: boolean
  childNavigationTrigger?: 'row' | 'icon'
  drilldownPath?: string[]
  onDrilldownPathChange?: (path: string[]) => void
  onSelectEntity: (id: string) => void
  onFilterChange: (filter: EntityFilter) => void
  onSearchChange: (query: string) => void
}

const filters: { key: EntityFilter; labelKey: string }[] = [
  { key: 'all', labelKey: 'messagehub.filter.all' },
  { key: 'unread', labelKey: 'messagehub.filter.unread' },
  { key: 'people', labelKey: 'messagehub.filter.people' },
  { key: 'agents', labelKey: 'messagehub.filter.agents' },
  { key: 'groups', labelKey: 'messagehub.filter.groups' },
]

function entityMatchesFilter(entity: Entity, filter: EntityFilter): boolean {
  switch (filter) {
    case 'all': return true
    case 'unread': return entity.unreadCount > 0
    case 'pinned': return !!entity.isPinned
    case 'agents': return entity.type === 'agent'
    case 'groups': return entity.type === 'group'
    case 'people': return entity.type === 'person'
  }
}

function entityMatchesSearch(entity: Entity, query: string): boolean {
  if (!query) return true
  const q = query.toLowerCase()
  return (
    entity.name.toLowerCase().includes(q) ||
    (entity.lastMessage?.text.toLowerCase().includes(q) ?? false)
  )
}

function findEntityInTree(entities: Entity[], id: string): Entity | null {
  const queue = [...entities]

  while (queue.length > 0) {
    const current = queue.shift()
    if (!current) {
      continue
    }

    if (current.id === id) {
      return current
    }

    if (current.children?.length) {
      queue.push(...current.children)
    }
  }

  return null
}

function hasInlineChildren(entity: Entity): boolean {
  return Boolean(entity.children?.length) && entity.childrenMode !== 'drilldown'
}

function hasDrilldownChildren(entity: Entity): boolean {
  return Boolean(entity.children?.length) && entity.childrenMode === 'drilldown'
}

function formatTime(ts: number): string {
  const now = Date.now()
  const diff = now - ts
  const mins = Math.floor(diff / 60_000)
  if (mins < 1) return 'now'
  if (mins < 60) return `${mins}m`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h`
  const days = Math.floor(hours / 24)
  if (days < 7) return `${days}d`
  return new Date(ts).toLocaleDateString()
}

function getEntityTypeLabel(type: EntityType, t: (key: string, fallback: string) => string) {
  switch (type) {
    case 'agent':
      return t('messagehub.entityType.agent', 'Agent')
    case 'group':
      return t('messagehub.entityType.group', 'Group')
    case 'service':
      return t('messagehub.entityType.service', 'Service')
    default:
      return t('messagehub.entityType.person', 'Person')
  }
}

function getChildrenSections(entity: Entity): Array<EntityChildrenSection & { items: Entity[] }> {
  const children = entity.children ?? []
  if (children.length === 0) {
    return []
  }

  const childMap = new Map(children.map((child) => [child.id, child]))
  const configuredSections = (entity.childrenSections ?? []).map((section) => ({
    ...section,
    items: section.childIds
      .map((childId) => childMap.get(childId))
      .filter((child): child is Entity => Boolean(child)),
  }))

  const claimedIds = new Set(
    configuredSections.flatMap((section) => section.items.map((child) => child.id)),
  )
  const remainingChildren = children.filter((child) => !claimedIds.has(child.id))

  if (configuredSections.length === 0) {
    return [
      {
        id: `${entity.id}-children`,
        title: 'Items',
        childIds: children.map((child) => child.id),
        items: children,
      },
    ]
  }

  if (remainingChildren.length === 0) {
    return configuredSections
  }

  return [
    ...configuredSections,
    {
      id: `${entity.id}-more`,
      title: 'More',
      childIds: remainingChildren.map((child) => child.id),
      items: remainingChildren,
    },
  ]
}

function countUnread(entities: Entity[]): number {
  return entities.reduce((total, entity) => total + entity.unreadCount, 0)
}

function InlineExpandIndicator({ isExpanded }: { isExpanded: boolean }) {
  return (
    <span
      className="pointer-events-none absolute top-1/2 block -translate-y-1/2 transition-transform duration-150"
      aria-hidden="true"
      style={{
        left: 'calc(0.375rem - 2px)',
        width: 0,
        height: 0,
        borderTop: '4px solid transparent',
        borderBottom: '4px solid transparent',
        borderLeft: '5px solid var(--cp-muted)',
        transform: `translateY(-50%) rotate(${isExpanded ? 90 : 0}deg)`,
        transformOrigin: '38% 50%',
      }}
    />
  )
}

function EntityAvatar({
  entity,
  size = 48,
}: {
  entity: Entity
  size?: number
}) {
  const colors: Record<EntityType, string> = {
    person: 'var(--cp-accent)',
    agent: 'var(--cp-success)',
    group: 'var(--cp-warning)',
    service: 'var(--cp-danger)',
  }
  const iconSize = size >= 46 ? 20 : size >= 38 ? 17 : 15
  const icons: Record<EntityType, React.ReactNode> = {
    person: <User size={iconSize} />,
    agent: <Bot size={iconSize} />,
    group: <Users size={iconSize} />,
    service: <SlidersHorizontal size={iconSize} />,
  }

  return (
    <div
      className="relative flex-shrink-0 flex items-center justify-center rounded-full"
      style={{
        width: size,
        height: size,
        background: `color-mix(in srgb, ${colors[entity.type]} 18%, transparent)`,
        color: colors[entity.type],
      }}
    >
      {icons[entity.type]}
      {entity.isOnline ? (
        <span
          className="absolute bottom-0 right-0 rounded-full border-2"
          style={{
            width: Math.max(10, Math.round(size * 0.24)),
            height: Math.max(10, Math.round(size * 0.24)),
            background: 'var(--cp-success)',
            borderColor: 'var(--cp-surface)',
          }}
        />
      ) : null}
    </div>
  )
}

function TopLevelEntityItem({
  entity,
  isExpanded,
  isSelected,
  childNavigationTrigger,
  onSelect,
  onOpenChildren,
}: {
  entity: Entity
  isExpanded: boolean
  isSelected: boolean
  childNavigationTrigger: 'row' | 'icon'
  onSelect: () => void
  onOpenChildren: () => void
}) {
  const canOpenChildren = hasInlineChildren(entity) || hasDrilldownChildren(entity)
  const usesIconTrigger = canOpenChildren && childNavigationTrigger === 'icon'

  if (!usesIconTrigger) {
    return (
      <button
        onClick={onSelect}
        className="relative flex w-full items-center gap-2.5 px-3 py-2.5 text-left transition-colors"
        style={{
          background: isSelected
            ? 'color-mix(in srgb, var(--cp-accent) 14%, transparent)'
            : 'transparent',
        }}
        type="button"
      >
        {hasInlineChildren(entity) ? <InlineExpandIndicator isExpanded={isExpanded} /> : null}
        <EntityAvatar entity={entity} />
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-2">
            <div className="flex min-w-0 items-center gap-1.5">
              <span
                className="truncate text-sm font-semibold"
                style={{ color: 'var(--cp-text)' }}
              >
                {entity.name}
              </span>
              {entity.isPinned ? (
                <Pin size={12} style={{ color: 'var(--cp-muted)' }} />
              ) : null}
              {entity.isMuted ? (
                <BellOff size={12} style={{ color: 'var(--cp-muted)' }} />
              ) : null}
            </div>
            <div className="flex flex-shrink-0 items-center gap-1.5">
              {entity.lastMessage ? (
                <span
                  className="text-xs"
                  style={{ color: 'var(--cp-muted)' }}
                >
                  {formatTime(entity.lastMessage.timestamp)}
                </span>
              ) : null}
            </div>
          </div>
          {entity.lastMessage ? (
            <div className="mt-0.5 flex items-center justify-between gap-2">
              <p
                className="truncate text-xs"
                style={{ color: 'var(--cp-muted)' }}
              >
                {entity.lastMessage.senderName && entity.type !== 'person'
                  ? `${entity.lastMessage.senderName}: `
                  : ''}
                {entity.lastMessage.text}
              </p>
              {entity.unreadCount > 0 ? (
                <span
                  className="flex h-5 min-w-5 flex-shrink-0 items-center justify-center rounded-full px-1.5 text-[11px] font-semibold"
                  style={{
                    background: entity.isMuted
                      ? 'color-mix(in srgb, var(--cp-muted) 30%, transparent)'
                      : 'var(--cp-accent)',
                    color: entity.isMuted ? 'var(--cp-muted)' : '#fff',
                  }}
                >
                  {entity.unreadCount}
                </span>
              ) : null}
            </div>
          ) : null}
        </div>
      </button>
    )
  }

  return (
    <div
      className="relative flex w-full items-center gap-2.5 px-3 py-2.5 transition-colors"
      style={{
        background: isSelected
          ? 'color-mix(in srgb, var(--cp-accent) 14%, transparent)'
          : 'transparent',
      }}
    >
      {hasInlineChildren(entity) ? <InlineExpandIndicator isExpanded={isExpanded} /> : null}
      {usesIconTrigger ? (
        <button
          type="button"
          onClick={onOpenChildren}
          className="flex min-w-0 flex-shrink-0 items-center"
          aria-label={entity.name}
        >
          <EntityAvatar entity={entity} />
        </button>
      ) : (
        <EntityAvatar entity={entity} />
      )}
      <button
        type="button"
        onClick={onSelect}
        className="min-w-0 flex-1 text-left"
      >
        <div className="flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-1.5">
            <span
              className="truncate text-sm font-semibold"
              style={{ color: 'var(--cp-text)' }}
            >
              {entity.name}
            </span>
            {entity.isPinned ? (
              <Pin size={12} style={{ color: 'var(--cp-muted)' }} />
            ) : null}
            {entity.isMuted ? (
              <BellOff size={12} style={{ color: 'var(--cp-muted)' }} />
            ) : null}
          </div>
          <div className="flex flex-shrink-0 items-center gap-1.5">
            {entity.lastMessage ? (
              <span
                className="text-xs"
                style={{ color: 'var(--cp-muted)' }}
              >
                {formatTime(entity.lastMessage.timestamp)}
              </span>
            ) : null}
          </div>
        </div>
        {entity.lastMessage ? (
          <div className="mt-0.5 flex items-center justify-between gap-2">
            <p
              className="truncate text-xs"
              style={{ color: 'var(--cp-muted)' }}
            >
              {entity.lastMessage.senderName && entity.type !== 'person'
                ? `${entity.lastMessage.senderName}: `
                : ''}
              {entity.lastMessage.text}
            </p>
            {entity.unreadCount > 0 ? (
              <span
                className="flex h-5 min-w-5 flex-shrink-0 items-center justify-center rounded-full px-1.5 text-[11px] font-semibold"
                style={{
                  background: entity.isMuted
                    ? 'color-mix(in srgb, var(--cp-muted) 30%, transparent)'
                    : 'var(--cp-accent)',
                  color: entity.isMuted ? 'var(--cp-muted)' : '#fff',
                }}
              >
                {entity.unreadCount}
              </span>
            ) : null}
          </div>
        ) : null}
      </button>
    </div>
  )
}

function InlineChildItem({
  entity,
  isSelected,
  onSelect,
}: {
  entity: Entity
  isSelected: boolean
  onSelect: () => void
}) {
  return (
    <button
      onClick={onSelect}
      className="flex w-full items-center gap-2.5 px-4 py-1.5 text-left transition-colors"
      style={{
        background: isSelected
          ? 'color-mix(in srgb, var(--cp-accent) 11%, transparent)'
          : 'transparent',
      }}
      type="button"
    >
      <EntityAvatar entity={entity} size={30} />
      <span
        className="min-w-0 flex-1 truncate text-[13px] font-medium"
        style={{ color: 'var(--cp-text)' }}
      >
        {entity.name}
      </span>
      {entity.unreadCount > 0 ? (
        <span
          className="flex h-4.5 min-w-4.5 flex-shrink-0 items-center justify-center rounded-full px-1.5 text-[10px] font-semibold"
          style={{
            background: 'color-mix(in srgb, var(--cp-accent) 16%, transparent)',
            color: 'var(--cp-accent)',
          }}
        >
          {entity.unreadCount}
        </span>
      ) : null}
    </button>
  )
}

function StatPill({
  label,
  value,
}: {
  label: string
  value: string
}) {
  return (
    <div
      className="rounded-full px-3 py-1.5"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface) 76%, var(--cp-accent) 8%)',
        border: '1px solid color-mix(in srgb, var(--cp-border) 88%, var(--cp-accent) 12%)',
      }}
    >
      <span
        className="text-[10px] font-semibold uppercase tracking-[0.14em]"
        style={{ color: 'var(--cp-muted)' }}
      >
        {label}
      </span>
      <span
        className="ml-2 text-sm font-semibold"
        style={{ color: 'var(--cp-text)' }}
      >
        {value}
      </span>
    </div>
  )
}

function DrilldownEntityRow({
  entity,
  isSelected,
  onSelect,
  typeLabel,
}: {
  entity: Entity
  isSelected: boolean
  onSelect: () => void
  typeLabel: string
}) {
  const metaText = entity.lastMessage
    ? formatTime(entity.lastMessage.timestamp)
    : entity.statusText

  return (
    <button
      onClick={onSelect}
      className="flex w-full items-center gap-3 px-4 py-2 text-left transition-colors"
      style={{
        background: isSelected
          ? 'color-mix(in srgb, var(--cp-accent) 12%, transparent)'
          : 'transparent',
      }}
      type="button"
    >
      <EntityAvatar entity={entity} size={34} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span
            className="truncate text-sm font-semibold"
            style={{ color: 'var(--cp-text)' }}
          >
            {entity.name}
          </span>
          <span
            className="rounded-full px-2 py-0.5 text-[10px] font-medium"
            style={{
              background: 'color-mix(in srgb, var(--cp-text) 6%, transparent)',
              color: 'var(--cp-muted)',
            }}
          >
            {typeLabel}
          </span>
        </div>
      </div>
      <div className="flex flex-shrink-0 items-center gap-2">
        {metaText ? (
          <span
            className="text-[11px]"
            style={{ color: 'var(--cp-muted)' }}
          >
            {metaText}
          </span>
        ) : null}
        {entity.unreadCount > 0 ? (
          <span
            className="flex h-5 min-w-5 items-center justify-center rounded-full px-1.5 text-[10px] font-semibold"
            style={{
              background: 'color-mix(in srgb, var(--cp-accent) 16%, transparent)',
              color: 'var(--cp-accent)',
            }}
          >
            {entity.unreadCount}
          </span>
        ) : null}
        <ChevronRight size={15} style={{ color: 'var(--cp-muted)' }} />
      </div>
    </button>
  )
}

function DrilldownPanel({
  entity,
  selectedEntityId,
  onSelectEntity,
}: {
  entity: Entity
  selectedEntityId: string | null
  onSelectEntity: (id: string) => void
}) {
  const { t } = useI18n()
  const sections = getChildrenSections(entity)
  const children = entity.children ?? []
  const unreadTotal = countUnread(children)

  return (
    <div className="flex-1 overflow-y-auto px-3 pb-3 shell-scrollbar">
      <section
        className="mx-1 rounded-[24px] px-4 py-4"
        style={{
          background:
            'linear-gradient(180deg, color-mix(in srgb, var(--cp-accent) 8%, var(--cp-surface) 92%), color-mix(in srgb, var(--cp-surface-2) 92%, transparent))',
          border: '1px solid color-mix(in srgb, var(--cp-border) 86%, var(--cp-accent) 14%)',
        }}
      >
        <div className="flex items-start gap-3">
          <EntityAvatar entity={entity} size={42} />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <h2
                className="truncate text-base font-semibold"
                style={{ color: 'var(--cp-text)' }}
              >
                {entity.name}
              </h2>
              {entity.statusText ? (
                <span
                  className="rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-[0.12em]"
                  style={{
                    background: 'color-mix(in srgb, var(--cp-text) 6%, transparent)',
                    color: 'var(--cp-muted)',
                  }}
                >
                  {entity.statusText}
                </span>
              ) : null}
            </div>
            <p
              className="mt-1 text-sm leading-6"
              style={{ color: 'var(--cp-muted)' }}
            >
              {entity.drilldownDescription
                ?? entity.lastMessage?.text
                ?? entity.statusText}
            </p>
          </div>
        </div>

        <div className="mt-4 flex flex-wrap gap-2">
          <StatPill
            label={t('messagehub.drilldownItems', 'Items')}
            value={`${children.length}`}
          />
          <StatPill
            label={t('messagehub.drilldownSections', 'Sections')}
            value={`${sections.length}`}
          />
          {unreadTotal > 0 ? (
            <StatPill
              label={t('messagehub.filter.unread', 'Unread')}
              value={`${unreadTotal}`}
            />
          ) : null}
        </div>
      </section>

      <div className="mt-4 space-y-4">
        {sections.map((section) => (
          <section key={section.id} className="mx-1">
            <div className="mb-2 px-1">
              <div className="flex items-end justify-between gap-3">
                <h3
                  className="text-xs font-semibold uppercase tracking-[0.14em]"
                  style={{ color: 'var(--cp-muted)' }}
                >
                  {section.title}
                </h3>
                <span
                  className="text-[11px]"
                  style={{ color: 'var(--cp-muted)' }}
                >
                  {section.items.length}
                </span>
              </div>
              {section.description ? (
                <p
                  className="mt-1 text-xs leading-5"
                  style={{ color: 'var(--cp-muted)' }}
                >
                  {section.description}
                </p>
              ) : null}
            </div>
            <div
              className="rounded-[20px] p-1.5"
              style={{
                background: 'color-mix(in srgb, var(--cp-text) 4%, transparent)',
                border: '1px solid color-mix(in srgb, var(--cp-border) 92%, transparent)',
              }}
            >
              {section.items.map((child) => (
                <DrilldownEntityRow
                  key={child.id}
                  entity={child}
                  isSelected={selectedEntityId === child.id}
                  onSelect={() => onSelectEntity(child.id)}
                  typeLabel={getEntityTypeLabel(child.type, t)}
                />
              ))}
            </div>
          </section>
        ))}
      </div>
    </div>
  )
}

export function EntityList({
  entities,
  selectedEntityId,
  filter,
  searchQuery,
  headerActions,
  enableDrilldownNavigation = true,
  useCompactInlineChildren = true,
  childNavigationTrigger = 'row',
  drilldownPath,
  onDrilldownPathChange,
  onSelectEntity,
  onFilterChange,
  onSearchChange,
}: EntityListProps) {
  const { t } = useI18n()
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set())
  const [internalDrilldownPath, setInternalDrilldownPath] = useState<string[]>([])
  const resolvedDrilldownPath = drilldownPath ?? internalDrilldownPath

  const setDrilldownPath = (next: string[] | ((prev: string[]) => string[])) => {
    const current = resolvedDrilldownPath
    const value = typeof next === 'function'
      ? next(current)
      : next

    if (drilldownPath === undefined) {
      setInternalDrilldownPath(value)
    }

    onDrilldownPathChange?.(value)
  }

  const filtered = entities
    .filter((entity) => entityMatchesFilter(entity, filter))
    .filter((entity) => entityMatchesSearch(entity, searchQuery))

  const drilldownEntities = useMemo(
    () => resolvedDrilldownPath
      .map((entityId) => findEntityInTree(entities, entityId))
      .filter((entity): entity is Entity => Boolean(entity)),
    [resolvedDrilldownPath, entities],
  )
  const drilldownEntity = drilldownEntities.at(-1) ?? null

  const toggleInlineGroup = (id: string) => {
    setExpandedGroups((prev) => {
      const next = new Set(prev)
      if (next.has(id)) {
        next.delete(id)
      } else {
        next.add(id)
      }
      return next
    })
  }

  const handleEntitySelect = (entity: Entity) => {
    onSelectEntity(entity.id)

    if (enableDrilldownNavigation && hasDrilldownChildren(entity)) {
      setDrilldownPath((prev) => (
        prev.at(-1) === entity.id ? prev : [...prev, entity.id]
      ))
      return
    }

    if (hasInlineChildren(entity) || hasDrilldownChildren(entity)) {
      toggleInlineGroup(entity.id)
    }
  }

  const breadcrumbLabel = drilldownEntities.map((entity) => entity.name).join(' / ')

  return (
    <div
      className="flex h-full flex-col"
      style={{ background: 'var(--cp-surface)' }}
    >
      <div className="px-4 pt-4 pb-2">
        <div className="mb-3 flex items-center justify-between gap-3">
          <h1
            className="min-w-0 text-lg font-bold"
            style={{ color: 'var(--cp-text)' }}
          >
            {t('messagehub.title', 'MessageHub')}
          </h1>
          {headerActions ? (
            <div className="flex flex-shrink-0 items-center gap-2">
              {headerActions}
            </div>
          ) : null}
        </div>

        {enableDrilldownNavigation && drilldownEntity ? (
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => setDrilldownPath((prev) => prev.slice(0, -1))}
              className="flex h-9 items-center gap-1.5 rounded-full px-3 text-sm font-medium"
              style={{
                color: 'var(--cp-accent)',
                background: 'color-mix(in srgb, var(--cp-accent) 12%, transparent)',
              }}
            >
              <ArrowLeft size={14} />
              {t('messagehub.backToEntities', 'Back to entities')}
            </button>
            <span
              className="min-w-0 truncate text-xs"
              style={{ color: 'var(--cp-muted)' }}
            >
              {breadcrumbLabel}
            </span>
          </div>
        ) : (
          <div
            className="flex items-center gap-2 rounded-xl px-3 py-2"
            style={{
              background: 'color-mix(in srgb, var(--cp-text) 6%, transparent)',
            }}
          >
            <Search size={16} style={{ color: 'var(--cp-muted)' }} />
            <input
              type="text"
              value={searchQuery}
              onChange={(event) => onSearchChange(event.target.value)}
              placeholder={t('messagehub.search', 'Search...')}
              className="flex-1 border-none bg-transparent text-sm outline-none"
              style={{ color: 'var(--cp-text)' }}
            />
          </div>
        )}
      </div>

      {enableDrilldownNavigation && drilldownEntity ? (
        <DrilldownPanel
          entity={drilldownEntity}
          selectedEntityId={selectedEntityId}
          onSelectEntity={onSelectEntity}
        />
      ) : (
        <>
          <div className="flex items-center gap-1.5 overflow-x-auto px-4 py-2">
            {filters.map((item) => (
              <button
                key={item.key}
                onClick={() => onFilterChange(item.key)}
                className="whitespace-nowrap rounded-full px-3 py-1 text-xs font-medium transition-colors"
                style={{
                  background:
                    filter === item.key
                      ? 'var(--cp-accent)'
                      : 'color-mix(in srgb, var(--cp-text) 8%, transparent)',
                  color: filter === item.key ? '#fff' : 'var(--cp-muted)',
                }}
                type="button"
              >
                {t(item.labelKey, item.key)}
              </button>
            ))}
          </div>

          <div className="flex-1 overflow-y-auto pb-2 shell-scrollbar">
            {filtered.length === 0 ? (
              <div className="flex h-32 items-center justify-center">
                <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>
                  {t('messagehub.noResults', 'No conversations found')}
                </p>
              </div>
            ) : (
              filtered.map((entity) => {
                const isExpanded = expandedGroups.has(entity.id)

                return (
                  <div key={entity.id} className="mb-1">
                    <TopLevelEntityItem
                      entity={entity}
                      isExpanded={isExpanded}
                      isSelected={selectedEntityId === entity.id}
                      childNavigationTrigger={childNavigationTrigger}
                      onSelect={() => (
                        childNavigationTrigger === 'icon' && hasDrilldownChildren(entity)
                          ? (enableDrilldownNavigation
                            ? setDrilldownPath((prev) => (
                              prev.at(-1) === entity.id ? prev : [...prev, entity.id]
                            ))
                            : onSelectEntity(entity.id))
                          : childNavigationTrigger === 'icon'
                          ? (setDrilldownPath([]), onSelectEntity(entity.id))
                          : handleEntitySelect(entity)
                      )}
                      onOpenChildren={() => {
                        if (childNavigationTrigger === 'icon') {
                          if (enableDrilldownNavigation && hasDrilldownChildren(entity)) {
                            setDrilldownPath((prev) => (
                              prev.at(-1) === entity.id ? prev : [...prev, entity.id]
                            ))
                          } else if (hasInlineChildren(entity) || hasDrilldownChildren(entity)) {
                            toggleInlineGroup(entity.id)
                          }
                          return
                        }

                        handleEntitySelect(entity)
                      }}
                    />

                    {hasInlineChildren(entity) && isExpanded ? (
                      <div
                        className="ml-7 mt-1 border-l pl-2"
                        style={{
                          borderColor: 'color-mix(in srgb, var(--cp-border) 90%, transparent)',
                        }}
                      >
                        {entity.children?.map((child) => (
                          useCompactInlineChildren ? (
                            <InlineChildItem
                              key={child.id}
                              entity={child}
                              isSelected={selectedEntityId === child.id}
                              onSelect={() => onSelectEntity(child.id)}
                            />
                          ) : (
                            <TopLevelEntityItem
                              key={child.id}
                              entity={child}
                              isExpanded={false}
                              isSelected={selectedEntityId === child.id}
                              childNavigationTrigger="row"
                              onSelect={() => onSelectEntity(child.id)}
                              onOpenChildren={() => onSelectEntity(child.id)}
                            />
                          )
                        ))}
                      </div>
                    ) : null}
                  </div>
                )
              })
            )}
          </div>
        </>
      )}
    </div>
  )
}
