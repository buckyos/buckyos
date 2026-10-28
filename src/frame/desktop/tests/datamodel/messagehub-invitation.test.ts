/// <reference types="vite/client" />
import { buckyos } from 'buckyos'
import { MessageHubApiStore } from '../../src/app/messagehub/api/store.ts'
import { groupErrorReason, MessageHubApiError } from '../../src/app/messagehub/datamodel/sessionApi.ts'
import { groupErrorText } from '../../src/app/messagehub/groupModel.ts'
import type { GroupInvitation } from '../../src/app/messagehub/types.ts'
import { createRealNetworkBackend } from '../../src/components/network/realBackend.ts'
import { networkErrorText } from '../../src/components/network/relation.ts'

const owner = 'did:bns:me'
const group = 'did:web:remote.test:groups:team'
const invitation: GroupInvitation = { groupDid: group, inviteId: 'invite-1', inviterDid: 'did:bns:alice', role: 'member', state: 'invited' }
const networkInvitation = { groupDid: group, inviteId: invitation.inviteId, inviterDid: invitation.inviterDid, role: invitation.role }
const doc = { id: group, host: 'did:web:remote.test', owner: invitation.inviterDid, controller: invitation.inviterDid, lifecycle: 'active', revision: '1', profile: { name: 'Remote team' } }

function equal(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`)
}

async function rejects(action: () => Promise<unknown>, reason: string) {
  try { await action() } catch (error) { equal((error as Error).message, reason); return }
  throw Error(`Expected ${reason}`)
}

async function fixture(run: (store: MessageHubApiStore, backend: {
  state: 'active' | 'pending_admin_approval'
  syncPending: boolean
  accepted: boolean
  listFailure: boolean
  sessionFailure: boolean
  omitGroup: boolean
  responseFailure: string | null
  calls: Record<string, unknown>[]
}) => Promise<void>) {
  const backend = { state: 'active' as 'active' | 'pending_admin_approval', syncPending: false, accepted: false, listFailure: false, sessionFailure: false, omitGroup: false, responseFailure: null as string | null, calls: [] as Record<string, unknown>[] }
  const saved = { getCurrentIdentity: buckyos.getCurrentIdentity, getAccountInfo: buckyos.getAccountInfo, getServiceRpcClient: buckyos.getServiceRpcClient }
  buckyos.getCurrentIdentity = async () => ({ kind: 'anonymous' })
  buckyos.getAccountInfo = async () => ({ user_id: 'me', user_name: 'Me', session_token: 'test-only' })
  buckyos.getServiceRpcClient = (() => ({
    call: async (method: string, input: Record<string, unknown>) => {
      if (method === 'user.get') return { local_profile: { did: owner } }
      if (method === 'agent.list') return { agents: [] }
      if (method === 'contact.list_contacts') return []
      if (method === 'group.get_doc') return { obj_id: 'doc', doc }
      if (method === 'group.accept_invitation') {
        backend.calls.push(input)
        backend.accepted = true
        if (backend.responseFailure) throw Error(backend.responseFailure)
        return { member_did: input.member_did ?? owner, state: backend.state, role: 'member', entity_kind: input.member_did ? 'agent' : 'user', invitation_id: input.invitation_id, sync_pending: backend.syncPending }
      }
      if (method === 'group.list_by_member') {
        if (backend.listFailure) throw Error('failed to fetch')
        if (backend.accepted && backend.state === 'pending_admin_approval') return { invitations: [{ group_did: group, hosted_here: false, member: { member_did: owner, invitation_id: invitation.inviteId, state: backend.state } }] }
        return backend.accepted && backend.state === 'active' && !backend.omitGroup ? { joined: [{ group_did: group, doc_cache: doc }] } : {}
      }
      if (method === 'msg.list_sessions') {
        if (backend.sessionFailure) throw Error('failed to fetch')
        return { items: [] }
      }
      throw Error(`Unexpected RPC ${method}`)
    },
  })) as unknown as typeof buckyos.getServiceRpcClient
  try {
    const store = new MessageHubApiStore()
    await store.initialize()
    await run(store, backend)
  } finally {
    Object.assign(buckyos, saved)
  }
}

Deno.test('an accepted remote invitation uses the returned membership and refreshed local directory', async () => {
  await fixture(async (store) => {
    const context = store.defaultContext()
    await store.acceptGroupInvitation(context, invitation)
    equal(store.groupInvitation(context, invitation), { groupName: 'Remote team', state: 'joined' })
    await store.ensureGroup(context, group)
    equal(store.group(context, group)?.hosted, false)
  })
  await fixture(async (store, backend) => {
    backend.state = 'pending_admin_approval'
    backend.syncPending = true
    const context = store.defaultContext()
    await store.acceptGroupInvitation(context, invitation)
    equal(store.groupInvitation(context, invitation).state, 'approval')
    const reloaded = new MessageHubApiStore()
    await reloaded.initialize()
    equal(reloaded.groupInvitation(reloaded.defaultContext(), invitation).state, 'approval')
  })
})

Deno.test('accepted invitations stay retryable while backend sync is pending and reuse the invitation identity', async () => {
  await fixture(async (store, backend) => {
    const context = store.defaultContext()
    backend.syncPending = true
    await rejects(() => store.acceptGroupInvitation(context, invitation), 'group-sync-pending')
    equal(store.groupInvitation(context, invitation).state, 'syncing')
    backend.syncPending = false
    await store.acceptGroupInvitation(context, invitation)
    equal(store.groupInvitation(context, invitation).state, 'joined')
    equal(backend.calls, Array(2).fill({ group_did: group, invitation_id: invitation.inviteId }))
  })
})

Deno.test('group and conversation refresh failures cannot display a fully joined invitation', async () => {
  for (const failure of ['listFailure', 'sessionFailure', 'omitGroup'] as const) {
    await fixture(async (store, backend) => {
      const context = store.defaultContext()
      backend[failure] = true
      await rejects(() => store.acceptGroupInvitation(context, invitation), 'group-refresh-pending')
      equal(store.groupInvitation(context, invitation).state, 'syncing')
      backend[failure] = false
      await store.acceptGroupInvitation(context, invitation)
      equal(store.groupInvitation(context, invitation).state, 'joined')
    })
  }
})

Deno.test('a later directory and session refresh settles background synchronization without accepting again', async () => {
  await fixture(async (store, backend) => {
    const context = store.defaultContext()
    backend.syncPending = true
    backend.omitGroup = true
    await rejects(() => store.acceptGroupInvitation(context, invitation), 'group-sync-pending')
    equal(store.groupInvitation(context, invitation).state, 'syncing')
    backend.syncPending = false
    backend.omitGroup = false
    await store.ensureOwner(context, true)
    equal(store.groupInvitation(context, invitation).state, 'joined')
    equal(backend.calls.length, 1)
  })
})

Deno.test('a lost accept response stays retryable and an agent acceptance uses its own returned state', async () => {
  await fixture(async (store, backend) => {
    const context = store.defaultContext()
    backend.responseFailure = 'remote-group-unavailable'
    await rejects(() => store.acceptGroupInvitation(context, invitation), 'remote-group-unavailable')
    equal(store.groupInvitation(context, invitation).state, 'pending')
    backend.responseFailure = null
    await store.acceptGroupInvitation(context, invitation)
    equal(store.groupInvitation(context, invitation).state, 'joined')
    equal(backend.calls[0], backend.calls[1])
  })
  await fixture(async (store, backend) => {
    const context = store.defaultContext()
    backend.omitGroup = true
    const forAgent = { ...invitation, memberDid: 'did:bns:my-agent' }
    await store.acceptGroupInvitation(context, forAgent)
    equal(store.groupInvitation(context, forAgent).state, 'joined')
    equal(store.groupInvitation(context, invitation).state, 'pending')
    equal(backend.calls[0].member_did, forAgent.memberDid)
  })
})

Deno.test('My Network verifies the group projection and preserves pending approval outcomes', async () => {
  await fixture(async (_store, backend) => {
    const network = createRealNetworkBackend()
    backend.syncPending = true
    await rejects(() => network.acceptGroupInvitation(networkInvitation), 'group-sync-pending')
    backend.syncPending = false
    backend.listFailure = true
    await rejects(() => network.acceptGroupInvitation(networkInvitation), 'group-refresh-pending')
    backend.listFailure = false
    equal(await network.acceptGroupInvitation(networkInvitation), 'active')
    backend.state = 'pending_admin_approval'
    equal(await network.acceptGroupInvitation(networkInvitation), 'pending_admin_approval')
    const pending = (await network.listGroups()).find(item => item.did === group)
    equal([pending?.state, pending?.hostedHere], ['pending_admin_approval', false])
  })
})

Deno.test('invitation errors distinguish remote failures, expired identity and local refresh failures', () => {
  const t = (key: string) => key
  for (const reason of ['remote-group-unavailable', 'remote-group-auth-required', 'remote-group-auth-failed', 'remote-group-not-found', 'group-sync-pending', 'group-refresh-pending', 'invitation-mismatch', 'invite-expired', 'invitation-expired', 'invitation-revoked']) {
    equal(groupErrorReason(new MessageHubApiError(`msg-center: ${reason} timeout`, 'unavailable')), reason)
    equal(groupErrorText(t, Error(`msg-center: ${reason}`)), `messagehub.groupError.${reason}`)
    equal(networkErrorText(t, Error(`msg-center: ${reason}`)), `network.error.${reason}`)
  }
  equal(groupErrorReason(new MessageHubApiError('failed to fetch', 'unavailable')), 'unavailable')
  equal(groupErrorText(t, Error('unavailable')), 'messagehub.groupError.unavailable')
})
