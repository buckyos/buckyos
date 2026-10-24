/**
 * Data access of the guest page: the host group store through `group.*`, the
 * message objects through the object route with a `context_path`, group
 * messages through `msg.post_send`. Nothing here reads an owner mailbox,
 * `owner_sessions`, contacts or notifications, which restricted sessions
 * cannot call (D-4). Failures keep their kRPC error: callers branch on
 * `denialOf`, never on the text.
 */
import type { CurrentIdentity, EntryMethods } from 'buckyos'
import { loadCurrentIdentity } from '../../../sso/identity'
import { endSessionInPlace } from '../../../auth/signOut'
import { isMessageHubMock } from '../store'
import { describeEntry } from '../../../api/sso'
import type { ObjectAccess } from '../conversation/history/objectAccess'
import { acceptGroupSessionInvitation, getGroupDoc, listGroupsByMember, listGroupMessages, listGroupSessionMembers, listGroupSessions, postSendMessage, submitGuestRequest, updateGroupReadMarker, type GroupSessionMemberRecord } from '../datamodel/sessionApi'
import { apiObjectAccess, authorizedFetch, objectUrl } from '../api/objects'
import type { MsgObject } from '../protocol/msgobj'
import { decodeNamedObject, guestSessions, objectContextPath, type GuestMessageRef, type GuestSession } from './model'

export interface GuestMessagePage {
  items: GuestMessageRef[]
  nextAfterSeq: number
  /** More messages follow `nextAfterSeq`. */
  limited: boolean
}

export interface GuestApi {
  identity(): Promise<CurrentIdentity>
  groupName(groupDid: string): Promise<string>
  /** The group Doc's `host`: the Zone whose Apps may serve the Session's panels (D-2). */
  groupHost(groupDid: string): Promise<string>
  /** The methods the group's public entry offers after the Zone switches (`auth.entry.describe`); null when it has none open. */
  entry(groupDid: string): Promise<EntryMethods | null>
  sessions(groupDid: string): Promise<GuestSession[]>
  /** Whether the visitor holds a pending invitation into the Session (`group.list_by_member`). */
  invited(groupDid: string, sessionId: string): Promise<boolean>
  members(groupDid: string, sessionId: string): Promise<GroupSessionMemberRecord[]>
  messages(groupDid: string, sessionId: string, afterSeq: number, limit: number): Promise<GuestMessagePage>
  /** One stored message of the Session at `sessionAddress`. */
  message(sessionAddress: string, objId: string): Promise<MsgObject>
  accept(groupDid: string, sessionId: string, idempotencyKey: string): Promise<void>
  /** `group.submit_guest_request`; resolves to the new Session id. */
  request(groupDid: string, requestId: string): Promise<string>
  send(message: MsgObject, idempotencyKey: string): Promise<{ ok: boolean; msgId: string; reason?: string }>
  markRead(groupDid: string, sessionId: string, seq: number): Promise<void>
  readonly objects: ObjectAccess
  /** Ends the browser session and drops what this page holds. */
  signOut(): Promise<void>
}

const text = (value: unknown) => typeof value === 'string' ? value.trim() : ''

export const msgCenterGuestApi: GuestApi = {
  identity: loadCurrentIdentity,
  async groupName(groupDid) {
    return text((await getGroupDoc(groupDid)).doc.profile?.name)
  },
  async groupHost(groupDid) {
    return text((await getGroupDoc(groupDid)).doc.host)
  },
  async entry(groupDid) {
    // The Control Panel applies the Zone's switches to the entry's own methods.
    const { data, error } = await describeEntry('msg-center', groupDid)
    if (error) throw error
    return data?.state === 'active' ? data.methods : null
  },
  async sessions(groupDid) {
    return guestSessions(groupDid, await listGroupSessions(groupDid))
  },
  async invited(groupDid, sessionId) {
    const listed = await listGroupsByMember()
    return (listed.session_invitations ?? []).some(item => item.group_did === groupDid && item.session_id === sessionId)
  },
  async members(groupDid, sessionId) {
    return (await listGroupSessionMembers(groupDid, sessionId)).items
  },
  async messages(groupDid, sessionId, afterSeq, limit) {
    const page = await listGroupMessages(groupDid, sessionId, afterSeq, limit)
    const items = (page.items ?? []).map(item => ({ seq: item.seq, objId: item.obj_id, ...(item.redacted ? { redacted: true } : {}) }))
    return { items, nextAfterSeq: page.next_after_seq ?? items.at(-1)?.seq ?? afterSeq, limited: page.limited === true }
  },
  async message(sessionAddress, objId) {
    const response = await authorizedFetch(objectUrl(objId, false, objectContextPath(sessionAddress, objId)))
    return decodeNamedObject(response.headers.get('content-type') ?? '', await response.text()) as MsgObject
  },
  async accept(groupDid, sessionId, idempotencyKey) {
    await acceptGroupSessionInvitation(groupDid, sessionId, idempotencyKey)
  },
  async request(groupDid, requestId) {
    return (await submitGuestRequest(groupDid, requestId)).session_id
  },
  async send(message, idempotencyKey) {
    const result = await postSendMessage(message, idempotencyKey)
    return { ok: result.ok, msgId: result.msg_id, ...(result.reason ? { reason: result.reason } : {}) }
  },
  async markRead(groupDid, sessionId, seq) {
    await updateGroupReadMarker(groupDid, sessionId, seq)
  },
  objects: apiObjectAccess,
  signOut: endSessionInPlace,
}

/** The msg-center backed access, or the interactive mock host (`mock/guest.ts`). */
export async function loadGuestApi(): Promise<GuestApi> {
  if (!isMessageHubMock()) return msgCenterGuestApi
  const { mockGuestApi } = await import('../mock/guest')
  return mockGuestApi()
}
