/**
 * Session Only (SSO-12) guest page model: a restricted visitor (Limited
 * account or External DID) reads the host group store directly through
 * `group.*` and the object route, never the owner mailbox projection.
 */
import type { AuthMethod, EntryMethods } from 'buckyos'
import type { GroupSessionItem } from '../datamodel/sessionApi'
import { groupSessionKey } from '../groupModel'
import type { MessageObject, MsgObject } from '../protocol/msgobj'

export const GUEST_PAGE_PATH = '/messagehub/guest'

/** The guest page of a group, opened on one of its Sessions. */
export function guestPagePath(groupDid: string, sessionId?: string | null): string {
  const query = new URLSearchParams({ group: groupDid })
  if (sessionId) query.set('session', sessionId)
  return `${GUEST_PAGE_PATH}?${query}`
}

/** Where a visitor without a session signs in for this group: its Message Hub entry (contract §8.4). */
export function guestEntryPath(groupDid: string): string {
  return `/sso/entry?${new URLSearchParams({ service: 'msg-center', entry: groupDid })}`
}

export function isGroupDid(value: string | null | undefined): value is string {
  return !!value && /^did:[a-z0-9]+:[^\s/?#]+$/i.test(value)
}

/** One Session of the group the visitor may read (`group.list_sessions`; the default session is never a guest's). */
export interface GuestSession {
  sessionId: string
  /** Canonical MailboxAddress (`<group>/<encoded session id>`). */
  address: string
  title: string
  lifecycle: string
}

export function guestSessions(groupDid: string, items: readonly GroupSessionItem[]): GuestSession[] {
  return items.flatMap(item => item.session_id === null ? [] : [{
    sessionId: item.session_id,
    address: groupSessionKey(groupDid, item.session_id),
    title: typeof item.shared_state?.title === 'string' ? item.shared_state.title : '',
    lifecycle: item.lifecycle,
  }])
}

/**
 * Whether the group's public entry takes a guest request from a session that
 * signed in by `authMethod` (the host's `EntryMethods::allows_session`; the
 * host decides, this only hides a button that would be refused).
 */
export function guestEntryAllows(methods: EntryMethods | null, authMethod: AuthMethod | undefined): boolean {
  if (!methods) return false
  return authMethod === 'did_federation' ? methods.did_login : methods.password || methods.self_register
}

/** `context_path` of the objects a Session message carries (msg-center `authorize_group_attachment`). */
export function objectContextPath(sessionAddress: string, msgId: string): string {
  return `${sessionAddress}/${msgId}`
}

/** The named object an object-route answer carries: plain JSON, or the payload of its signed JWT form. */
export function decodeNamedObject(contentType: string, body: string): unknown {
  if (!contentType.includes('jwt')) return JSON.parse(body)
  const payload = body.trim().split('.')[1]
  if (!payload) throw new Error('malformed named object jwt')
  const base64 = payload.replace(/-/g, '+').replace(/_/g, '/').padEnd(Math.ceil(payload.length / 4) * 4, '=')
  const bytes = Uint8Array.from(atob(base64), char => char.charCodeAt(0))
  return JSON.parse(new TextDecoder().decode(bytes))
}

/** `group.list_messages` item: the host's order (`seq`) and the message ObjId. */
export interface GuestMessageRef {
  seq: number
  objId: string
  redacted?: boolean
}

/**
 * A stored Session message as the guest timeline shows it. It names its
 * Session (`ui_group_session`) so its attachments are read with a
 * `context_path`; the Agent's task belongs to the host and is dropped.
 */
export function guestTimelineMessage(msg: MsgObject, ref: GuestMessageRef, sessionAddress: string): MessageObject {
  const message: MessageObject = { ...msg, ui_message_id: ref.objId, ui_session_id: sessionAddress, ui_group_session: sessionAddress, ui_seq: ref.seq }
  delete message.agent_task
  return message
}

/** Stored messages in host order, then own sends the host has not listed yet; `nameOf` labels the senders. */
export function guestTimeline(stored: Iterable<MessageObject>, pending: Iterable<MessageObject>, nameOf: (did: string) => string | undefined): MessageObject[] {
  const ordered = [...stored].sort((a, b) => Number(a.ui_seq) - Number(b.ui_seq))
  const storedIds = new Set(ordered.map(message => message.ui_message_id))
  const waiting = [...pending].filter(message => !storedIds.has(message.ui_sent_msg_id)).sort((a, b) => a.created_at_ms - b.created_at_ms)
  return [...ordered, ...waiting].map(message => {
    const name = nameOf(message.from)
    return name ? { ...message, ui_sender_name: name } : message
  })
}

/** An account id suggested from a customer's name (`[a-z0-9_.-]`); empty when nothing of the name fits. */
export function suggestCustomerUserId(name: string): string {
  return name.normalize('NFKD').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 64)
}

export function isCustomerUserId(value: string): boolean {
  return /^[a-z0-9][a-z0-9_.-]{0,63}$/.test(value)
}
