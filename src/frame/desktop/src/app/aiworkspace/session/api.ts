/* The `session.*` methods (service README “Presentation sessions”), one call each over any Transport. A display or a
 * prompter passes its link token as the session token (LinkTransport); an audience link travels in `params.link`. */

import { ServiceFailure } from '../api/client'
import { randomId } from '../api/ids'
import type { Transport } from '../api/transport'
import type { Json, Result } from '../api/types'
import type { Invite, MySession, ReplayBundle, SessionEvent, SessionInfo, SessionPost, SessionStage, SessionStep } from './types'

export function unwrapResult<T>(r: Result<T> | undefined): T {
  if (r && r.ok) return r as T
  throw new ServiceFailure(r?.error ?? { code: 'UNKNOWN', detail: 'malformed response' })
}

export class SessionApi {
  readonly transport: Transport
  readonly sessionId: string
  /** The audience link this page was opened with (`at_…`), sent along until the caller has joined. */
  link: string | null

  constructor(transport: Transport, sessionId: string, link: string | null) {
    this.transport = transport
    this.sessionId = sessionId
    this.link = link
  }

  async call<T>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    const r = await this.transport.call<Result<T>>(method, { session_id: this.sessionId, ...(this.link ? { link: this.link } : {}), ...params })
    return unwrapResult(r)
  }

  join(name?: string) { return this.call<SessionInfo>('session.join', name ? { name } : {}) }
  get() { return this.call<SessionInfo>('session.get') }
  start() { return this.call<{ state: string; stage: SessionStage }>('session.start') }
  end() { return this.call<{ state: string; expires_at: string }>('session.end') }
  save() { return this.call<{ state: string; archive: Json }>('session.save') }
  remove() { return this.call<object>('session.delete') }
  setAccess(params: { access?: 'open' | 'closed'; posts?: 'open' | 'closed'; guest_interact?: boolean }) { return this.call<{ access: string; posts: string }>('session.set_access', params) }
  invite(kind: 'at' | 'dt' | 'pt', config?: Record<string, Json>, label?: string) { return this.call<{ invite: Invite }>('session.invite', { kind, ...(config ? { config } : {}), ...(label ? { label } : {}) }) }
  invites() { return this.call<{ invites: Invite[] }>('session.list_invites') }
  revokeInvite(invite_id: string) { return this.call<{ revoked: boolean }>('session.revoke_invite', { invite_id }) }
  command(kind: string, args?: Record<string, Json>, extra?: Record<string, unknown>) {
    return this.call<{ stage: SessionStage; duplicate: boolean }>('session.command', { command: { command_id: randomId('cmd'), kind, ...(args ? { args } : {}) }, ...extra })
  }
  takeover(terminal_id: string) { return this.call<{ controller: string }>('session.takeover', { terminal_id }) }
  task(op: string, params: Record<string, unknown>) { return this.call<{ task_id: string; seq: number }>('session.task', { op, command_id: randomId('task'), ...params }) }
  claim(task_id: string) { return this.call<{ response_id: string; state: string; duplicate?: boolean }>('session.claim', { task_id }) }
  respond(task_id: string, content: Json, command_id = randomId('ans')) { return this.call<{ response_id: string }>('session.respond', { task_id, content, command_id }) }
  review(response_id: string, review: Record<string, Json>) { return this.call<object>('session.review', { response_id, review }) }
  publish(task_id: string, content: Json, title?: string) { return this.call<{ result_id: string }>('session.publish', { task_id, content, command_id: randomId('pub'), ...(title ? { title } : {}) }) }
  react(params: { kind: 'feedback' | 'danmaku'; text?: string; x?: number; y?: number; target_id?: string; surface_id?: string; symbol?: string }) { return this.call<object>('session.react', { ...params, command_id: randomId('re') }) }
  events(after: number, limit = 1000, until?: number) { return this.call<{ events: SessionEvent[]; more: boolean; head: number }>('session.events', { after, limit, ...(until !== undefined ? { until_t_rel_ms: until } : {}) }) }
  watch(after: number, timeout_ms: number) { return this.call<{ events: SessionEvent[]; head: number; stage: SessionStage; timed_out: boolean }>('session.watch', { after, timeout_ms }) }
  replay() { return this.call<ReplayBundle>('session.replay') }
  notes() { return this.call<{ notes: Record<string, string>; steps: SessionStep[] }>('session.notes') }
  post(params: { kind: 'comment' | 'rating' | 'moment'; body?: string; score?: number; t_rel_ms?: number }) { return this.call<{ post_id: string }>('session.post', params) }
  posts() { return this.call<{ posts: SessionPost[]; open: boolean }>('session.list_posts') }
  deletePost(post_id: string) { return this.call<{ deleted: boolean }>('session.delete_post', { post_id }) }
  rtc(params: Record<string, unknown>) { return this.call<{ sdp?: string; role?: 'publish' | 'listen' }>('session.rtc', params) }
}

/** Calls that name no session: creating one, a Workspace's sessions, a courseware's sample feed, a media address. */
export async function createSession(transport: Transport, params: { workspace_id: string; path_id: string; title?: string; presenter?: string; policy?: Record<string, Json> }): Promise<{ session: SessionInfo }> {
  return unwrapResult(await transport.call<Result<{ session: SessionInfo }>>('session.create', params))
}

export async function listSessions(transport: Transport, workspace_id: string): Promise<SessionInfo[]> {
  return unwrapResult(await transport.call<Result<{ sessions: SessionInfo[] }>>('session.list', { workspace_id })).sessions
}

export async function listMySessions(transport: Transport): Promise<MySession[]> {
  return unwrapResult(await transport.call<Result<{ sessions: MySession[] }>>('session.list_mine', {})).sessions
}

export async function feedSample(transport: Transport, workspace_id: string, feed: string): Promise<string> {
  return unwrapResult(await transport.call<Result<{ entity_id: string }>>('session.feed_sample', { workspace_id, feed })).entity_id
}

export async function streamUrl(transport: Transport, workspace_id: string, object_id: string): Promise<string> {
  const r = unwrapResult(await transport.call<Result<{ url: string }>>('asset.stream_url', { workspace_id, object_id }))
  return r.url
}

/** The address of a session on the Workspace's own origin (§1: the same host as the source). */
export function sessionUrl(origin: string, workspaceSlug: string, sessionId: string, link?: string | null): string {
  return `${origin.replace(/\/+$/, '')}/${encodeURIComponent(workspaceSlug || 'workspace')}?session=${encodeURIComponent(sessionId)}${link ? `#k=${encodeURIComponent(link)}` : ''}`
}

/** The link a session page was opened with (`#k=<at_|dt_|pt_…>`). */
export function linkOfHash(hash: string): string | null {
  const k = new URLSearchParams(hash.replace(/^#/, '')).get('k')
  return k && /^(at|dt|pt)_[a-z2-7]{26}$/.test(k) ? k : null
}
