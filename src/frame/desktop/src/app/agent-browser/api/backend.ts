/* What the App needs from an Agent's service: the list of the user's own Agents, kRPC `browser.*`, the screen
 * socket and the upload of a local file. The Zone implementation and the mock Agent Service (mock/backend.ts) are
 * the two implementations; everything above this file is the same code in both. */

import { buckyos } from 'buckyos'
import { fetchCurrentAccount } from '../../../api/account'
import { fetchAgentList, type AgentEntry } from '../../../api/user_mgr'
import { isMockRuntime } from '../../../runtime'
import { agentDisplayName, usableImageUrl } from '../../agent-setup/model'
import { agentServiceClient } from '../../../components/conversation/worklog/source'
import type { ScreenSocket } from './channel'

export interface BrowserAgent {
  agentId: string
  agentDid: string
  name: string
  avatar?: string
  /** A read-only Agent cannot write the owner's buckets, so it cannot maintain the personal site. */
  readOnly: boolean
}

export interface BrowserBackend {
  /** The signed-in user's own Agents that are ready (D10: a browser always belongs to an Agent). */
  agents(): Promise<BrowserAgent[]>
  call<T>(agent: BrowserAgent, method: string, params?: Record<string, unknown>): Promise<T>
  /** The session token the screen's `hello` and the upload carry. */
  token(agent: BrowserAgent): Promise<string>
  openScreen(agent: BrowserAgent): ScreenSocket
  /** `PUT /kapi/<agent_id>/browser/upload/<request_id>` (protocol §4). */
  upload(agent: BrowserAgent, requestId: string, file: File, done: boolean): Promise<void>
}

export function toBrowserAgent(entry: AgentEntry): BrowserAgent {
  const avatar = usableImageUrl(entry.profile?.avatar)
  return { agentId: entry.agent_id, agentDid: entry.agent_did, name: agentDisplayName(entry), readOnly: entry.permission_mode === 'read_only', ...(avatar ? { avatar } : {}) }
}

export async function ownReadyAgents(): Promise<AgentEntry[]> {
  const [account, list] = await Promise.all([fetchCurrentAccount(), fetchAgentList()])
  if (list.error || !list.data) throw list.error ?? Error('Agent directory unavailable')
  return list.data.agents.filter((agent) => agent.owner_user_id === account?.user_id && agent.install.state === 'ready')
}

function servicePath(agent: BrowserAgent): string {
  return buckyos.getZoneServiceURL(agent.agentId).replace(/\/+$/, '')
}

class UploadError extends Error {
  readonly code: string
  constructor(code: string, message: string) {
    super(message)
    this.code = code
  }
}

const zoneBackend: BrowserBackend = {
  async agents() {
    return (await ownReadyAgents()).map(toBrowserAgent)
  },
  async call<T>(agent: BrowserAgent, method: string, params: Record<string, unknown> = {}) {
    return (await agentServiceClient(agent.agentDid)).call<T, Record<string, unknown>>(method, params)
  },
  async token(agent) {
    const fromClient = (await agentServiceClient(agent.agentDid)).getSessionToken()
    if (fromClient) return fromClient
    return (await buckyos.getAccountInfo().catch(() => null))?.session_token ?? ''
  },
  openScreen(agent) {
    const url = new URL(`${servicePath(agent)}/browser/screen`, window.location.href)
    return new WebSocket(`${url.protocol === 'https:' ? 'wss:' : 'ws:'}//${url.host}${url.pathname}`)
  },
  async upload(agent, requestId, file, done) {
    const query = new URLSearchParams({ name: file.name })
    if (done) query.set('done', '1')
    const response = await fetch(`${servicePath(agent)}/browser/upload/${encodeURIComponent(requestId)}?${query}`, {
      method: 'PUT',
      headers: { Authorization: `Bearer ${await this.token(agent)}` },
      body: file,
    })
    if (response.ok) return
    const wire = await response.json().catch(() => null) as { code?: unknown; message?: unknown } | null
    const code = typeof wire?.code === 'string' ? wire.code : response.status === 413 ? 'upload_too_large' : 'upload_failed'
    throw new UploadError(code, typeof wire?.message === 'string' ? wire.message : `upload failed: ${response.status}`)
  },
}

let backend: Promise<BrowserBackend> | null = null

export function getBrowserBackend(): Promise<BrowserBackend> {
  backend ??= isMockRuntime() ? import('../mock/backend').then((module) => module.mockBackend) : Promise.resolve(zoneBackend)
  return backend
}
