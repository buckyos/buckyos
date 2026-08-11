import type { AppSummary } from '../../../api/app_mgr.ts'
import type {
  AgentInfo,
  UserContactSettings,
  UserDetail,
  UserInfo,
  UserStateString,
  UserTunnelBinding,
  UserType,
} from '../../../api/user_mgr.ts'
import type { SelfHostGroupSummary } from '../../../api/self_host_groups.ts'
import type {
  AgentEntity,
  EntityGroupEntity,
  LocalUserEntity,
  SelfEntity,
  SocialAccount,
  SocialAccountStatus,
  ZoneUserSource,
  ZoneUserStatus,
  ZoneUserType,
} from './types'

const fallbackCreatedAt = '1970-01-01T00:00:00Z'

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {}
}

function stringValue(value: unknown): string | undefined {
  if (typeof value === 'string' && value.trim()) return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  return undefined
}

function firstString(...values: unknown[]): string | undefined {
  for (const value of values) {
    const next = stringValue(value)
    if (next) return next
  }
  return undefined
}

function systemContactFromDetail(detail: UserDetail): UserContactSettings | undefined {
  if (detail.contact) return detail.contact
  const localProfile = asRecord(detail.local_profile)
  const privateExtra = asRecord(localProfile.private_extra)
  const systemContact = asRecord(privateExtra.system_contact)
  return Object.keys(systemContact).length > 0 ? systemContact as UserContactSettings : undefined
}

function stringArray(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value.map(stringValue).filter(Boolean) as string[]
  }
  const text = stringValue(value)
  if (!text) return []
  return text.split(',').map((item) => item.trim()).filter(Boolean)
}

function numberValue(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) return value
  if (typeof value !== 'string' || !value.trim()) return undefined
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : undefined
}

function booleanValue(value: unknown): boolean | undefined {
  if (typeof value === 'boolean') return value
  if (typeof value !== 'string') return undefined
  const normalized = value.trim().toLowerCase()
  if (normalized === 'true') return true
  if (normalized === 'false') return false
  return undefined
}

function isoFromUnix(value: unknown): string | undefined {
  const timestamp = numberValue(value)
  if (timestamp === undefined || timestamp <= 0) {
    return undefined
  }
  const millis = timestamp > 10_000_000_000 ? timestamp : timestamp * 1000
  return new Date(millis).toISOString()
}

function profileMap(profile: unknown, fallback: Record<string, string> = {}): Record<string, string> {
  const source = asRecord(profile)
  const extra = asRecord(source.extra)
  const links = asRecord(source.links)
  const websiteLink = asRecord(links.website)
  const result: Record<string, string> = {}

  for (const [key, value] of Object.entries({
    displayName: firstString(source.display_name, source.displayName, fallback.displayName),
    title: firstString(source.title, fallback.title),
    bio: firstString(source.bio, fallback.bio),
    location: firstString(source.location, fallback.location),
    organization: firstString(source.organization, extra.organization, fallback.organization),
    website: firstString(source.website, websiteLink.url, fallback.website),
    email: firstString(source.email, fallback.email),
    phone: firstString(source.phone, fallback.phone),
  })) {
    if (value) result[key] = value
  }

  for (const [key, value] of Object.entries(extra)) {
    if (result[key] !== undefined) continue
    const text = stringValue(value)
    if (text) result[key] = text
  }

  return Object.keys(result).length > 0 ? result : fallback
}

function socialStatus(value: unknown): SocialAccountStatus {
  if (value === 'pending' || value === 'error') return value
  return 'active'
}

export function toSocialAccounts(bindings: UserTunnelBinding[] | undefined): SocialAccount[] {
  return (bindings ?? []).map((binding) => ({
    id: `social-${binding.platform}-${binding.account_id}`,
    platform: binding.platform,
    accountId: binding.account_id,
    displayId: binding.display_id || binding.account_id,
    status: socialStatus(binding.status),
    isPublic: binding.meta?.public === 'true',
    canIdentify: binding.meta?.can_identify !== 'false',
    lastSyncAt: isoFromUnix(binding.last_sync_at),
    lastVerifiedAt: isoFromUnix(binding.last_sync_at),
  }))
}

function socialAccountsFromUnknown(value: unknown): SocialAccount[] {
  if (!Array.isArray(value)) return []
  const bindings = value.flatMap((item): UserTunnelBinding[] => {
    const record = asRecord(item)
    const platform = stringValue(record.platform)
    const accountId = stringValue(record.account_id ?? record.accountId)
    if (!platform || !accountId) return []
    return [{
      platform,
      account_id: accountId,
      display_id: stringValue(record.display_id ?? record.displayId),
      tunnel_instance_id: stringValue(record.tunnel_instance_id ?? record.tunnelId),
      status: stringValue(record.status),
      last_sync_at: typeof record.last_sync_at === 'number'
        ? record.last_sync_at
        : undefined,
      meta: asRecord(record.meta) as Record<string, string>,
    }]
  })
  return toSocialAccounts(bindings)
}

function userStateBase(state: UserStateString | undefined): string {
  return String(state ?? 'active').split(':', 1)[0]
}

function zoneUserType(type: UserType | string | undefined): ZoneUserType {
  const normalized = String(type ?? 'user').toLowerCase()
  if (normalized === 'admin' || normalized === 'root') return 'admin'
  if (normalized === 'limited' || normalized === 'guest') return 'limited'
  return 'user'
}

function zoneUserSource(type: UserType | string | undefined): ZoneUserSource {
  const normalized = String(type ?? 'user').toLowerCase()
  return normalized === 'limited' || normalized === 'guest' ? 'local-account' : 'primary-did'
}

function zoneUserStatus(state: UserStateString | undefined): ZoneUserStatus {
  const base = userStateBase(state)
  if (base === 'pending') return 'pending-invitation'
  if (base === 'suspended' || base === 'banned' || base === 'deleted') return 'suspended'
  return 'active'
}

function appDisplayNames(apps: AppSummary[]): string[] {
  return apps
    .map((app) => app.show_name || app.app_id)
    .filter(Boolean)
}

export function toSelfEntity(
  detail: UserDetail | null,
  apps: AppSummary[],
  fallback: SelfEntity,
): SelfEntity {
  if (!detail) return fallback

  const profile = asRecord(detail.profile)
  const contact = systemContactFromDetail(detail)
  const displayName = firstString(
    profile.display_name,
    profile.displayName,
    asRecord(detail.local_profile).display_name,
    detail.show_name,
    fallback.displayName,
  ) ?? fallback.displayName
  const bio = firstString(profile.bio, fallback.bio)
  const appNames = appDisplayNames(apps)

  return {
    ...fallback,
    id: detail.user_id,
    displayName,
    avatarUrl: firstString(profile.avatar, profile.avatar_url, profile.avatarUrl, fallback.avatarUrl),
    did: firstString(profile.did, asRecord(detail.local_profile).did, contact?.did, asRecord(detail.did_document).id, fallback.did),
    socialAccounts: toSocialAccounts(contact?.bindings),
    bio,
    email: firstString(profile.email, fallback.email),
    phone: firstString(profile.phone, fallback.phone),
    info: profileMap(profile, fallback.info),
    settings: {
      userType: String(detail.user_type),
      state: userStateBase(detail.state),
      resPool: detail.res_pool_id,
      passwordPolicy: detail.allow_password_change ? 'Change allowed' : 'Change restricted',
      apps: appNames.length > 0 ? appNames.join(', ') : (fallback.settings.apps ?? 'No apps'),
    },
    didDocument: detail.did_document ?? fallback.didDocument,
  }
}

export function toLocalUserEntity(
  user: UserInfo,
  apps: AppSummary[],
  now: string,
): LocalUserEntity {
  const source = zoneUserSource(user.user_type)
  const status = zoneUserStatus(user.state)
  const role = zoneUserType(user.user_type)
  const availableApps = apps
    .filter((app) => app.user_id === user.user_id)
    .map((app) => app.show_name || app.app_id)

  return {
    id: user.user_id,
    kind: 'local-user',
    displayName: user.show_name || user.user_id,
    did: `did:bns:${user.user_id}`,
    socialAccounts: [],
    role,
    source,
    status,
    credentialStatus: status === 'pending-invitation'
      ? 'invite-pending'
      : source === 'primary-did'
        ? 'passkey-ready'
        : 'password-set',
    canChangePassword: role !== 'limited' && status === 'active',
    storageUsed: 'Unknown',
    storageQuota: 'Unknown',
    lastActive: now,
    isOnline: status === 'active',
    availableApps,
    defaultGroup: 'zone-members',
    profile: {
      displayName: user.show_name || user.user_id,
      userId: user.user_id,
    },
    settings: {
      source: source === 'primary-did' ? 'Primary BNS / DID' : 'Local account',
      userType: String(user.user_type),
      state: userStateBase(user.state),
      apps: availableApps.length > 0 ? availableApps.join(', ') : 'Not loaded',
    },
    createdAt: fallbackCreatedAt,
  }
}

function agentStateToStatus(state: unknown): AgentEntity['status'] {
  const normalized = String(state ?? '').toLowerCase()
  if (normalized === 'running') return 'running'
  if (normalized === 'stopped' || normalized === 'new' || normalized === 'deleted') return 'stopped'
  return normalized ? 'error' : 'stopped'
}

function runtimeFromAgentInfo(
  runtimeValue: unknown,
  status: AgentEntity['status'],
  fallback: AgentEntity['runtime'],
): AgentEntity['runtime'] {
  const runtime = asRecord(runtimeValue)
  const available = runtime.available === true
  const uiSessions = Number(runtime.ui_session_count ?? 0)
  const workSessions = Number(runtime.work_session_count ?? runtime.total ?? 0)
  const workspaces = Number(runtime.workspace_count ?? 0)

  return {
    uptime: available ? 'Online' : fallback.uptime,
    memoryUsage: fallback.memoryUsage,
    cpuUsage: fallback.cpuUsage,
    lastActive: available ? new Date().toISOString() : fallback.lastActive,
    runningTasks: available && status === 'running' ? workSessions : 0,
    queuedTasks: 0,
    healthStatus: available
      ? status === 'running'
        ? workSessions > 10
          ? 'busy'
          : 'healthy'
        : 'degraded'
      : 'offline',
    uiSessions,
    workSessions,
    workspaces,
  }
}

export function toAgentEntity(
  agentInfo: AgentInfo | null,
  fallback: AgentEntity,
): AgentEntity {
  const raw = asRecord(agentInfo)
  const settings = asRecord(raw.settings)
  const spec = asRecord(raw.spec)
  const appDoc = asRecord(raw.app_doc ?? spec.app_doc)
  const profile = asRecord(settings.profile ?? raw.profile)
  const capabilities = [
    ...stringArray(settings.capabilities),
    ...stringArray(profile.capabilities),
  ]
  const dedupedCapabilities = Array.from(new Set(capabilities)).slice(0, 6)
  const status = agentStateToStatus(settings.state ?? raw.state ?? spec.state)
  const agentDid = firstString(raw.id, raw.did, raw.agent_did, settings.agent_did, fallback.did)
  const agentId = firstString(
    raw.agent_id,
    raw.agentId,
    raw.app_id,
    raw.appId,
    appDoc.name,
    agentDid,
    fallback.id,
  ) ?? fallback.id
  const displayName = firstString(
    profile.display_name,
    profile.displayName,
    settings.display_name,
    raw.display_name,
    raw.name,
    appDoc.show_name,
    appDoc.showName,
    agentId,
    fallback.displayName,
  ) ?? fallback.displayName

  return {
    ...fallback,
    id: agentId,
    displayName,
    avatarUrl: firstString(profile.avatar, profile.avatar_url, profile.avatarUrl, fallback.avatarUrl),
    did: agentDid,
    agentType: firstString(profile.agent_type, settings.agent_type, fallback.agentType) ?? fallback.agentType,
    version: firstString(raw.version, settings.version, appDoc.version, fallback.version) ?? fallback.version,
    status,
    capabilities: dedupedCapabilities.length > 0 ? dedupedCapabilities : fallback.capabilities,
    socialAccounts: socialAccountsFromUnknown(settings.bindings),
    info: {
      description: firstString(profile.description, raw.description, settings.description, fallback.info.description) ?? '',
      model: firstString(profile.model, settings.model, fallback.info.model) ?? '',
      appId: agentId,
      appType: 'agent',
    },
    settings: {
      owner: firstString(settings.owner, settings.owner_user_id, raw.owner_user_id, spec.user_id, fallback.settings.owner) ?? '',
      permissions: firstString(settings.permissions, fallback.settings.permissions) ?? 'Not configured',
      workspaceRoot: firstString(settings.workspace_root, settings.workspaceRoot, fallback.settings.workspaceRoot) ?? '',
      serviceState: String(settings.state ?? raw.state ?? spec.state ?? 'unknown'),
    },
    didDocument: Object.keys(raw).length > 0 ? raw : fallback.didDocument,
    runtime: runtimeFromAgentInfo(raw.runtime, status, fallback.runtime),
  }
}

export function toEntityGroupEntity(group: SelfHostGroupSummary | unknown): EntityGroupEntity {
  const raw = asRecord(group)
  const groupDid = firstString(raw.group_did, raw.groupDid)
  const groupId = firstString(raw.group_id, raw.groupId, groupDid, raw.id) ?? 'unknown-group'
  const memberIds = stringArray(raw.member_entity_ids ?? raw.memberEntityIds)
  const members = Array.isArray(raw.members) ? raw.members : []
  const ownerName = firstString(raw.owner_name, raw.ownerName, raw.owner, raw.owner_did)
  const createdAt =
    firstString(raw.created_at, raw.createdAt) ??
    isoFromUnix(raw.updated_at_ms ?? raw.updatedAtMs) ??
    fallbackCreatedAt

  return {
    id: `eg-${groupId}`,
    kind: 'entity-group',
    displayName: firstString(raw.display_name, raw.displayName, raw.name, groupDid, groupId) ?? groupId,
    avatarUrl: firstString(raw.avatar_url, raw.avatarUrl, raw.avatar),
    did: groupDid,
    socialAccounts: [],
    description: firstString(raw.description),
    memberCount: numberValue(raw.member_count ?? raw.memberCount) ?? (memberIds.length || members.length),
    memberIds,
    ownerName,
    isHostedBySelf: booleanValue(raw.is_hosted_by_self ?? raw.isHostedBySelf) ?? false,
    canMessage: booleanValue(raw.can_message ?? raw.canMessage) ?? false,
    createdAt,
  }
}
