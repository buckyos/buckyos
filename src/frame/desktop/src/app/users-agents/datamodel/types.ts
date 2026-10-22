import { z } from 'zod'
import type {
  AgentRuntimeBinding,
  AgentSettings,
  AgentStatus,
  AgentTemplateBinding,
} from '../../../api/user_mgr.ts'
import type { AssignableUserType, ContactProjectionRule } from '../../../api/user_type.ts'

/* ── Users & Agents – UI datamodel type definitions ── */

// ── Entity types ──

export type EntityKind = 'self' | 'agent' | 'local-user' | 'entity-group'

export type SocialAccountStatus = 'active' | 'pending' | 'error'

export interface SocialAccount {
  id: string
  platform: string
  accountId: string
  displayId: string
  status: SocialAccountStatus
  isPublic: boolean
  canIdentify: boolean
  lastSyncAt?: string
  lastVerifiedAt?: string
}

export const socialAccountPlatformOptions = [
  { id: 'github', label: 'GitHub' },
  { id: 'x', label: 'X' },
  { id: 'telegram', label: 'Telegram' },
  { id: 'discord', label: 'Discord' },
  { id: 'linkedin', label: 'LinkedIn' },
  { id: 'mastodon', label: 'Mastodon' },
  { id: 'wechat', label: 'WeChat' },
  { id: 'email', label: 'Email' },
  { id: 'phone', label: 'Phone' },
] as const

export interface EntityBase {
  id: string
  kind: EntityKind
  displayName: string
  avatarUrl?: string
  did?: string
  socialAccounts: SocialAccount[]
  createdAt: string
}

// ── Self ──

export interface SelfEntity extends EntityBase {
  kind: 'self'
  bio?: string
  email?: string
  phone?: string
  info: Record<string, string>          // lightweight public profile
  settings: Record<string, string>
  didDocument?: Record<string, unknown> // serious identity data
  twoFactorEnabled: boolean
  lastLogin: string
}

// ── Agent ──

/** What the Agent is doing now: an unfinished creation, or the state of its constructed App. */
export type AgentLifecycle = 'creating' | 'failed' | 'running' | 'stopped' | 'unknown'

export interface AgentEntity extends EntityBase {
  kind: 'agent'
  /** The Agent's user name (first label of its AgentId). */
  name: string
  ownerUserId: string
  ownerDid?: string
  bio?: string
  nickname?: string
  status: AgentLifecycle
  install: AgentStatus
  settings: AgentSettings
  template?: AgentTemplateBinding | null
  runtime?: AgentRuntimeBinding | null
  /** Created from the desktop Jarvis guide (it is that entry's Agent). */
  createdFromGuide: boolean
}

// ── Local space user ──

export type ZoneUserSource = 'primary-did' | 'local-account'
/** The account type as listed (the owner record is not listed); `unknown` is a type this Desktop does not know. */
export type ZoneUserType = AssignableUserType | 'unknown'
export type ZoneUserStatus = 'active' | 'pending-invitation' | 'suspended'
export type CredentialStatus = 'invite-pending' | 'password-set' | 'passkey-ready'

export interface ZoneInvitation {
  inviteUrl: string
  targetZone: string
  requestedDid: string
  expiresAt: string
  bindedZoneListKey: 'binded_zone_list'
}

export interface LocalUserEntity extends EntityBase {
  kind: 'local-user'
  role: ZoneUserType
  source: ZoneUserSource
  status: ZoneUserStatus
  credentialStatus: CredentialStatus
  /** The account may change its password now (allowed and active). */
  canChangePassword: boolean
  /** Effective permissions and the per-account overrides behind them (null = type default). */
  passwordChangeAllowed: boolean
  passwordChangeOverride: boolean | null
  contactEditAllowed: boolean
  contactEditOverride: boolean | null
  /** System user group ids. */
  groups: string[]
  storageUsed: string
  storageQuota: string
  lastActive: string
  isOnline: boolean
  availableApps: string[]
  profile: Record<string, string>
  settings: Record<string, string>
  invitation?: ZoneInvitation
}

// ── Entity group ──

export interface EntityGroupEntity extends EntityBase {
  kind: 'entity-group'
  description?: string
  memberCount: number
  memberIds: string[]
  ownerName?: string
  isHostedBySelf: boolean
  canMessage: boolean
}

// ── System user groups (admin only) ──

export interface SystemUserGroup {
  id: string
  name: string
  description?: string
  members: string[]
}

export interface UserGroupsInfo {
  groups: SystemUserGroup[]
  projectionRules: ContactProjectionRule[]
  /** No rules were saved; the Zone uses the defaults. */
  projectionsAreDefault: boolean
}

// ── Union type ──

export type AnyEntity =
  | SelfEntity
  | AgentEntity
  | LocalUserEntity
  | EntityGroupEntity

// ── View state ──

export type SidebarSelection =
  | { kind: 'entity'; entityId: string }
  | { kind: 'self' }
  | { kind: 'user-groups' }

// ── Store snapshot ──

export interface UsersAgentsSnapshot {
  self: SelfEntity
  agents: AgentEntity[]
  localUsers: LocalUserEntity[]
  entityGroups: EntityGroupEntity[]
  /** Limited accounts cannot create Agents. */
  canCreateAgents: boolean
  /** The signed-in account manages users, groups and projection rules. */
  viewerIsAdmin: boolean
  /** Null when the viewer is not an admin or the groups could not be read. */
  userGroups: UserGroupsInfo | null
}

// ── New user wizard ──

export const newZoneUserInputSchema = z
  .object({
    username: z
      .string()
      .trim()
      .min(1, 'usersAgents.newUser.error.usernameRequired')
      .max(64, 'usersAgents.newUser.error.usernameLength')
      .regex(
        /^[a-z0-9_.-]+$/i,
        'usersAgents.newUser.error.usernameChars',
      ),
    displayName: z.string().trim().min(1, 'usersAgents.newUser.error.displayNameRequired').max(64),
    password: z.string().min(8, 'usersAgents.newUser.error.passwordLength').max(128),
    confirmPassword: z.string().max(128),
    userType: z.enum(['user', 'limited']),
    groups: z.array(z.string()),
  })
  .superRefine((value, ctx) => {
    if (['root', 'system', 'admin', 'guest'].includes(value.username.trim().toLowerCase())) {
      ctx.addIssue({
        code: 'custom',
        path: ['username'],
        message: 'usersAgents.newUser.error.usernameReserved',
      })
    }
    if (value.password !== value.confirmPassword) {
      ctx.addIssue({
        code: 'custom',
        path: ['confirmPassword'],
        message: 'usersAgents.newUser.error.passwordMismatch',
      })
    }
  })

export type NewZoneUserInput = z.infer<typeof newZoneUserInputSchema>
