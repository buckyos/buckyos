// Attachments of private messages across real Zones (bucket design §21 N18, issue #689):
// the recipient's Zone holds nothing when the message arrives and pulls the bytes from the sender's
// Zone on first read of the content. A recipient reads them end to end; nobody else gets a token.
// A sender not accepted yet gets nothing out of the reader's Zone. The contact relations this needs
// are set at the start of the run, whatever earlier runs left.
//
// Run against the ssotest VM group (src/dev_configs/ssotest.json), from test/nfs_server_test:
//   BUCKYOS_TEST_DNS=<sn ip> BUCKYOS_TEST_DEVICE_DIR=<dir> node remote_attachment_dv.mjs
//
// A non-recipient asks exactly as its Zone's msg-center would: a cross-Zone assertion signed with
// the device key, then `msg.attachment_ticket`. BUCKYOS_TEST_DEVICE_DIR holds `<user>.pem` (the
// node's private key) and `<user>.did` (its device DID) for bob and charlie; without it those
// checks are reported as NOT VERIFIED and the run does not count as a pass.
//
// alice's Zone keeps an account `bobfed` for did:bns:bob (test_hybrid_sso_federation_dv), so a message
// from alice to bob stays local there: bob is the bystander in this run, and charlie the recipient.
//
// The SN also answers AAAA for a NAT Zone: on a LAN with global IPv6, a Zone created before
// ssotest.json rejected IPv6 is reached directly and the relay is not exercised. Which path was
// taken shows in the sender's gateway log, as the source of the `/ndn/` requests.
// Other env as test/test_msg_center/test_native_routes_dv.mjs: BUCKYOS_TEST_CA,
// BUCKYOS_TEST_PASSWORD, BUCKYOS_TEST_ZONE_A/B/C, BUCKYOS_TEST_HOST_MAP.
import { randomBytes } from 'node:crypto';
import { ndn } from 'buckyos';
import {
  CHUNK, MAX_READ_BYTES, awaitMessage, check, contentPath, crossZoneSession, describe, describedRemotely, device, finish, hex, http, log, login, lookup, ndnPath, notVerified,
  pushSigned, readAttachment, rpc, rpcRaw, secondAccount, send, setAccess, sleep, ticket, upload, verifiedRead, zones,
} from './attachment_dv_common.mjs';

// ── the run ─────────────────────────────────────────────────────────────────

const tokens = { alice: await login('alice'), charlie: await login('charlie'), bob: await login('bob') };
for (const user of Object.keys(zones)) log('zone', { user, host: zones[user].host, address: await new Promise((ok, no) => lookup(zones[user].host, {}, (e, a) => e ? no(e) : ok(a))) });

// Who takes whose messages at the start of a run: nobody anyone's.
for (const [owner, peer] of [['charlie', 'alice'], ['alice', 'charlie'], ['bob', 'alice'], ['bob', 'charlie'], ['alice', 'bob']]) await setAccess(owner, peer, 'stranger', tokens[owner]);

// A file over one chunk goes through the typed walk (`File.content`, `ChunkList[i]`).
const small = await upload('alice', tokens.alice, 'note.txt', Buffer.from(`hello from alice ${randomBytes(8).toString('hex')}\n`.repeat(4096)));
const big = await upload('alice', tokens.alice, 'big.bin', randomBytes(CHUNK + 5 * 1024 * 1024));
log('uploaded', { small: small.id, big: big.id, big_size: big.bytes.length });

// 1. alice → charlie: the sender's Zone is behind NAT, reached through the SN relay
const toRecipient = await send('alice', 'charlie', [small, big], tokens.alice);
const arrived = await awaitMessage('charlie', 'alice', toRecipient.text, tokens.charlie);
const wire = JSON.stringify(arrived);
check(wire.includes(small.id) && wire.includes(big.id), 'the message names both attachments');
check(!wire.includes('bkt_') && !wire.includes('hint='), 'no token travels in the message');

// alice is a stranger to charlie so far: nothing goes out of his Zone, not even for a description.
const unaccepted = await readAttachment('charlie', tokens.charlie, small.id, toRecipient.msgId);
check(unaccepted.status === 403 && unaccepted.text() === 'attachment-awaiting-acceptance', 'a stranger\'s attachment waits for acceptance', `HTTP ${unaccepted.status} ${unaccepted.text().slice(0, 100)}`);
const unacceptedCard = await describe('charlie', tokens.charlie, small.id, toRecipient.msgId);
check(unacceptedCard.status === 403, 'and so does its description', `HTTP ${unacceptedCard.status}`);
log('stranger', { note: 'alice\'s gateway log must show no request for this message before the next step', msg: toRecipient.msgId });
await setAccess('charlie', 'alice', 'allow', tokens.charlie);

// A card asks for the description only; it comes from the sender and binds nothing.
const card = await describe('charlie', tokens.charlie, big.id, toRecipient.msgId);
check(card.status === 200 && JSON.parse(card.text()).name === 'big.bin' && describedRemotely(card), 'a card reads the description from the sender', `HTTP ${card.status} ${card.text().slice(0, 200)}`);
const cardAgain = await describe('charlie', tokens.charlie, big.id, toRecipient.msgId);
check(cardAgain.status === 200 && describedRemotely(cardAgain), 'and it stays there until the content is asked for');

const smallRead = await verifiedRead('charlie', tokens.charlie, small, toRecipient.msgId);
check(String(smallRead.headers['content-type']).startsWith('text/plain') && String(smallRead.headers['content-disposition']).startsWith('inline'), 'text stays inline', smallRead.headers);
const bigRead = await verifiedRead('charlie', tokens.charlie, big, toRecipient.msgId);
check(bigRead.headers['content-type'] === 'application/octet-stream' && String(bigRead.headers['content-disposition']).startsWith('attachment'), 'an unknown type downloads', bigRead.headers);
const local = await describe('charlie', tokens.charlie, big.id, toRecipient.msgId);
check(local.status === 200 && !describedRemotely(local), 'once here, the description is local');

// 2. who else can get at it, at the sender's Zone
const unrelated = new ndn.FileObject('nothing', 1, small.id).genObjId()[0].toString();
check((await http('alice', 'GET', ndnPath(small.id))).status === 404, 'anonymous /ndn/ at the sender: 404');
check((await http('alice', 'GET', ndnPath(small.id, `bkt_${randomBytes(16).toString('hex')}`))).status === 404, 'a made-up token at the sender: 404');
const anonymous = await ticket('alice', toRecipient.msgId, small.id, null);
check(anonymous.error && !anonymous.result, 'no session, no ticket', anonymous);
const senderAsked = await ticket('alice', toRecipient.msgId, small.id, tokens.alice);
check(senderAsked.error && !senderAsked.result, 'the sender herself is no recipient: no ticket', senderAsked);

const charlieAtAlice = await crossZoneSession('charlie', 'alice');
const bobAtAlice = await crossZoneSession('bob', 'alice');
if (!charlieAtAlice || !bobAtAlice) {
  notVerified.push('non-recipient and recipient asking across Zones (BUCKYOS_TEST_DEVICE_DIR not given)');
} else {
  const issued = await ticket('alice', toRecipient.msgId, small.id, charlieAtAlice);
  check(issued.result?.token?.startsWith('bkt_'), 'the recipient, asking across Zones, gets a ticket', issued.error ?? 'no token');
  const left = issued.result.expires_at - Date.now();
  // One hour, give or take the clock difference between this machine and the Zone.
  check(left > 0 && left <= 3660 * 1000, 'the ticket is short-lived', left);
  const viaTicket = await http('alice', 'GET', ndnPath(small.id, issued.result.token));
  check(viaTicket.status === 200 && hex(viaTicket.bytes) === small.sha, 'the ticket reads that attachment at /ndn/', `HTTP ${viaTicket.status}`);
  const otherFile = await http('alice', 'GET', ndnPath(big.id, issued.result.token));
  check(otherFile.status === 404, 'and no other attachment of the same message', `HTTP ${otherFile.status}`);
  const bigTicket = await ticket('alice', toRecipient.msgId, big.id, charlieAtAlice);
  const from = CHUNK - 100;
  const across = await http('alice', 'GET', ndnPath(big.id, bigTicket.result?.token), { headers: { Range: `bytes=${from}-${from + 199}` } });
  check(across.status === 206 && across.bytes.equals(big.bytes.subarray(from, from + 200)), '/ndn/ answers a Range across the chunk boundary', `HTTP ${across.status}`);

  const bobAsked = await ticket('alice', toRecipient.msgId, small.id, bobAtAlice);
  check(bobAsked.error && !bobAsked.result, 'a non-recipient, asking across Zones, gets no ticket', bobAsked.result ? 'a token was issued' : bobAsked);
  const bobOther = await ticket('alice', toRecipient.msgId, big.id, bobAtAlice);
  check(bobOther.error && !bobOther.result, 'nor for the other attachment');
  const noSuch = await ticket('alice', unrelated, small.id, bobAtAlice);
  check(noSuch.error && JSON.stringify(noSuch.error) === JSON.stringify(bobAsked.error), 'the refusal does not tell that the message exists', { asked: bobAsked.error, none: noSuch.error });
  const charlieOther = await ticket('alice', toRecipient.msgId, unrelated, charlieAtAlice);
  check(charlieOther.error && !charlieOther.result, 'a recipient gets no ticket for what the message does not reference');
  log('denials', { non_recipient: bobAsked.error, sender: senderAsked.error, anonymous: anonymous.error });
}

// 3. who else can get at it, at the recipient's Zone
const bobAtHome = await readAttachment('bob', tokens.bob, small.id, toRecipient.msgId);
check(bobAtHome.status === 404, 'bob\'s own Zone has no such message', `HTTP ${bobAtHome.status}`);
const bobAtCharlie = await crossZoneSession('bob', 'charlie');
if (bobAtCharlie) {
  const r = await readAttachment('charlie', bobAtCharlie, small.id, toRecipient.msgId);
  check(r.status !== 200 && !r.bytes.includes(small.bytes.subarray(0, 32)), 'bob cannot read it from charlie\'s Zone', `HTTP ${r.status}`);
} else {
  notVerified.push('non-recipient reading at the recipient\'s Zone (BUCKYOS_TEST_DEVICE_DIR not given)');
}
check((await http('charlie', 'GET', ndnPath(small.id))).status === 404, 'pulled does not mean public: anonymous /ndn/ at the recipient: 404');

// 4. the same file in another message is another key, and comes again
const again = await send('alice', 'charlie', [small], tokens.alice);
await awaitMessage('charlie', 'alice', again.text, tokens.charlie);
await verifiedRead('charlie', tokens.charlie, small, again.msgId);
if (bobAtAlice) {
  const stillNot = await ticket('alice', again.msgId, small.id, bobAtAlice);
  check(stillNot.error && !stillNot.result, 'and bob gets no ticket for that one either');
}

// 5. the other way: a Zone behind NAT pulls from one reached directly
const backFile = await upload('charlie', tokens.charlie, 'reply.txt', Buffer.from(`hello from charlie ${randomBytes(8).toString('hex')}\n`.repeat(1024)));
const toAlice = await send('charlie', 'alice', [backFile], tokens.charlie);
await awaitMessage('alice', 'charlie', toAlice.text, tokens.alice);
await setAccess('alice', 'charlie', 'allow', tokens.alice);
await verifiedRead('alice', tokens.alice, backFile, toAlice.msgId);

// 6. more than a read can hand over (64 MiB, Desktop's limit too): refused without fetching it
const huge = await upload('charlie', tokens.charlie, 'huge.bin', randomBytes(MAX_READ_BYTES + 1024));
const hugeMsg = await send('charlie', 'alice', [huge], tokens.charlie);
await awaitMessage('alice', 'charlie', hugeMsg.text, tokens.alice);
const refused = await readAttachment('alice', tokens.alice, huge.id, hugeMsg.msgId);
check(refused.status === 413, 'an attachment over the read limit: 413, not "try again"', `HTTP ${refused.status}`);
const refusedAgain = await readAttachment('alice', tokens.alice, huge.id, hugeMsg.msgId);
check(refusedAgain.status === 413, 'and the same on the next read', `HTTP ${refusedAgain.status}`);
log('not_fetched', { file: huge.id, note: 'the sender\'s gateway log must show this object and no chunk of it' });

// 7. a size the sender made up: the chunks decide, not `size`
const forgedBig = await upload('charlie', tokens.charlie, 'forged-big.bin', randomBytes(MAX_READ_BYTES + 1024), 10);
const forgedBigMsg = await send('charlie', 'alice', [forgedBig], tokens.charlie);
await awaitMessage('alice', 'charlie', forgedBigMsg.text, tokens.alice);
const forgedBigRead = await readAttachment('alice', tokens.alice, forgedBig.id, forgedBigMsg.msgId);
check(forgedBigRead.status === 413, 'a small size over big chunks: 413', `HTTP ${forgedBigRead.status} ${forgedBigRead.text().slice(0, 200)}`);
log('not_fetched', { file: forgedBig.id, note: 'the sender\'s gateway log must show this object and its list, and no chunk' });
const forgedSmall = await upload('charlie', tokens.charlie, 'forged-small.bin', randomBytes(4096), 10);
const forgedSmallMsg = await send('charlie', 'alice', [forgedSmall], tokens.charlie);
await awaitMessage('alice', 'charlie', forgedSmallMsg.text, tokens.alice);
const forgedSmallRead = await readAttachment('alice', tokens.alice, forgedSmall.id, forgedSmallMsg.msgId);
check(forgedSmallRead.status === 502, 'a size that disagrees with its chunks fails verification', `HTTP ${forgedSmallRead.status} ${forgedSmallRead.text().slice(0, 200)}`);

// 8. sending on and collecting what was never opened: fetched first, under the same rules
const unread = await upload('alice', tokens.alice, 'unread.txt', Buffer.from(`not opened ${randomBytes(8).toString('hex')}\n`.repeat(256)));
const unreadMsg = await send('alice', 'charlie', [unread], tokens.alice);
await awaitMessage('charlie', 'alice', unreadMsg.text, tokens.charlie);
const forwarded = await send('charlie', 'bob', [unread], tokens.charlie);
check(forwarded.msgId, 'charlie forwards an attachment he never opened');
await awaitMessage('bob', 'charlie', forwarded.text, tokens.bob);
await setAccess('bob', 'charlie', 'allow', tokens.bob);
await verifiedRead('bob', tokens.bob, unread, forwarded.msgId);
const toCollect = await upload('alice', tokens.alice, 'collect.txt', Buffer.from(`to collect ${randomBytes(8).toString('hex')}\n`.repeat(256)));
const toCollectMsg = await send('alice', 'charlie', [toCollect], tokens.alice);
await awaitMessage('charlie', 'alice', toCollectMsg.text, tokens.charlie);
const collected = await rpcRaw('charlie', 'msg-center', 'msg.collect_attachment', { obj_id: toCollect.id, context_path: toCollectMsg.msgId }, tokens.charlie);
check(collected.result?.key, 'charlie collects an attachment he never opened', collected.error);

// 9. a recipient the sender blocked gets no ticket; what came stays
const beforeBlock = await upload('alice', tokens.alice, 'before-block.txt', Buffer.from(`for charlie ${randomBytes(8).toString('hex')}\n`.repeat(64)));
const afterBlock = await upload('alice', tokens.alice, 'after-block.txt', Buffer.from(`too late ${randomBytes(8).toString('hex')}\n`.repeat(64)));
const beforeBlockMsg = await send('alice', 'charlie', [beforeBlock], tokens.alice);
const afterBlockMsg = await send('alice', 'charlie', [afterBlock], tokens.alice);
await awaitMessage('charlie', 'alice', beforeBlockMsg.text, tokens.charlie);
await awaitMessage('charlie', 'alice', afterBlockMsg.text, tokens.charlie);
await verifiedRead('charlie', tokens.charlie, beforeBlock, beforeBlockMsg.msgId);
await setAccess('alice', 'charlie', 'block', tokens.alice);
const blockedRead = await readAttachment('charlie', tokens.charlie, afterBlock.id, afterBlockMsg.msgId);
check(blockedRead.status === 404, 'a blocked recipient reads nothing new', `HTTP ${blockedRead.status} ${blockedRead.text().slice(0, 100)}`);
const stillRead = await readAttachment('charlie', tokens.charlie, beforeBlock.id, beforeBlockMsg.msgId);
check(stillRead.status === 200 && hex(stillRead.bytes) === beforeBlock.sha, 'and keeps what came');
if (charlieAtAlice) {
  const blockedTicket = await ticket('alice', afterBlockMsg.msgId, afterBlock.id, charlieAtAlice);
  const noSuchTicket = await ticket('alice', unrelated, afterBlock.id, charlieAtAlice);
  check(blockedTicket.error && JSON.stringify(blockedTicket.error) === JSON.stringify(noSuchTicket.error), 'the refusal reads as no such message', { blocked: blockedTicket.error, none: noSuchTicket.error });
}
await setAccess('alice', 'charlie', 'stranger', tokens.alice);

// 10. naming an object in a message opens nothing, nor does sending it on (N1): another account of
// bob's Zone gets a message from a lying Zone that names bob's private file.
if (device('charlie')) {
  const secret = await upload('bob', tokens.bob, 'bobs-private.txt', Buffer.from(`private to bob ${randomBytes(8).toString('hex')}\n`.repeat(64)));
  const mal = await secondAccount('bob', 'mal689');
  const malAccess = level => rpc('bob', 'msg-center', 'contact.update_contact', { did: zones.charlie.did, patch: { access_level: level }, contact_mgr_owner: mal.did }, mal.token);
  await malAccess('stranger');
  const stamp = Date.now();
  const text = `#689 N1 ${stamp}`;
  const lie = { from: zones.charlie.did, to: [mal.did], kind: 'chat', created_at_ms: stamp, nonce: stamp, content: { format: 'text/plain', content: text, refs: [{ role: 'input', target: { type: 'data_obj', obj_id: secret.id }, label: 'read me' }] } };
  // The account reaches msg-center with the next directory sync.
  let pushed;
  for (let attempt = 0; attempt < 30; attempt++) {
    pushed = await pushSigned('charlie', 'bob', mal.did, lie);
    if (pushed.status < 300) break;
    await sleep(3000);
  }
  check(pushed.status < 300, 'a Zone that lies gets its message delivered', `HTTP ${pushed.status} ${pushed.text().slice(0, 300)}`);
  const page = await rpc('bob', 'msg-center', 'msg.list_session', { owner: mal.did, session_id: `dm:${zones.charlie.did}`, limit: 20, descending: true, with_object: true }, mal.token);
  const received = page.items?.find(item => item.msg?.content?.content === text);
  check(received, 'the other account has the message');
  const named = received.msg_id;
  const stranger = await readAttachment('bob', mal.token, secret.id, named);
  check(stranger.status === 403, 'from a stranger: nothing', `HTTP ${stranger.status}`);
  await malAccess('allow');
  const read = await readAttachment('bob', mal.token, secret.id, named);
  check(read.status === 404 && !read.bytes.includes(secret.bytes.subarray(0, 16)), 'accepted, its Zone does not have it: not found, whatever is stored here', `HTTP ${read.status}`);
  const card = await describe('bob', mal.token, secret.id, named);
  check(card.status === 404 && !card.text().includes('bobs-private'), 'nor its name', `HTTP ${card.status}`);
  const resend = await rpcRaw('bob', 'msg-center', 'msg.post_send', {
    msg: { from: mal.did, to: [zones.charlie.did], kind: 'chat', created_at_ms: Date.now(), nonce: Date.now(), content: { format: 'text/plain', content: 'as if it were mine', refs: [{ role: 'input', target: { type: 'data_obj', obj_id: secret.id }, label: 'mine' }] } },
    idempotency_key: `n1-${stamp}`,
  }, mal.token);
  check(resend.error && !resend.result, 'and sending it on is refused', resend);
  log('n1', { refused: resend.error });
  // The owner sends the same file as before: this is about the message, not the file.
  const legit = await send('bob', 'charlie', [secret], tokens.bob);
  await awaitMessage('charlie', 'bob', legit.text, tokens.charlie);
  await setAccess('charlie', 'bob', 'allow', tokens.charlie);
  await verifiedRead('charlie', tokens.charlie, secret, legit.msgId);
} else {
  notVerified.push('N1 from a lying Zone (BUCKYOS_TEST_DEVICE_DIR not given)');
}

finish('Remote attachment DV');
