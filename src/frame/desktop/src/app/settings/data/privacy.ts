/* Privacy overview: who can reach what, gathered from the services that actually decide it.
 *
 *  - public access: app availability (a `guest` allow makes the gateway serve the app without a
 *    session), the HomeStation entries an anonymous reader gets, AI Workspace `public` grants;
 *  - messaging: msg-center's fixed routing by contact access level, with the caller's counts;
 *  - app & agent access: approved permissions, folder mounts and availability of each app,
 *    group / external-platform reach of each Agent.
 *
 * Every source is read independently: one unreachable service fails only its own section. */

import { buckyos } from 'buckyos'
import { fetchCurrentAccount } from '../../../api/account'
import { isPrivilegedUserType, parseUserGroupRef } from '../../../api/user_type'
import {
  fetchAppAvailability,
  fetchAppDetails,
  fetchAppList,
  type AppAvailabilityPolicy,
  type AppSummary,
} from '../../../api/app_mgr'
import { fetchAgentList } from '../../../api/user_mgr'
import type { LocalizedText } from '../../../presentation/model'
import { currentZoneHost } from '../../../presentation/zone'
import { isMockRuntime } from '../../../runtime'
import { backendAppText } from '../../backend-apps'
import { AiwsClient, unwrap as unwrapAiws } from '../../aiworkspace/api/client'
import { resolveTransport as resolveAiwsTransport } from '../../aiworkspace/api/transport'
import { homeStationTransport } from '../../homestation/api/transport'
import type { AudienceSpec } from '../../homestation/datamodel/types'
import { workspaceOriginOn, workspaceSlug } from '../../aiworkspace/links'

export type Section<T> = { ok: true; data: T } | { ok: false; error: string }

/** Who may use an app besides its owner, read from its availability policy. */
export type AppAudience = 'owner' | 'admins' | 'users' | 'selected' | 'public'

export interface AppMount {
  /** Path inside the app's container. */
  path: string
  /** Host folder under the BuckyOS data root. */
  target: string
  access: string
  kind: 'data' | 'cache' | 'external'
}

export interface AppAccessEntry {
  appInstanceId: string
  /** The app document's title (per language) or its plain name. */
  name: LocalizedText
  iconUrl: string | null
  ownerUserId: string
  isOwn: boolean
  runtimeType: string
  /** Only for apps the caller owns (or any app for an administrator). */
  detail: Section<{
    audience: AppAudience
    sharedUsers: string[]
    /** System user group ids the app is open to. */
    sharedGroups: string[]
    permissions: { scope: string; actions: string[]; required: boolean }[]
    mounts: AppMount[]
  }> | null
}

export interface AgentAccessEntry {
  agentId: string
  name: string
  ownerUserId: string
  enabled: boolean
  allowGroup: boolean
  tunnels: string[]
  runtimeApp: string | null
}

export interface PublicApp {
  appInstanceId: string
  name: LocalizedText
  urls: string[]
}

export interface HomeStationExposure {
  url: string
  /** Entries an anonymous reader sees on the first page; `more` when there are further pages. */
  publicEntries: number
  more: boolean
  defaultAudience: AudienceSpec['kind']
  commentsOpen: boolean
}

export interface PublicWorkspace {
  workspaceId: string
  title: string
  url: string
}

export interface MessagingCounts {
  blocked: number
  strangers: number
}

export interface PrivacyOverview {
  userId: string
  isAdmin: boolean
  zoneHost: string
  publicApps: Section<PublicApp[]>
  homestation: Section<HomeStationExposure> | null
  workspaces: Section<PublicWorkspace[]> | null
  messaging: Section<MessagingCounts>
  apps: Section<AppAccessEntry[]>
  agents: Section<AgentAccessEntry[]>
}

const PUBLISHED_PROBE_LIMIT = 50
const CONTACT_COUNT_LIMIT = 1000

function message(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  return 'unavailable'
}

async function section<T>(work: () => Promise<T>): Promise<Section<T>> {
  try {
    return { ok: true, data: await work() }
  } catch (error) {
    return { ok: false, error: message(error) }
  }
}

async function required<T>(call: Promise<{ data: T | null; error: unknown }>): Promise<T> {
  const { data, error } = await call
  if (data === null) throw error ?? new Error('unavailable')
  return data
}

function appName(app: AppSummary): LocalizedText {
  return backendAppText(app, false).title ?? app.app_id
}

export function audienceOf(policy: AppAvailabilityPolicy): { audience: AppAudience; sharedUsers: string[]; sharedGroups: string[] } {
  const allowedRules = policy.group_rules.filter((rule) => rule.effect === 'allow')
  const allowed = new Set<string>(allowedRules.map((rule) => rule.group_id))
  const sharedUsers = policy.user_rules.filter((rule) => rule.effect === 'allow').map((rule) => rule.user_id)
  const sharedGroups = allowedRules
    .map((rule) => parseUserGroupRef(rule.group_id))
    .flatMap((ref) => (ref?.kind === 'group' ? [ref.groupId] : []))
  if (allowed.has('guest')) return { audience: 'public', sharedUsers, sharedGroups }
  if (allowed.has('users') || allowed.has('limited')) return { audience: 'users', sharedUsers, sharedGroups }
  if (allowed.has('admins')) return { audience: 'admins', sharedUsers, sharedGroups }
  return { audience: sharedUsers.length > 0 || sharedGroups.length > 0 ? 'selected' : 'owner', sharedUsers, sharedGroups }
}

/** `spec.spec_config.{data,local_cache,external}_mount_point` of an `AppServiceSpec`. */
export function mountsOf(spec: Record<string, unknown>): AppMount[] {
  const config = (spec.spec_config ?? {}) as Record<string, unknown>
  const kinds: [string, AppMount['kind']][] = [
    ['data_mount_point', 'data'],
    ['local_cache_mount_point', 'cache'],
    ['external_mount_point', 'external'],
  ]
  return kinds.flatMap(([field, kind]) =>
    Object.entries((config[field] ?? {}) as Record<string, { target_path?: string; access?: string }>).map(([path, mount]) => ({
      path,
      target: String(mount?.target_path ?? ''),
      access: String(mount?.access ?? 'read_only'),
      kind,
    })),
  )
}

function permissionsOf(spec: Record<string, unknown>) {
  const items = Array.isArray(spec.permission) ? (spec.permission as Record<string, unknown>[]) : []
  return items.map((item) => ({
    scope: String(item.scope_path ?? ''),
    actions: Array.isArray(item.actions) ? item.actions.map(String) : [],
    required: item.required === true,
  }))
}

function appUrls(app: AppSummary, zoneHost: string): string[] {
  // A web host is the app's label under the Zone host (see backend-apps.ts).
  return app.web_hosts
    .map((host) => host.trim())
    .filter(Boolean)
    .map((host) => `https://${host.endsWith(zoneHost) ? host : `${host}.${zoneHost}`}`)
}

async function loadApps(userId: string, isAdmin: boolean, zoneHost: string) {
  const list = await required(fetchAppList())
  const entries: AppAccessEntry[] = []
  const publicApps: PublicApp[] = []
  await Promise.all(
    list.apps.map(async (app) => {
      const isOwn = app.owner_user_id === userId
      const entry: AppAccessEntry = {
        appInstanceId: app.app_instance_id,
        name: appName(app),
        iconUrl: app.app_icon_url,
        ownerUserId: app.owner_user_id,
        isOwn,
        runtimeType: String(app.runtime_type),
        detail: null,
      }
      if (isOwn || isAdmin) {
        entry.detail = await section(async () => {
          const [details, policy] = await Promise.all([
            required(fetchAppDetails(app.app_instance_id)),
            required(fetchAppAvailability(app.app_instance_id)),
          ])
          const { audience, sharedUsers, sharedGroups } = audienceOf(policy)
          if (audience === 'public') {
            publicApps.push({ appInstanceId: app.app_instance_id, name: entry.name, urls: appUrls(app, zoneHost) })
          }
          return { audience, sharedUsers, sharedGroups, permissions: permissionsOf(details.spec), mounts: mountsOf(details.spec) }
        })
      }
      entries.push(entry)
    }),
  )
  const order = new Map<string, number>(list.apps.map((app, index) => [app.app_instance_id, index]))
  const byListOrder = (a: { appInstanceId: string }, b: { appInstanceId: string }) =>
    (order.get(a.appInstanceId) ?? 0) - (order.get(b.appInstanceId) ?? 0)
  entries.sort(byListOrder)
  publicApps.sort(byListOrder)
  return { entries, publicApps }
}

async function loadAgents(): Promise<AgentAccessEntry[]> {
  const { agents } = await required(fetchAgentList())
  return agents.map((agent) => ({
    agentId: agent.agent_id,
    name: agent.profile?.display_name || agent.display_name || agent.name,
    ownerUserId: agent.owner_user_id,
    enabled: agent.settings.enabled,
    allowGroup: agent.settings.allow_group,
    tunnels: agent.settings.msg_tunnels.map((tunnel) => tunnel.platform),
    runtimeApp: agent.runtime?.app_instance_id ?? null,
  }))
}

async function loadHomeStation(userId: string, zoneHost: string): Promise<Section<HomeStationExposure> | null> {
  const transport = homeStationTransport()
  if (!transport) return null
  return section(async () => {
    const [prefs, page] = await Promise.all([
      transport.call<{ defaultAudience?: AudienceSpec; commentsOpen?: boolean }>('prefs.get', {}),
      transport.call<{ entries: unknown[]; nextCursor?: string | null }>('published.list', {
        reader: { kind: 'anonymous' },
        limit: PUBLISHED_PROBE_LIMIT,
      }),
    ])
    return {
      url: `https://${zoneHost}/home/${encodeURIComponent(userId)}/`,
      publicEntries: page.entries.length,
      more: Boolean(page.nextCursor),
      defaultAudience: prefs.defaultAudience?.kind ?? 'public',
      commentsOpen: prefs.commentsOpen ?? true,
    }
  })
}

async function loadPublicWorkspaces(zoneHost: string): Promise<Section<PublicWorkspace[]> | null> {
  const availability = resolveAiwsTransport()
  if (!availability.ok) return null
  const client = new AiwsClient(availability.transport)
  return section(async () => {
    const { workspaces } = unwrapAiws(await client.wsList())
    const managed = workspaces.filter((workspace) => workspace.capabilities.includes('manage'))
    const grants = await Promise.all(
      managed.map(async (workspace) => {
        const { grants: rows } = unwrapAiws(await client.wsListGrants({ workspace_id: workspace.workspace_id }))
        return rows.some((grant) => grant.subject === 'public') ? workspace : null
      }),
    )
    return grants
      .filter((workspace) => workspace !== null)
      .map((workspace) => ({
        workspaceId: workspace.workspace_id,
        title: workspace.title,
        // the workspace's own origin (AI Workspace T4-8)
        url: `${workspaceOriginOn(zoneHost, workspace.workspace_id)}/${workspaceSlug(workspace.title)}`,
      }))
  })
}

async function countContacts(accessLevel: 'block' | 'stranger'): Promise<number> {
  const contacts = await buckyos
    .getServiceRpcClient('msg-center')
    .call<unknown[] | null, Record<string, unknown>>('contact.list_contacts', {
      query: { access_level: accessLevel, limit: CONTACT_COUNT_LIMIT },
    })
  return contacts?.length ?? 0
}

export async function loadPrivacyOverview(): Promise<PrivacyOverview> {
  if (isMockRuntime()) return mockPrivacyOverview()
  const account = await fetchCurrentAccount()
  if (!account) throw new Error('not signed in')
  const userId = account.user_id
  const isAdmin = isPrivilegedUserType(account.user_type)
  const zoneHost = currentZoneHost()

  const [apps, agents, homestation, workspaces, messaging] = await Promise.all([
    section(() => loadApps(userId, isAdmin, zoneHost)),
    section(loadAgents),
    loadHomeStation(userId, zoneHost),
    loadPublicWorkspaces(zoneHost),
    section(async () => {
      const [blocked, strangers] = await Promise.all([countContacts('block'), countContacts('stranger')])
      return { blocked, strangers }
    }),
  ])

  return {
    userId,
    isAdmin,
    zoneHost,
    publicApps: apps.ok ? { ok: true, data: apps.data.publicApps } : apps,
    homestation,
    workspaces,
    messaging,
    apps: apps.ok ? { ok: true, data: apps.data.entries } : apps,
    agents,
  }
}

function mockPrivacyOverview(): PrivacyOverview {
  const zoneHost = 'alice.web3.buckyos.ai'
  const empty = new URLSearchParams(window.location.search).get('scenario') === 'empty'
  const apps: AppAccessEntry[] = [
    {
      appInstanceId: 'photo-gallery@alice',
      name: 'Photo Gallery',
      iconUrl: null,
      ownerUserId: 'alice',
      isOwn: true,
      runtimeType: 'web',
      detail: {
        ok: true,
        data: {
          audience: 'public',
          sharedUsers: [],
          sharedGroups: [],
          permissions: [{ scope: 'fs.library', actions: ['read'], required: false }],
          mounts: [{ path: '/srv/library', target: 'srv/library', access: 'read_only', kind: 'data' }],
        },
      },
    },
    {
      appInstanceId: 'code-assistant@alice',
      name: 'Code Assistant',
      iconUrl: null,
      ownerUserId: 'alice',
      isOwn: true,
      runtimeType: 'service',
      detail: {
        ok: true,
        data: {
          audience: 'selected',
          sharedUsers: ['bob'],
          sharedGroups: [],
          permissions: [
            { scope: 'kapi/aicc', actions: [], required: true },
            { scope: 'wan', actions: [], required: false },
          ],
          mounts: [{ path: '/home/alice/projects', target: 'home/alice/projects', access: 'read_write', kind: 'data' }],
        },
      },
    },
    {
      appInstanceId: 'family-wiki@bob',
      name: 'Family Wiki',
      iconUrl: null,
      ownerUserId: 'bob',
      isOwn: false,
      runtimeType: 'web',
      detail: null,
    },
  ]
  return {
    userId: 'alice',
    isAdmin: true,
    zoneHost,
    publicApps: { ok: true, data: empty ? [] : [{ appInstanceId: 'photo-gallery@alice', name: 'Photo Gallery', urls: [`https://photos.${zoneHost}`] }] },
    homestation: {
      ok: true,
      data: { url: `https://${zoneHost}/home/alice/`, publicEntries: empty ? 0 : 12, more: false, defaultAudience: 'public', commentsOpen: true },
    },
    workspaces: {
      ok: true,
      data: empty ? [] : [{ workspaceId: 'ws-trip-2026', title: '2026 家庭旅行计划', url: `https://${zoneHost}/workspace/ws-trip-2026` }],
    },
    messaging: { ok: true, data: { blocked: empty ? 0 : 2, strangers: empty ? 0 : 5 } },
    apps: { ok: true, data: empty ? [] : apps },
    agents: {
      ok: true,
      data: empty
        ? []
        : [
            { agentId: 'jarvis', name: 'Jarvis', ownerUserId: 'alice', enabled: true, allowGroup: true, tunnels: ['telegram'], runtimeApp: 'jarvis@alice' },
            { agentId: 'helper', name: 'Helper', ownerUserId: 'alice', enabled: false, allowGroup: false, tunnels: [], runtimeApp: 'helper@alice' },
          ],
    },
  }
}
