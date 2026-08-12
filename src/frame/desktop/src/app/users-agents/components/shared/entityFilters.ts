import type { AgentEntity, AnyEntity, EntityGroupEntity, LocalUserEntity, SelfEntity } from '../../datamodel/types'

export type InternalEntityFilter = 'all' | 'users' | 'agents' | 'groups' | 'online'

export function getInternalEntities(
  self: SelfEntity,
  agents: AgentEntity[],
  localUsers: LocalUserEntity[],
  entityGroups: EntityGroupEntity[],
): AnyEntity[] {
  const entities: AnyEntity[] = [self, ...localUsers]
  entities.push(...agents.filter((agent) => agent.id !== 'agent-unavailable'))
  entities.push(...entityGroups.filter((group) => group.isHostedBySelf))
  return entities
}

export function filterInternalEntities(
  entities: AnyEntity[],
  query: string,
  filter: InternalEntityFilter,
) {
  const normalizedQuery = query.trim().toLowerCase()

  return entities.filter((entity) => {
    if (filter === 'users' && entity.kind !== 'self' && entity.kind !== 'local-user') return false
    if (filter === 'agents' && entity.kind !== 'agent') return false
    if (filter === 'groups' && entity.kind !== 'entity-group') return false
    if (filter === 'online' && !isOnlineEntity(entity)) return false

    if (!normalizedQuery) return true
    return getEntitySearchText(entity).includes(normalizedQuery)
  })
}

export function isOnlineEntity(entity: AnyEntity) {
  if (entity.kind === 'self') return true
  if (entity.kind === 'local-user') return entity.isOnline
  if (entity.kind === 'agent') return entity.status === 'running'
  if (entity.kind === 'entity-group') return entity.isHostedBySelf
  return false
}

function getEntitySearchText(entity: AnyEntity) {
  const values = [entity.displayName, entity.did, entity.kind]

  if (entity.kind === 'self') {
    values.push(entity.bio, entity.email, entity.phone, ...Object.values(entity.info), ...Object.values(entity.settings))
  }

  if (entity.kind === 'local-user') {
    values.push(
      entity.role,
      entity.source,
      entity.status,
      entity.credentialStatus,
      entity.defaultGroup,
      ...entity.availableApps,
      ...Object.values(entity.profile),
      ...Object.values(entity.settings),
    )
  }

  if (entity.kind === 'agent') {
    values.push(
      entity.agentType,
      entity.status,
      entity.version,
      ...entity.capabilities,
      ...Object.values(entity.info),
      ...Object.values(entity.settings),
      entity.runtime.healthStatus,
    )
  }

  if (entity.kind === 'entity-group') {
    values.push(
      entity.description,
      entity.ownerName,
      String(entity.memberCount),
      entity.isHostedBySelf ? 'hosted' : 'joined',
      entity.canMessage ? 'messageable' : undefined,
    )
  }

  return values.filter(Boolean).join(' ').toLowerCase()
}
