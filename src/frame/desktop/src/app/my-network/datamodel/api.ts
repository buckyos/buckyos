import { buckyos } from 'buckyos'
import { mockSelfHostGroups } from '../../../api/self_host_groups.ts'
import { fetchUserDetail } from '../../../api/user_mgr.ts'
import type { UserDetail, UserTunnelBinding } from '../../../api/user_mgr.ts'
import { mockCollections, mockContacts } from '../../users-agents/mock/seed'
import { toEntityGroupEntity, toSocialAccounts } from '../../users-agents/datamodel/transforms'
import type {
  Collection,
  ContactEntity,
  MyNetworkSnapshot,
} from './types'

interface AccountInfo {
  user_id: string
}

interface ChatContactListResponse {
  items?: unknown[]
}

type GroupListByMemberResponse = unknown[]

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

function chatContactSource(accessLevel: unknown): ContactEntity['source'] {
  const normalized = String(accessLevel ?? '').toLowerCase()
  if (normalized === 'friend') return 'manual'
  if (normalized === 'temporary' || normalized === 'stranger') return 'discovered'
  return 'imported'
}

function socialAccountsFromChatBindings(value: unknown) {
  if (!Array.isArray(value)) return []
  const bindings = value.flatMap((item): UserTunnelBinding[] => {
    const record = asRecord(item)
    const platform = stringValue(record.platform)
    const accountId = stringValue(record.account_id ?? record.accountId)
    if (!platform || !accountId) return []
    return [{
      platform,
      account_id: accountId,
      display_id: stringValue(record.display_id ?? record.displayId) ?? accountId,
      tunnel_instance_id: stringValue(record.tunnel_instance_id ?? record.tunnelId),
      status: 'active',
      last_sync_at: numberValue(record.last_active_at ?? record.lastActiveAt),
      meta: asRecord(record.meta) as Record<string, string>,
    }]
  })
  return toSocialAccounts(bindings)
}

function toContactEntity(value: unknown): ContactEntity {
  const raw = asRecord(value)
  const did = firstString(raw.did, raw.contact_did, raw.id)
  const displayName = firstString(raw.name, raw.display_name, did, 'Unknown Contact') ?? 'Unknown Contact'
  const socialAccounts = socialAccountsFromChatBindings(raw.bindings)
  const createdAt = isoFromUnix(raw.created_at ?? raw.createdAt) ?? '1970-01-01T00:00:00Z'
  const updatedAt = isoFromUnix(raw.updated_at ?? raw.updatedAt)
  const isVerified = booleanValue(raw.is_verified ?? raw.isVerified) ?? false
  const tags = Array.from(new Set([
    ...stringArray(raw.groups),
    ...stringArray(raw.tags),
  ]))

  return {
    id: firstString(did, raw.contact_id, raw.id, displayName) ?? displayName,
    kind: 'contact',
    displayName,
    avatarUrl: firstString(raw.avatar, raw.avatar_url, raw.avatarUrl),
    did,
    socialAccounts,
    source: chatContactSource(raw.access_level ?? raw.accessLevel),
    sourceLabel: firstString(
      socialAccounts[0]?.displayId,
      raw.access_level,
      raw.accessLevel,
    ),
    isVerified,
    relation: isVerified ? 'mutual' : 'one-way',
    lastSyncedAt: updatedAt,
    tags,
    notes: firstString(raw.note, raw.notes),
    lastInteraction: updatedAt,
    createdAt,
  }
}

function ownerDidFromDetail(detail: UserDetail | null, fallbackUserId?: string): string | undefined {
  const detailRecord = asRecord(detail)
  const didDocument = asRecord(detail?.did_document)
  const profile = asRecord(detail?.profile)
  const localProfile = asRecord(detail?.local_profile)
  const systemContact = asRecord(asRecord(localProfile.private_extra).system_contact)
  return firstString(
    profile.did,
    localProfile.did,
    detail?.contact?.did,
    systemContact.did,
    didDocument.id,
    detailRecord.did,
    fallbackUserId ? `did:bns:${fallbackUserId}` : undefined,
  )
}

async function fetchOwnerDid(): Promise<string | undefined> {
  const accountInfo = await buckyos.getAccountInfo() as AccountInfo | null
  const selfResult = await fetchUserDetail(accountInfo?.user_id ? { userId: accountInfo.user_id } : {})
  return ownerDidFromDetail(selfResult.data, accountInfo?.user_id)
}

async function fetchChatContactList(ownerDid?: string): Promise<{
  data: ChatContactListResponse | null
  error: unknown
}> {
  try {
    const rpcClient = buckyos.getServiceRpcClient('msg-center')
    const result = await rpcClient.call('contact.list_contacts', {
      query: { limit: 100 },
      contact_mgr_owner: ownerDid,
    })
    return { data: { items: Array.isArray(result) ? result : [] }, error: null }
  } catch (error) {
    console.warn('Failed to load contacts from msg-center.', error)
    return { data: null, error }
  }
}

async function fetchGroupListByMember(memberDid?: string): Promise<{
  data: GroupListByMemberResponse | null
  error: unknown
}> {
  if (!memberDid) {
    return { data: [], error: null }
  }

  try {
    const rpcClient = buckyos.getServiceRpcClient('msg-center')
    const result = await rpcClient.call('group.list_by_member', {
      member_did: memberDid,
      host_owner: memberDid,
    })
    return { data: Array.isArray(result) ? result : [], error: null }
  } catch (error) {
    console.warn('Failed to load my-network groups from msg-center.', error)
    return { data: null, error }
  }
}

function buildCollections(
  baseCollections: Collection[],
  contacts: ContactEntity[],
  entityGroups: MyNetworkSnapshot['entityGroups'],
): Collection[] {
  const contactIds = contacts.map((contact) => contact.id)
  const joinedGroupIds = entityGroups
    .filter((group) => !group.isHostedBySelf)
    .map((group) => group.id)
  const validEntityIds = new Set([
    ...contactIds,
    ...entityGroups.map((group) => group.id),
  ])

  return baseCollections.map((collection) => {
    if (collection.id === 'col-contacts') {
      return { ...collection, entityIds: contactIds }
    }

    if (collection.id === 'col-friends') {
      return {
        ...collection,
        entityIds: contacts
          .filter((contact) => contact.relation === 'mutual' || contact.isVerified)
          .map((contact) => contact.id),
      }
    }

    if (collection.id === 'col-joined-groups') {
      return { ...collection, entityIds: joinedGroupIds }
    }

    if (collection.id === 'col-github-view') {
      return {
        ...collection,
        entityIds: contacts
          .filter((contact) =>
            contact.socialAccounts.some((account) => account.platform === 'github'),
          )
          .map((contact) => contact.id),
      }
    }

    return {
      ...collection,
      entityIds: collection.entityIds.filter((id) => validEntityIds.has(id)),
    }
  })
}

function realBaseCollections(): Collection[] {
  return structuredClone(mockCollections).filter((collection) => collection.isBuiltIn)
}

export async function fetchMyNetworkSnapshot(): Promise<MyNetworkSnapshot> {
  const ownerDid = await fetchOwnerDid()
  const [contactsResult, groupsResult] = await Promise.all([
    fetchChatContactList(ownerDid),
    fetchGroupListByMember(ownerDid),
  ])
  const contacts = (contactsResult.data?.items ?? []).map(toContactEntity)
  const entityGroups = (groupsResult.data ?? [])
    .map(toEntityGroupEntity)
    .filter((group) => !group.isHostedBySelf)

  return {
    contacts,
    entityGroups,
    collections: buildCollections(realBaseCollections(), contacts, entityGroups),
  }
}

export function createEmptyMyNetworkSnapshot(): MyNetworkSnapshot {
  const contacts: ContactEntity[] = []
  const entityGroups: MyNetworkSnapshot['entityGroups'] = []

  return {
    contacts,
    entityGroups,
    collections: buildCollections(realBaseCollections(), contacts, entityGroups),
  }
}

export function createMockMyNetworkSnapshot(): MyNetworkSnapshot {
  const contacts = structuredClone(mockContacts)
  const entityGroups = mockSelfHostGroups
    .map(toEntityGroupEntity)
    .filter((group) => !group.isHostedBySelf)

  return {
    contacts,
    entityGroups,
    collections: buildCollections(
      structuredClone(mockCollections),
      contacts,
      entityGroups,
    ),
  }
}
