// Attachments of a group across real Zones (issue #689 P2): the author in one Zone, the host in
// another, a member in a third. The host fetches from the author as the group, on its own; the member
// asks the host, as itself, and never looks for the author. The host's rules decide throughout.
//
// Run against the ssotest VM group (src/dev_configs/ssotest.json), from test/nfs_server_test:
//   BUCKYOS_TEST_DNS=<sn ip> BUCKYOS_TEST_DEVICE_DIR=<dir> \
//   BUCKYOS_TEST_AUTHOR_DOWN='<takes alice off the network>' BUCKYOS_TEST_AUTHOR_UP='<brings her back>' \
//     node group_attachment_dv.mjs
// Freezing her gateway does (`buckyos-devtest ssotest run alice-ood1 "sudo pkill -STOP -x cyfs_gateway"`, then -CONT).
// `qm suspend` does too, but a VM resumes with a clock behind and SN tokens read as from the future until it syncs.
//
// See remote_attachment_dv.mjs for BUCKYOS_TEST_DEVICE_DIR and the other variables. BUCKYOS_TEST_GROUP_HOST
// picks the host, bob (a did:bns group) by default or charlie (did:web). Each run leaves a group named
// `#689 <time>` on the host.
import { randomBytes } from 'node:crypto';
import {
  check, crossZoneLogin, crossZoneSession, describe, describedRemotely, finish, hex, http, log, login, memberTicket, ndnPath, notVerified, readAttachment,
  rpc, rpcRaw, runGiven, setAccess, sleep, ticket, upload, verifiedRead, zones,
} from './attachment_dv_common.mjs';

const HOST = process.env.BUCKYOS_TEST_GROUP_HOST?.trim() || 'bob';
const AUTHOR = 'alice';
const MEMBER = HOST === 'bob' ? 'charlie' : 'bob';
const tokens = { [HOST]: await login(HOST), [AUTHOR]: await login(AUTHOR), [MEMBER]: await login(MEMBER) };
const did = user => zones[user].did;
const stamp = Date.now();

async function createGroup(label) {
  const created = await rpc(HOST, 'msg-center', 'group.create', {
    idempotency_key: `g689-${label}-${stamp}`, profile: { name: `#689 ${label} ${stamp}` }, configuration: { default_session: { history: 'all' } },
  }, tokens[HOST]);
  return created.group_did;
}
/** A new group's document is published within a minute; a lookup before that is remembered as "no such group" for ten. */
async function awaitPublished(group) {
  const host = group.startsWith('did:bns:') ? `${group.slice('did:bns:'.length)}.${zones[HOST].host.split('.').slice(1).join('.')}` : group.slice('did:web:'.length);
  for (let attempt = 0; attempt < 60; attempt++) {
    const answer = await http(HOST, 'GET', '/.well-known/did.json', { headers: { Host: host } });
    if (answer.status === 200 && answer.text().startsWith('ey')) return;
    await sleep(3000);
  }
  throw new Error(`the document of ${group} was not published`);
}
/** Invited by the host, accepted from the member's Zone. */
async function join(group, user) {
  await awaitPublished(group);
  const invitation = await rpc(HOST, 'msg-center', 'group.invite_member', { group_did: group, member_did: did(user) }, tokens[HOST]);
  if (invitation.state === 'active') return;
  let last;
  for (let attempt = 0; attempt < 10; attempt++) {
    last = await rpcRaw(user, 'msg-center', 'group.accept_invitation', { group_did: group, invitation_id: invitation.invite_id }, tokens[user]);
    if (last.result?.state === 'active') return;
    await sleep(3000);
  }
  throw new Error(`${user} did not join ${group}: ${JSON.stringify(last)}`);
}
async function post(group, from, file, extra = {}) {
  const at = Date.now();
  const text = `#689 group ${from} ${at}`;
  const sent = await rpc(from, 'msg-center', 'msg.post_send', {
    msg: {
      from: did(from), to: [group], kind: 'group_msg', created_at_ms: at, nonce: at,
      content: { format: 'text/plain', content: text, ...(file ? { refs: [{ role: 'input', target: { type: 'data_obj', obj_id: file.id, uri_hint: `cyfs://${file.id}` }, label: file.name }] } : {}) },
      ...extra,
    },
    idempotency_key: `g689-post-${at}`,
  }, tokens[from]);
  check(sent.ok === true, `${from} posts to the group`, sent);
  return { msgId: sent.msg_id, text };
}
/** The message in `user`'s timeline of the group's default Session. */
async function awaitInSession(user, group, text) {
  for (let attempt = 0; attempt < 80; attempt++) {
    if (user !== HOST) await rpcRaw(user, 'msg-center', 'group.sync_joined', { group_did: group }, tokens[user]);
    const page = await rpc(user, 'msg-center', 'msg.list_session', { owner: did(user), session_id: group, limit: 50, descending: true, with_object: true }, tokens[user]);
    const found = page.items?.find(item => item.msg?.content?.content === text);
    if (found) return found;
    await sleep(3000);
  }
  throw new Error(`${user}: "${text}" not in the group's timeline`);
}
const contextOf = (group, msgId) => `${group}/${msgId}`;
const blob = label => Buffer.from(`${label} ${randomBytes(8).toString('hex')}\n`.repeat(2048));

const group = await createGroup('main');
log('group', { group });
await join(group, AUTHOR);
await join(group, MEMBER);

// 1. the author posts; the host fetches on its own, with nobody reading
const file = await upload(AUTHOR, tokens[AUTHOR], 'team-notes.txt', blob('from the author'));
const message = await post(group, AUTHOR, file);
await awaitInSession(HOST, group, message.text);
let hostHolds = false;
for (let attempt = 0; attempt < 60 && !hostHolds; attempt++) {
  const described = await describe(HOST, tokens[HOST], file.id, contextOf(group, message.msgId));
  hostHolds = described.status === 200 && !describedRemotely(described);
  if (!hostHolds) await sleep(2000);
}
check(hostHolds, 'the host fetched the attachment by itself, with no read');

// 2. who speaks for the group at the author's Zone: the host's device, nobody else's
const asGroup = await crossZoneLogin(HOST, AUTHOR, group);
if (!asGroup) {
  notVerified.push('who speaks for the group (BUCKYOS_TEST_DEVICE_DIR not given)');
} else {
  check(asGroup.result?.session_token, 'the host\'s device signs in as the group', asGroup.error);
  const issued = await ticket(AUTHOR, message.msgId, file.id, asGroup.result.session_token);
  check(issued.result?.token?.startsWith('bkt_'), 'and the group gets a ticket from the author\'s Zone', issued.error);
  const pretender = await crossZoneLogin(MEMBER, AUTHOR, group);
  check(pretender.error && !pretender.result?.session_token, 'another Zone\'s device cannot speak for the group', pretender);
  log('pretender', { error: pretender.error });
  const memberThere = await crossZoneSession(MEMBER, AUTHOR);
  const memberAsks = await ticket(AUTHOR, message.msgId, file.id, memberThere);
  check(memberAsks.error && !memberAsks.result, 'a member asking the author\'s Zone as itself gets nothing: the host decides');
}

// 3. the author leaves; a member's Zone reads through the host
const authorAway = runGiven('BUCKYOS_TEST_AUTHOR_DOWN', 'the author offline when a member reads');
try {
  await awaitInSession(MEMBER, group, message.text);
  const card = await describe(MEMBER, tokens[MEMBER], file.id, message.msgId);
  check(card.status === 200 && JSON.parse(card.text()).name === 'team-notes.txt' && describedRemotely(card), 'a member\'s card asks the host for the description', `HTTP ${card.status} ${card.text().slice(0, 200)}`);
  await verifiedRead(MEMBER, tokens[MEMBER], file, message.msgId);
  const local = await describe(MEMBER, tokens[MEMBER], file.id, message.msgId);
  check(local.status === 200 && !describedRemotely(local), 'and the member\'s Zone keeps it now');
} finally {
  if (authorAway) runGiven('BUCKYOS_TEST_AUTHOR_UP', 'the author back');
}

// 4. the host's rules for whoever asks: a group the member is not in
const other = await createGroup('other');
await join(other, AUTHOR);
const otherFile = await upload(AUTHOR, tokens[AUTHOR], 'other-notes.txt', blob('for the other group'));
const otherMessage = await post(other, AUTHOR, otherFile);
await awaitInSession(HOST, other, otherMessage.text);
const memberAtHost = await crossZoneSession(MEMBER, HOST);
if (memberAtHost) {
  const own = await memberTicket(HOST, contextOf(group, message.msgId), file.id, memberAtHost);
  check(own.result?.token?.startsWith('bkt_'), 'a member gets a ticket from the host', own.error);
  const notIn = await memberTicket(HOST, contextOf(other, otherMessage.msgId), otherFile.id, memberAtHost);
  const none = await memberTicket(HOST, contextOf(other, message.msgId), otherFile.id, memberAtHost);
  check(notIn.error && JSON.stringify(notIn.error) === JSON.stringify(none.error), 'who is not in a group gets nothing, and learns nothing', { notIn: notIn.error, none: none.error });
  const direct = await ticket(HOST, message.msgId, file.id, memberAtHost);
  check(direct.error && !direct.result, 'by the message alone only the group itself may ask');
} else {
  notVerified.push('tickets from the host (BUCKYOS_TEST_DEVICE_DIR not given)');
}

// 5. what a display shows
const rules = await upload(HOST, tokens[HOST], 'house-rules.txt', blob('house rules'));
const hidden = await upload(HOST, tokens[HOST], 'not-shown.txt', blob('not shown'));
const config = await rpc(HOST, 'msg-center', 'group.get_config', { group_did: group }, tokens[HOST]);
const applied = await rpc(HOST, 'msg-center', 'group.apply_config', {
  group_did: group, expected_revision: config.revision, idempotency_key: `g689-display-${stamp}`, patch: { display: { files: [{ title: 'Rules', obj_id: rules.id }] } },
}, tokens[HOST]);
await rpcRaw(MEMBER, 'msg-center', 'group.sync_joined', { group_did: group }, tokens[MEMBER]);
const displayContext = `${group}/_display`;
const displayPath = (id, content) => `/kapi/msg-center/objects/${id}${content ? '/content' : ''}?context_path=${encodeURIComponent(displayContext)}`;
const memberGet = (id, content) => http(MEMBER, 'GET', displayPath(id, content), { timeout: 120000, headers: { Authorization: `Bearer ${tokens[MEMBER]}` } });
const shown = await memberGet(rules.id, false);
check(shown.status === 200 && JSON.parse(shown.text()).name === 'house-rules.txt', 'a joined member reads the description of a display file', `HTTP ${shown.status} ${shown.text().slice(0, 200)}`);
const shownContent = await memberGet(rules.id, true);
check(shownContent.status === 200 && hex(shownContent.bytes) === rules.sha, 'and its content', `HTTP ${shownContent.status}`);
const notShown = await memberGet(hidden.id, true);
check(notShown.status === 404, 'what no display shows stays out of reach', `HTTP ${notShown.status}`);
if (memberAtHost) {
  const displayTicket = await memberTicket(HOST, displayContext, rules.id, memberAtHost);
  check(displayTicket.result?.token?.startsWith('bkt_'), 'a member gets a ticket for a display file', displayTicket.error);
  const before = await http(HOST, 'GET', ndnPath(rules.id, displayTicket.result.token));
  check(before.status === 200, 'it reads at /ndn/', `HTTP ${before.status}`);
  await rpc(HOST, 'msg-center', 'group.apply_config', {
    group_did: group, expected_revision: applied.revision, idempotency_key: `g689-display2-${stamp}`, patch: { display: { files: [{ title: 'Other', obj_id: hidden.id }] } },
  }, tokens[HOST]);
  const after = await http(HOST, 'GET', ndnPath(rules.id, displayTicket.result.token));
  check(after.status === 404, 'and stops the moment the display no longer shows it', `HTTP ${after.status}`);
}

// 6. a recall ends it, for anyone who has not got it
const recalled = await upload(AUTHOR, tokens[AUTHOR], 'recalled.txt', blob('to be recalled'));
const recalledMessage = await post(group, AUTHOR, recalled);
await awaitInSession(MEMBER, group, recalledMessage.text);
await post(group, HOST, null, { relates_to: { rel: 'redact', target: recalledMessage.msgId } });
let gone = false;
for (let attempt = 0; attempt < 40 && !gone; attempt++) {
  await rpcRaw(MEMBER, 'msg-center', 'group.sync_joined', { group_did: group }, tokens[MEMBER]);
  gone = (await readAttachment(MEMBER, tokens[MEMBER], recalled.id, recalledMessage.msgId)).status === 404;
  if (!gone) await sleep(3000);
}
check(gone, 'a recalled message\'s attachment is gone for a member who had not read it');

// 7. the message comes from the group: a member who blocked its author gets it all the same
const authorIsContact = await rpcRaw(MEMBER, 'msg-center', 'contact.get_contact', { did: did(AUTHOR), contact_mgr_owner: did(MEMBER) }, tokens[MEMBER]);
const accessBefore = authorIsContact.result?.access_level ?? 'stranger';
await rpc(MEMBER, 'msg-center', 'contact.block_contact', { did: did(AUTHOR), contact_mgr_owner: did(MEMBER) }, tokens[MEMBER]);
try {
  const fromBlocked = await upload(AUTHOR, tokens[AUTHOR], 'from-blocked.txt', blob('from someone the member blocked'));
  const blockedMessage = await post(group, AUTHOR, fromBlocked);
  const seen = await awaitInSession(MEMBER, group, blockedMessage.text);
  check(seen.msg?.from === did(AUTHOR), 'a member who blocked the author still gets the message in the group', seen);
  const blockedCard = await describe(MEMBER, tokens[MEMBER], fromBlocked.id, blockedMessage.msgId);
  check(blockedCard.status === 200, 'and the description of its attachment', `HTTP ${blockedCard.status} ${blockedCard.text().slice(0, 200)}`);
  await verifiedRead(MEMBER, tokens[MEMBER], fromBlocked, blockedMessage.msgId);
} finally {
  await setAccess(MEMBER, AUTHOR, accessBefore, tokens[MEMBER]);
}

// 8. the owner's Zone is the host: an owner elsewhere would move the group; an admin is another matter
const transfer = await rpcRaw(HOST, 'msg-center', 'group.transfer_owner', { group_did: group, member_did: did(MEMBER) }, tokens[HOST]);
check(JSON.stringify(transfer.error ?? '').includes('owner-must-be-in-the-host-zone'), 'ownership does not go to another Zone', transfer);
const admin = await rpcRaw(HOST, 'msg-center', 'group.update_member_role', { group_did: group, member_did: did(MEMBER), role: 'admin' }, tokens[HOST]);
check(!admin.error, 'an admin may live in another Zone', admin.error);
await rpc(HOST, 'msg-center', 'group.update_member_role', { group_did: group, member_did: did(MEMBER), role: 'member' }, tokens[HOST]);

// 9. removed: no new tickets, and what came stays. The member blocked the one who acts and still
// hears it: the notice comes from the group, into the group's session.
const actorIsContact = await rpcRaw(MEMBER, 'msg-center', 'contact.get_contact', { did: did(HOST), contact_mgr_owner: did(MEMBER) }, tokens[MEMBER]);
const actorAccessBefore = actorIsContact.result?.access_level ?? 'stranger';
await rpc(MEMBER, 'msg-center', 'contact.block_contact', { did: did(HOST), contact_mgr_owner: did(MEMBER) }, tokens[MEMBER]);
await rpc(HOST, 'msg-center', 'group.remove_member', { group_did: group, member_did: did(MEMBER) }, tokens[HOST]);
try {
  let notice;
  for (let attempt = 0; attempt < 40 && !notice; attempt++) {
    const page = await rpc(MEMBER, 'msg-center', 'msg.list_session', { owner: did(MEMBER), session_id: group, limit: 20, descending: true, with_object: true }, tokens[MEMBER]);
    notice = page.items?.find(item => item.msg?.content?.machine?.data?.action === 'removed');
    if (!notice) await sleep(3000);
  }
  check(notice?.msg?.from === group, 'the group tells a member who blocked the one who acted', notice?.msg);
  check(notice?.msg?.content?.machine?.data?.by === did(HOST), 'and names who acted', notice?.msg?.content?.machine?.data);
} finally {
  await setAccess(MEMBER, HOST, actorAccessBefore, tokens[MEMBER]);
}
if (memberAtHost) {
  const afterRemoval = await memberTicket(HOST, contextOf(group, message.msgId), file.id, memberAtHost);
  check(afterRemoval.error && !afterRemoval.result, 'a removed member gets no ticket', afterRemoval.result ? 'a token was issued' : afterRemoval);
}
const kept = await readAttachment(MEMBER, tokens[MEMBER], file.id, message.msgId);
check(kept.status === 200 && hex(kept.bytes) === file.sha, 'and keeps what came', `HTTP ${kept.status}`);

finish('Group attachment DV');
