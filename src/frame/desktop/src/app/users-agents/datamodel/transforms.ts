import type { AppSummary } from '../../../api/app_mgr.ts'
import type {
  AgentEntry,
  AgentInstallState,
  UserContactSettings,
  UserDetail,
  UserInfo,
  UserStateString,
  UserTunnelBinding,
  UserType,
} from '../../../api/user_mgr.ts'
import { agentDisplayName, JARVIS_GUIDE_ENTRY, usableImageUrl } from '../../agent-setup/model.ts'
import type {
  AgentEntity,
  AgentLifecycle,
  LocalUserEntity,
  SelfEntity,
  SocialAccount,
  SocialAccountStatus,
  ZoneUserSource,
  ZoneUserStatus,
  ZoneUserType,
} from './types'

const fallbackCreatedAt = '1970-01-01T00:00:00Z'

export function appListTargetUserIds(
  selfUserId: string | undefined,
  callerUserType: string | undefined,
  users: readonly Pick<UserInfo, 'user_id'>[],
): string[] {
  const targetUserIds = new Set<string>()
  if (selfUserId) targetUserIds.add(selfUserId)
  if (callerUserType === 'root' || callerUserType === 'admin') {
    for (const user of users) {
      if (user.user_id) targetUserIds.add(user.user_id)
    }
  }
  return [...targetUserIds]
}

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
  const localProfile = asRecord(detail.local_profile)
  const privateExtra = asRecord(localProfile.private_extra)
  const systemContact = asRecord(privateExtra.system_contact)
  return Object.keys(systemContact).length > 0 ? systemContact as UserContactSettings : undefined
}

function numberValue(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) return value
  if (typeof value !== 'string' || !value.trim()) return undefined
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : undefined
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

function userStateBase(state: UserStateString | undefined): string {
  return String(state ?? 'active').split(':', 1)[0]
}

function zoneUserType(type: UserType | string | undefined): ZoneUserType {
  const normalized = String(type ?? 'user').toLowerCase()
  if (normalized === 'admin' || normalized === 'root') return 'admin'
  if (normalized === 'limited' || normalized === 'guest') return 'limited'
  return 'user'
}

function zoneUserSource(isLocal: boolean | undefined): ZoneUserSource {
  return isLocal ? 'local-account' : 'primary-did'
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
  const source = zoneUserSource(user.is_local)
  const status = zoneUserStatus(user.state)
  const role = zoneUserType(user.user_type)
  const availableApps = apps.map((app) => app.show_name || app.app_id)

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
    canChangePassword: (user.allow_password_change ?? role !== 'limited') && status === 'active',
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

export function toVisibleLocalUserEntities(
  users: UserInfo[],
  appsByUser: ReadonlyMap<string, AppSummary[]>,
  selfUserId: string,
  now: string,
): LocalUserEntity[] {
  const seenUserIds = new Set([selfUserId])
  const visibleUsers: LocalUserEntity[] = []

  for (const user of users) {
    const userId = user.user_id.trim()
    if (
      !userId ||
      String(user.user_type).toLowerCase() === 'root' ||
      seenUserIds.has(userId)
    ) {
      continue
    }
    seenUserIds.add(userId)
    visibleUsers.push(toLocalUserEntity(user, appsByUser.get(userId) ?? [], now))
  }

  return visibleUsers
}

/**
 * An unfinished creation is `creating` / `failed`; a ready Agent takes the
 * state of its constructed App. Anything not recognised is `unknown`, never
 * an error.
 */
export function agentStateToStatus(
  installState: AgentInstallState | string | null | undefined,
  runtimeState: string | null | undefined,
): AgentLifecycle {
  switch (installState) {
    case 'provisioning':
    case 'bound':
      return 'creating'
    case 'failed':
      return 'failed'
    case 'ready':
      break
    default:
      return 'unknown'
  }
  switch (String(runtimeState ?? '').toLowerCase()) {
    case 'running':
    case 'restarting':
    case 'updating':
      return 'running'
    case 'new':
    case 'stopped':
    case 'stopping':
    case 'deleted':
      return 'stopped'
    default:
      return 'unknown'
  }
}

export function toAgentEntity(entry: AgentEntry): AgentEntity {
  const install = entry.install
  const tunnels = entry.settings?.msg_tunnels ?? []
  return {
    id: entry.agent_id,
    kind: 'agent',
    displayName: agentDisplayName(entry),
    avatarUrl: usableImageUrl(entry.profile?.avatar),
    did: entry.agent_did,
    socialAccounts: tunnels.map((tunnel) => ({
      id: `tunnel-${tunnel.platform}`,
      platform: tunnel.platform,
      accountId: tunnel.bot_account_id ?? '',
      displayId: tunnel.bot_account_id ? `@${tunnel.bot_account_id}` : '',
      status: install?.tunnel_state === 'failed' ? 'error' : install?.tunnel_state === 'bound' ? 'active' : 'pending',
      isPublic: false,
      canIdentify: false,
    })),
    createdAt: isoFromUnix(install?.created_at) ?? fallbackCreatedAt,
    name: entry.name,
    ownerUserId: entry.owner_user_id,
    ownerDid: entry.owner_did || undefined,
    bio: entry.profile?.bio?.trim() || undefined,
    nickname: entry.profile?.display_name?.trim() || undefined,
    status: agentStateToStatus(install?.state, entry.runtime?.state),
    install,
    settings: entry.settings,
    template: entry.template ?? null,
    runtime: entry.runtime ?? null,
    createdFromGuide: entry.settings?.desktop_entry === JARVIS_GUIDE_ENTRY,
  }
}
