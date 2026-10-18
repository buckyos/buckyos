/* ── msg-center relationship surface: websdk MsgCenterClient for `contact.*`, kRPC for `group.*` / `msg.*` ── */

import { buckyos } from 'buckyos'
import type { MsgCenterClient } from 'buckyos'
import { fetchUserDetail } from '../../api/user_mgr'
import { GROUP_INVITATION_INTENT } from './relation'
import type {
  GroupDocInfo,
  GroupInvitationItem,
  GroupRef,
  GroupRole,
  JoinRequestItem,
  NetworkBackend,
  RequestSession,
} from './types'

const MSG_CENTER_SERVICE = 'msg-center'
const SESSION_PAGE = 100
const SESSION_PAGES = 5
const BY_MEMBER_TTL_MS = 1_500

interface GroupDocWire {
  id: string
  owner?: string
  host?: string
  profile?: { name?: unknown; description?: unknown; avatar?: unknown } | null
  join_policy?: string
  lifecycle?: string
}

interface GroupMemberWire {
  member_did: string
  role: GroupRole
  state: string
  entity_kind?: string
}

interface MsgWire {
  content?: { content?: unknown; machine?: { intent?: unknown; data?: Record<string, unknown> } }
}

interface SessionSummaryWire {
  session_id: string
  request_count?: number
  last_activity_ms?: number
  updated_at_ms?: number
  state?: { peer_did?: string; origin?: string } | null
  last_record?: { record: { from: string; from_name?: string; created_at_ms?: number }; msg?: MsgWire | null } | null
}

interface SessionItemWire {
  record_id: string
  recipient_state?: string
  msg?: MsgWire | null
}

interface ByMemberItem {
  doc?: unknown
  member?: { state?: string; role?: GroupRole | null; invitation_id?: string | null; invited_by?: string | null; expires_at_ms?: number | null } | null
}

interface ByMemberResult {
  items?: ByMemberItem[]
  invitations?: ByMemberItem[]
  joined?: Array<{ group_did: string; doc_cache?: unknown }>
}

const text = (value: unknown) => (typeof value === 'string' && value.trim() ? value.trim() : undefined)

function rpc() {
  return buckyos.getServiceRpcClient(MSG_CENTER_SERVICE)
}

function call<T>(method: string, params: Record<string, unknown>): Promise<T> {
  return rpc().call<T, Record<string, unknown>>(method, params)
}

/** `did:<method>:<id>` → the raw host name msg-center uses in its kevent paths. */
function ownerEventToken(did: string): string {
  const parts = did.split(':')
  const method = parts[1] ?? ''
  const id = parts.slice(2).join(':').split(':')[0] ?? ''
  return method === 'web' ? id : `${id}.${method}.did`
}

function docOf(value: unknown): GroupDocWire | null {
  if (!value || typeof value !== 'object') return null
  const record = value as Record<string, unknown>
  const doc = (record.doc && typeof record.doc === 'object' ? record.doc : record) as GroupDocWire
  return typeof doc.id === 'string' ? doc : null
}

function docInfo(doc: GroupDocWire): GroupDocInfo {
  return {
    did: doc.id,
    name: text(doc.profile?.name),
    description: text(doc.profile?.description),
    owner: doc.owner,
    host: doc.host,
    joinPolicy: doc.join_policy,
  }
}

function requestSession(summary: SessionSummaryWire): RequestSession | null {
  const peerDid = summary.state?.peer_did ?? (summary.session_id.startsWith('dm:') ? summary.session_id.slice(3) : undefined)
  if (!peerDid || summary.state?.origin === 'group') return null
  const record = summary.last_record
  return {
    sessionId: summary.session_id,
    peerDid,
    peerName: record?.record.from === peerDid ? text(record.record.from_name) : undefined,
    count: summary.request_count ?? 0,
    lastAtMs: summary.last_activity_ms ?? summary.updated_at_ms ?? 0,
    intent: text(record?.msg?.content?.machine?.intent),
    preview: text(record?.msg?.content?.content),
  }
}

export function createRealNetworkBackend(): NetworkBackend {
  let owner: Promise<string> | null = null
  const client = (): MsgCenterClient => buckyos.getMsgCenterClient()

  const ownerDid = () => {
    owner ??= (async () => {
      const account = await buckyos.getAccountInfo()
      const userId = account?.user_id
      if (!userId) throw new Error('forbidden: not signed in')
      const { data, error } = await fetchUserDetail({ userId })
      if (error) throw error instanceof Error ? error : new Error(String(error))
      const did = data?.local_profile?.did ?? data?.profile?.did
      if (typeof did !== 'string' || !did.startsWith('did:')) throw new Error('forbidden: user profile has no owner DID')
      return did
    })().catch((error) => {
      owner = null
      throw error
    })
    return owner
  }

  let byMember: { at: number; promise: Promise<ByMemberResult> } | null = null
  const listByMember = () => {
    if (!byMember || Date.now() - byMember.at > BY_MEMBER_TTL_MS) {
      const promise = call<ByMemberResult | null>('group.list_by_member', {}).then(result => result ?? {})
      promise.catch(() => { if (byMember?.promise === promise) byMember = null })
      byMember = { at: Date.now(), promise }
    }
    return byMember.promise
  }
  const listMembers = (groupDid: string) =>
    call<{ items?: GroupMemberWire[] } | null>('group.list_members', { group_did: groupDid }).then(result => result?.items ?? []).catch(() => [])
  /** Invitations whose notice could not be found are only hidden for this page. */
  const hiddenInvitations = new Set<string>()
  const invitationNotice = async (owner: string, invitation: GroupInvitationItem) => {
    const page = await call<{ items?: SessionItemWire[] } | null>('msg.list_session', { owner, session_id: `dm:${invitation.inviterDid}`, limit: 50, descending: true, with_object: true }).catch(() => null)
    return page?.items?.find((item) => {
      const machine = item.msg?.content?.machine
      const detail = machine?.data?.data as Record<string, unknown> | undefined
      return machine?.intent === GROUP_INVITATION_INTENT && machine.data?.action === 'invite' && detail?.invite_id === invitation.inviteId
    })
  }

  const postAccess = (groupDid: string) =>
    call<{ allowed: boolean; reason?: string | null }>('group.check_access', { group_did: groupDid, action: 'session.post' }).catch(() => null)

  const idempotencyKey = () => crypto.randomUUID()

  const backend: NetworkBackend = {
    isMock: false,
    ownerDid,
    shareText: ownerDid,
    lookup: async (input, options) => client().lookup({ input, ...options, contact_mgr_owner: await ownerDid() }),
    listContacts: async () => client().listContacts({ limit: 1000 }, await ownerDid()),
    saveContact: async (did, name) => {
      const me = await ownerDid()
      const existing = await client().getContact(did, me)
      return existing ?? client().updateContact(did, { access_level: 'stranger', ...(name ? { name } : {}) }, me)
    },
    updateTags: async (did, change) => client().updateContactTags(did, change, await ownerDid()),
    removeContact: async (did) => client().removeContact(did, await ownerDid()),
    blockContact: async (did) => client().blockContact(did, undefined, await ownerDid()),
    unblockContact: async (did) => client().unblockContact(did, await ownerDid()),
    allowMessages: async (did, name) => client().updateContact(did, { access_level: 'allow', ...(name ? { name } : {}) }, await ownerDid()),
    sendFriendRequest: async (did, message) => client().sendFriendRequest(did, message || undefined, await ownerDid()),
    respondFriendRequest: async (requestId, decision) => client().respondFriendRequest(requestId, decision, await ownerDid()),
    cancelFriendRequest: async (requestId) => client().cancelFriendRequest(requestId, await ownerDid()),
    removeFriend: async (did, notify) => client().removeFriend(did, notify, await ownerDid()),
    listFriendRequests: async () => client().listFriendRequests({ limit: 200, contact_mgr_owner: await ownerDid() }),
    listRequestSessions: async () => {
      const owner = await ownerDid()
      const sessions: RequestSession[] = []
      let cursor: { cursor_updated_at_ms?: number; cursor_session_id?: string } = {}
      for (let page = 0; page < SESSION_PAGES; page += 1) {
        const result = await call<{ items?: SessionSummaryWire[]; next_cursor_updated_at_ms?: number; next_cursor_session_id?: string } | null>('msg.list_sessions', { owner, limit: SESSION_PAGE, with_object: true, lifecycle: 'active', ...cursor })
        for (const summary of result?.items ?? []) {
          const session = (summary.request_count ?? 0) > 0 ? requestSession(summary) : null
          if (session) sessions.push(session)
        }
        if (!result?.next_cursor_session_id) break
        cursor = { cursor_updated_at_ms: result.next_cursor_updated_at_ms, cursor_session_id: result.next_cursor_session_id }
      }
      return sessions
    },
    listGroups: async () => {
      const result = await listByMember()
      const hosted = (result.items ?? []).flatMap(item => {
        const doc = docOf(item.doc)
        return doc ? [{ doc, role: item.member?.role ?? 'member' }] : []
      })
      const refs = await Promise.all(hosted.map(async ({ doc, role }): Promise<GroupRef> => {
        const access = await postAccess(doc.id)
        return {
          ...docInfo(doc),
          avatar: text(doc.profile?.avatar),
          role,
          state: 'active',
          hostedHere: true,
          canPost: access ? access.allowed : null,
          postBlockedReason: access?.reason ?? undefined,
        }
      }))
      const waiting = (result.invitations ?? []).flatMap((item): GroupRef[] => {
        const doc = docOf(item.doc)
        return doc && item.member?.state === 'pending_admin_approval'
          ? [{ ...docInfo(doc), state: 'pending_admin_approval', hostedHere: true, canPost: false }]
          : []
      })
      const remote = (result.joined ?? [])
        .filter(item => !hosted.some(({ doc }) => doc.id === item.group_did))
        .map((item): GroupRef => {
          const doc = docOf(item.doc_cache)
          return { ...(doc ? docInfo(doc) : {}), did: item.group_did, state: 'active', role: 'member', hostedHere: false, canPost: null }
        })
      return [...refs, ...waiting, ...remote]
    },
    getGroupDoc: async (groupDid) => {
      const doc = docOf(await call<unknown>('group.get_doc', { group_did: groupDid }))
      if (!doc) throw new Error('not-found')
      return docInfo(doc)
    },
    requestJoin: async (groupDid, invite) => {
      const record = await call<GroupMemberWire>('group.request_join', { group_did: groupDid, ...(invite ? { invite } : {}) })
      byMember = null
      return record.state === 'pending_admin_approval' ? 'pending_admin_approval' : 'active'
    },
    leaveGroup: async (groupDid) => {
      await call<unknown>('group.leave', { group_did: groupDid, idempotency_key: idempotencyKey() })
      byMember = null
    },
    listGroupInvitations: async () => {
      const owner = await ownerDid()
      const result = await listByMember()
      const invitations = (result.invitations ?? []).flatMap((item): GroupInvitationItem[] => {
        const doc = docOf(item.doc)
        const member = item.member
        if (!doc || member?.state !== 'invited' || !member.invitation_id || !member.invited_by) return []
        return [{
          groupDid: doc.id,
          groupName: text(doc.profile?.name),
          inviteId: member.invitation_id,
          inviterDid: member.invited_by,
          role: 'member',
          ...(member.expires_at_ms ? { expiresAtMs: member.expires_at_ms } : {}),
        }]
      })
      const withNotices = await Promise.all(invitations.filter(item => !hiddenInvitations.has(item.inviteId)).map(async (item) => {
        const notice = await invitationNotice(owner, item)
        if (notice?.recipient_state === 'ARCHIVED' || notice?.recipient_state === 'DELETED') return null
        return notice ? { ...item, recordId: notice.record_id } : item
      }))
      return withNotices.filter((item): item is GroupInvitationItem => Boolean(item))
    },
    acceptGroupInvitation: async (invitation) => {
      const record = await call<GroupMemberWire>('group.accept_invitation', { group_did: invitation.groupDid, invitation_id: invitation.inviteId })
      byMember = null
      return record.state === 'pending_admin_approval' ? 'pending_admin_approval' : 'active'
    },
    dismissGroupInvitation: async (invitation) => {
      hiddenInvitations.add(invitation.inviteId)
      if (invitation.recordId) await call<unknown>('msg.update_record_state', { record_id: invitation.recordId, new_state: 'ARCHIVED' })
    },
    listJoinRequests: async () => {
      const result = await listByMember()
      const managed = (result.items ?? []).flatMap(item => {
        const doc = docOf(item.doc)
        return doc && (item.member?.role === 'owner' || item.member?.role === 'admin') ? [docInfo(doc)] : []
      })
      const lists = await Promise.all(managed.map(async group => (await listMembers(group.did))
        .filter(item => item.state === 'pending_admin_approval')
        .map((item): JoinRequestItem => ({ groupDid: group.did, groupName: group.name, memberDid: item.member_did, entityKind: item.entity_kind }))))
      return lists.flat()
    },
    approveJoinRequest: async (request) => {
      await call<unknown>('group.approve_member', { group_did: request.groupDid, member_did: request.memberDid, idempotency_key: idempotencyKey() })
      byMember = null
    },
    rejectJoinRequest: async (request) => { await call<unknown>('group.reject_member', { group_did: request.groupDid, member_did: request.memberDid, idempotency_key: idempotencyKey() }) },
    subscribe: (listener) => {
      let stopped = false
      let handle: { close(): Promise<void> } | null = null
      const online = () => { listener('contacts'); listener('requests'); listener('mailbox'); listener('groups') }
      window.addEventListener('online', online)
      void ownerDid().then((did) => {
        const token = ownerEventToken(did)
        const contacts = `/msg_center/${token}/CONTACTS/changed`
        return buckyos.subscribeKEvent([contacts, `/msg_center/${token}/REQUEST_BOX/**`, `/msg_center/${token}/INBOX/**`], (event) => {
          if (stopped) return
          byMember = null
          const id = String(event.eventid ?? '')
          if (id.startsWith(contacts)) {
            listener('contacts')
            listener('requests')
            return
          }
          listener('mailbox')
          if (id.includes('/INBOX/')) listener('groups')
        })
      }).then((subscription) => {
        if (stopped) void subscription.close()
        else handle = subscription
      }).catch(() => { /* reloads on reconnect and on each action remain */ })
      return () => {
        stopped = true
        window.removeEventListener('online', online)
        if (handle) void handle.close()
      }
    },
  }
  return backend
}

