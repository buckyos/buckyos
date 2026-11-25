import type { ZoneChannel } from '../channel.ts'
import { callControlPanel } from './call.ts'

interface UserIdentity {
  local_profile?: unknown
  profile?: unknown
}

/** The account's own DID in a `user.get` answer: its message mailbox belongs to it. */
export function accountDidOf(user: UserIdentity | null | undefined): string | null {
  const didOf = (profile: unknown) => (profile && typeof profile === 'object' ? (profile as { did?: unknown }).did : undefined)
  const did = didOf(user?.local_profile) ?? didOf(user?.profile)
  return typeof did === 'string' && did.startsWith('did:') ? did : null
}

export async function readAccountDid(channel: ZoneChannel, userId: string): Promise<string | null> {
  return accountDidOf(await callControlPanel<UserIdentity>(channel, 'user.get', { user_id: userId }))
}

/** The DIDs of the account's Agents (`agent.list`). */
export async function readAgentDids(channel: ZoneChannel): Promise<string[]> {
  const { agents } = await callControlPanel<{ agents?: Array<{ agent_did?: unknown }> }>(channel, 'agent.list')
  return agentDidsOf(agents ?? [])
}

export function agentDidsOf(agents: readonly { agent_did?: unknown }[]): string[] {
  return agents.map(agent => agent.agent_did).filter((did): did is string => typeof did === 'string' && did.startsWith('did:'))
}
