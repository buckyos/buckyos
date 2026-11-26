// Joining a group hosted by another Zone (issue #760), on real Zones: alice hosts, charlie joins
// from his own Zone by the group DID, by an invite link, or by accepting a directed invitation.
// Nothing here calls `group.sync_joined`: approvals and rejections must reach charlie's Zone through
// its own background follow-up (about every 30 s).
//
// Run against the ssotest VM group (src/dev_configs/ssotest.json), both OODs with the msg-center of
// this tree (the host writes the request id, the member Zone forwards and follows it up):
//   BUCKYOS_TEST_DNS=<sn ip> node test/test_msg_center/cross_zone_join_dv.mjs
// Environment as in test/nfs_server_test/attachment_dv_common.mjs. alice's Zone keeps an account
// whose DID is bob's (hybrid SSO DV), so bob is not used as the member. Each run leaves groups named
// `#760 <case> <time>` on alice's Zone, with charlie in some of them.
import {
  check, finish, http, log, login, rpc, rpcRaw, sleep, zones,
} from '../nfs_server_test/attachment_dv_common.mjs';

const HOST = 'alice';
const MEMBER = 'charlie';
const tokens = { [HOST]: await login(HOST), [MEMBER]: await login(MEMBER) };
const did = user => zones[user].did;
const stamp = Date.now();
const FOLLOW_UP_MS = 120_000;

const host = (method, params) => rpc(HOST, 'msg-center', method, params, tokens[HOST]);
const member = (method, params) => rpc(MEMBER, 'msg-center', method, params, tokens[MEMBER]);
const memberRaw = (method, params) => rpcRaw(MEMBER, 'msg-center', method, params, tokens[MEMBER]);
const reason = answer => JSON.stringify(answer.error ?? '');

async function createGroup(label, membership = {}) {
  const created = await host('group.create', {
    idempotency_key: `g760-${label}-${stamp}`,
    configuration: { profile: { name: `#760 ${label} ${stamp}` }, default_session: { history: 'all' }, membership },
  });
  log('group created', { label, group: created.group_did });
  return created.group_did;
}
/** A lookup before the document is published is remembered as "no such group" for ten minutes. */
async function awaitPublished(group) {
  const name = group.startsWith('did:bns:') ? `${group.slice('did:bns:'.length)}.${zones[HOST].host.split('.').slice(1).join('.')}` : group.slice('did:web:'.length);
  for (let attempt = 0; attempt < 60; attempt++) {
    const answer = await http(HOST, 'GET', '/.well-known/did.json', { headers: { Host: name } });
    if (answer.status === 200 && answer.text().startsWith('ey')) return;
    await sleep(3000);
  }
  throw new Error(`the document of ${group} was not published`);
}
async function setPolicy(group, policy) {
  const config = await host('group.get_config', { group_did: group });
  await host('group.apply_config', {
    group_did: group, expected_revision: config.revision, idempotency_key: `policy-${policy}-${stamp}`,
    patch: { membership: { join_policy: policy } },
  });
}
async function lookupGroup(group) {
  const result = await member('contact.lookup', { input: group, kinds: ['group'], contact_mgr_owner: did(MEMBER) });
  return result.candidates?.find(candidate => candidate.did === group);
}
const listed = () => member('group.list_by_member', {});
const invitationsOf = async group => (await listed()).invitations?.filter(item => (item.group_did ?? item.doc?.id) === group) ?? [];
const joinedTo = async group => ((await listed()).joined ?? []).some(item => item.group_did === group);
async function hostRecord(group) {
  return (await host('group.list_members', { group_did: group })).items.find(item => item.member_did === did(MEMBER));
}
async function until(label, probe, timeout = FOLLOW_UP_MS) {
  const started = Date.now();
  for (;;) {
    const value = await probe();
    if (value) {
      log('reached', { label, after_ms: Date.now() - started });
      return value;
    }
    if (Date.now() - started > timeout) throw new Error(`FAILED ${label}: not within ${timeout} ms`);
    await sleep(3000);
  }
}
async function hostPostReaches(group, text) {
  const at = Date.now();
  const sent = await host('msg.post_send', {
    idempotency_key: `g760-${stamp}-${at}`,
    msg: { from: did(HOST), to: [group], kind: 'group_msg', created_at_ms: at, nonce: at, content: { format: 'text/plain', content: text } },
  });
  check(sent.ok, `host accepts "${text}"`, sent);
  await until(`"${text}" in ${MEMBER}'s INBOX`, async () => {
    const page = await member('msg.peek_box', { mailbox: `${did(MEMBER)}/${group}`, box_kind: 'INBOX', limit: 50, with_object: false });
    return page.some(({ record }) => record.msg_id === sent.msg_id);
  }, 60_000);
}

// ── A: by DID, the host's join policy decides; approval reaches the member's Zone ──
const g1 = await createGroup('policy');
await awaitPublished(g1);
const found = await lookupGroup(g1);
check(found?.group?.hosted_here === false, 'lookup: the group is hosted by another Zone', found);
check(!found?.relation?.group_state, 'lookup: no state before joining', found?.relation);
const doc = await member('group.get_doc', { group_did: g1 });
check(doc.doc?.join_policy === 'invite_only' && doc.doc?.profile?.name === `#760 policy ${stamp}`, 'the document comes through charlie\'s Zone with the join policy', doc.doc);
let answer = await memberRaw('group.request_join', { group_did: g1 });
check(reason(answer).includes('invite-required'), 'invite_only: invite-required', answer);
check((await invitationsOf(g1)).length === 0 && !(await joinedTo(g1)), 'invite_only: nothing is left behind', await listed());

await setPolicy(g1, 'request_and_approve');
const requested = await member('group.request_join', { group_did: g1 });
check(requested.state === 'pending_admin_approval' && typeof requested.invitation_id === 'string', 'request_and_approve: waits, with the request id', requested);
check((await invitationsOf(g1))[0]?.member?.state === 'pending_admin_approval', 'the wait is listed with charlie\'s invitations');
check((await lookupGroup(g1))?.relation?.group_state === 'pending_admin_approval', 'lookup: pending_admin_approval');
check((await hostRecord(g1))?.state === 'pending_admin_approval', 'the host holds the request');
answer = await memberRaw('group.request_join', { group_did: g1 });
check(reason(answer).includes('member-already-participating'), 'asking again while waiting is refused', answer);
check((await lookupGroup(g1))?.relation?.group_state === 'pending_admin_approval', '...and the wait stays');
await host('group.approve_member', { group_did: g1, member_did: did(MEMBER) });
await until('approval reaches charlie\'s Zone on its own', () => joinedTo(g1));
check((await lookupGroup(g1))?.relation?.group_state === 'active', 'lookup: active');
check((await invitationsOf(g1)).length === 0, 'the wait is no longer listed');
await hostPostReaches(g1, `#760 after approval ${stamp}`);

// ── B: a rejection ends the request; the follow-up never asks again ──
const g2 = await createGroup('reject', { join_policy: 'request_and_approve' });
await awaitPublished(g2);
check((await member('group.request_join', { group_did: g2 })).state === 'pending_admin_approval', 'second group: waits');
await host('group.reject_member', { group_did: g2, member_did: did(MEMBER) });
await until('the rejection reaches charlie\'s Zone', async () => (await invitationsOf(g2)).length === 0);
check(!(await lookupGroup(g2))?.relation?.group_state, 'lookup: no state after the rejection');
await sleep(45_000);
check((await hostRecord(g2))?.state === 'rejected', 'the host still has the rejection after another follow-up round (not asked again)', await hostRecord(g2));

// ── C: by invite link ──
const g3 = await createGroup('link');
await awaitPublished(g3);
const link = await host('group.create_invite_link', { group_did: g3, max_uses: 1 });
const linked = await member('group.request_join', { group_did: g3, invite: link.token });
check(linked.state === 'active' && linked.sync_pending === false, 'invite link: joined at once and synced', linked);
check(await joinedTo(g3), 'invite link: listed as joined');
await hostPostReaches(g3, `#760 after the link ${stamp}`);

// ── D: a directed invitation waits with charlie's invitations, with the group's name ──
const g4 = await createGroup('invite');
await awaitPublished(g4);
const invitation = await host('group.invite_member', { group_did: g4, member_did: did(MEMBER) });
const waiting = await until('the invitation is listed in charlie\'s Zone', async () => (await invitationsOf(g4))[0]);
check(waiting.member?.state === 'invited' && waiting.member?.invitation_id === invitation.invite_id && waiting.member?.invited_by === did(HOST), 'invited, with the invitation id and the inviter', waiting);
check(waiting.hosted_here === false && waiting.doc === null && waiting.profile?.name === `#760 invite ${stamp}`, 'no document, the name from the invitation', waiting);
check((await lookupGroup(g4))?.relation?.group_state === 'invited', 'lookup: invited');
const accepted = await member('group.accept_invitation', { group_did: g4, invitation_id: invitation.invite_id });
check(accepted.state === 'active', 'accepting joins', accepted);
check((await invitationsOf(g4)).length === 0 && await joinedTo(g4), 'accepted: no longer listed, joined');

// ── E: a revoked invitation goes away once accepting it finds out ──
const g5 = await createGroup('revoked');
await awaitPublished(g5);
const revoked = await host('group.invite_member', { group_did: g5, member_did: did(MEMBER) });
await until('the second invitation is listed', async () => (await invitationsOf(g5)).length === 1);
await host('group.revoke_invite', { group_did: g5, member_did: did(MEMBER) });
answer = await memberRaw('group.accept_invitation', { group_did: g5, invitation_id: revoked.invite_id });
check(reason(answer).includes('invitation-revoked'), 'accepting a revoked invitation: invitation-revoked', answer);
check((await invitationsOf(g5)).length === 0, 'the revoked invitation is no longer listed');

log('groups', { g1, g2, g3, g4, g5 });
finish('#760 cross-Zone join DV');
