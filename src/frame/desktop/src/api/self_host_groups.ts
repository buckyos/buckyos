export type SelfHostGroupPurpose =
  | 'conversation'
  | 'notification'
  | 'collaboration'
  | 'permission_scope'
  | 'organization'
  | 'custom'

export type SelfHostGroupMemberRole = 'owner' | 'admin' | 'member' | 'guest'

export type SelfHostGroupMemberState =
  | 'invited'
  | 'pending_member_signature'
  | 'pending_admin_approval'
  | 'active'
  | 'muted'
  | 'left'
  | 'removed'
  | 'blocked'

export type SelfHostGroupMemberKind =
  | 'unknown'
  | 'single_entity'
  | 'collection_entity'

export interface SelfHostGroupMember {
  member_did: string
  member_id?: string
  display_name?: string
  avatar_url?: string
  member_kind: SelfHostGroupMemberKind
  role: SelfHostGroupMemberRole
  state: SelfHostGroupMemberState
  joined_at: string
  updated_at: string
}

export interface SelfHostGroupPolicy {
  join_policy: 'invite_only' | 'request_and_admin_approve'
  post_policy: 'all_members' | 'admin_only' | 'role_based'
  history_visibility: 'new_members_only' | 'all_history'
  nested_group_policy:
    | 'disallow'
    | 'allow_as_opaque_member'
    | 'allow_with_recursive_expansion'
  expansion_policy:
    | 'no_auto_expansion'
    | 'expand_for_delivery'
    | 'expand_for_permission_check'
    | 'expand_for_public_view'
  max_expansion_depth: number
}

export interface SelfHostGroupSummary {
  group_id: string
  group_did: string
  display_name: string
  avatar_url?: string
  description?: string
  purpose: SelfHostGroupPurpose
  host_zone_did: string
  owner_did: string
  owner_name?: string
  is_hosted_by_self: boolean
  can_message: boolean
  member_count: number
  active_member_count: number
  member_entity_ids: string[]
  members: SelfHostGroupMember[]
  policy: SelfHostGroupPolicy
  created_at: string
  updated_at: string
  did_document?: Record<string, unknown>
}

export interface SelfHostGroupsListResponse {
  total: number
  groups: SelfHostGroupSummary[]
}

export const mockSelfHostGroups: SelfHostGroupSummary[] = [
  {
    group_id: 'family-space',
    group_did: 'did:bns:family.alice',
    display_name: 'Family Space',
    description: 'Family members sharing the Alice Zone.',
    purpose: 'conversation',
    host_zone_did: 'did:zone:alice',
    owner_did: 'did:bns:alice',
    owner_name: 'Alice',
    is_hosted_by_self: true,
    can_message: true,
    member_count: 3,
    active_member_count: 3,
    member_entity_ids: ['self-001', 'lu-001', 'lu-002'],
    members: [
      {
        member_did: 'did:bns:alice',
        member_id: 'self-001',
        display_name: 'Alice',
        member_kind: 'single_entity',
        role: 'owner',
        state: 'active',
        joined_at: '2025-06-15T00:00:00Z',
        updated_at: '2026-04-04T08:00:00Z',
      },
      {
        member_did: 'did:bns:bob',
        member_id: 'lu-001',
        display_name: 'Bob',
        member_kind: 'single_entity',
        role: 'member',
        state: 'active',
        joined_at: '2025-10-01T00:00:00Z',
        updated_at: '2026-04-03T18:00:00Z',
      },
      {
        member_did: 'did:bns:carol',
        member_id: 'lu-002',
        display_name: 'Carol',
        member_kind: 'single_entity',
        role: 'member',
        state: 'active',
        joined_at: '2025-11-15T00:00:00Z',
        updated_at: '2026-03-28T12:00:00Z',
      },
    ],
    policy: {
      join_policy: 'invite_only',
      post_policy: 'all_members',
      history_visibility: 'all_history',
      nested_group_policy: 'allow_as_opaque_member',
      expansion_policy: 'no_auto_expansion',
      max_expansion_depth: 4,
    },
    created_at: '2025-06-15T00:00:00Z',
    updated_at: '2026-04-04T08:00:00Z',
    did_document: {
      '@context': 'https://www.w3.org/ns/did/v1',
      id: 'did:bns:family.alice',
      entity_kind: 'DIDCollection',
      host_zone: 'did:zone:alice',
    },
  },
  {
    group_id: 'project-alpha',
    group_did: 'did:bns:proj-alpha',
    display_name: 'Project Alpha',
    description: 'Cross-team project collaboration group.',
    purpose: 'collaboration',
    host_zone_did: 'did:zone:alice',
    owner_did: 'did:bns:alice',
    owner_name: 'Alice',
    is_hosted_by_self: true,
    can_message: true,
    member_count: 5,
    active_member_count: 5,
    member_entity_ids: ['self-001', 'ct-001', 'ct-002', 'ct-008', 'ct-005'],
    members: [],
    policy: {
      join_policy: 'request_and_admin_approve',
      post_policy: 'role_based',
      history_visibility: 'new_members_only',
      nested_group_policy: 'allow_as_opaque_member',
      expansion_policy: 'expand_for_permission_check',
      max_expansion_depth: 3,
    },
    created_at: '2025-11-01T00:00:00Z',
    updated_at: '2026-04-01T16:00:00Z',
    did_document: {
      '@context': 'https://www.w3.org/ns/did/v1',
      id: 'did:bns:proj-alpha',
      entity_kind: 'DIDCollection',
      host_zone: 'did:zone:alice',
    },
  },
  {
    group_id: 'buckyos-community',
    group_did: 'did:bns:buckyos-community',
    display_name: 'BuckyOS Community',
    description: 'Open community for BuckyOS users.',
    purpose: 'conversation',
    host_zone_did: 'did:zone:community',
    owner_did: 'did:bns:buckyos',
    owner_name: 'BuckyOS',
    is_hosted_by_self: false,
    can_message: true,
    member_count: 128,
    active_member_count: 128,
    member_entity_ids: ['self-001'],
    members: [],
    policy: {
      join_policy: 'request_and_admin_approve',
      post_policy: 'all_members',
      history_visibility: 'new_members_only',
      nested_group_policy: 'allow_as_opaque_member',
      expansion_policy: 'no_auto_expansion',
      max_expansion_depth: 2,
    },
    created_at: '2025-03-01T00:00:00Z',
    updated_at: '2026-04-03T17:00:00Z',
  },
]

export const fetchSelfHostGroupList = async (): Promise<{
  data: SelfHostGroupsListResponse | null
  error: unknown
}> => ({
  data: {
    total: mockSelfHostGroups.length,
    groups: structuredClone(mockSelfHostGroups),
  },
  error: null,
})
