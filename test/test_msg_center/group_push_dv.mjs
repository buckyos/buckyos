import { createReadStream } from 'node:fs';
import { stat } from 'node:fs/promises';
import { createInterface } from 'node:readline';
import { login, rpc, sleep, zones } from '../nfs_server_test/attachment_dv_common.mjs';

const host = process.env.BUCKYOS_TEST_GROUP_HOST ?? 'bob';
if (!['alice', 'bob'].includes(host)) throw new Error('BUCKYOS_TEST_GROUP_HOST must be alice or bob');
const member = host === 'bob' ? 'alice' : 'bob';
const logOffset = process.env.BUCKYOS_TEST_LOG_OFFSET ?? 'Z';
if (!/^(Z|[+-](?:0[0-9]|1[0-4]):[0-5][0-9])$/.test(logOffset)) {
  throw new Error('BUCKYOS_TEST_LOG_OFFSET must be Z or a numeric timezone offset such as +08:00');
}
const count = Number(process.env.BUCKYOS_TEST_SAMPLES ?? 20);
if (!Number.isSafeInteger(count) || count < 2) throw new Error('BUCKYOS_TEST_SAMPLES must be an integer >= 2');
const [hostToken, memberToken] = await Promise.all([login(host), login(member)]);
const stamp = Date.now();
const group = process.env.BUCKYOS_TEST_JOINED_GROUP ?? (await rpc(host, 'msg-center', 'group.create', {
  idempotency_key: `push737-${stamp}`, configuration: { profile: { name: `#737 ${stamp}` } },
}, hostToken)).group_did;
if (!process.env.BUCKYOS_TEST_JOINED_GROUP) {
  const invitation = await rpc(host, 'msg-center', 'group.invite_member', {
    group_did: group, member_did: zones[member].did,
  }, hostToken);
  const deadline = Date.now() + 180_000;
  for (;;) {
    try {
      const joined = await rpc(member, 'msg-center', 'group.accept_invitation', {
        group_did: group, invitation_id: invitation.invite_id,
      }, memberToken);
      if (joined.state === 'active' && !joined.sync_pending) break;
      if (Date.now() >= deadline) throw new Error(`initial sync pending for ${group}`);
    } catch (err) {
      if (Date.now() >= deadline) throw err;
    }
    await sleep(3000);
  }
}
async function logPosition(path) {
  return path ? { path, start: (await stat(path)).size } : null;
}

async function countLogLines(position, classify, counters, startMs, endMs) {
  if (!position) return null;
  const year = new Date(startMs).getUTCFullYear();
  const result = { ...counters };
  const lines = createInterface({ input: createReadStream(position.path, { start: position.start }), crlfDelay: Infinity });
  for await (const line of lines) {
    const label = classify(line);
    if (!label) continue;
    const fields = line.match(/^(\d{2}-\d{2}) (\d{2}:\d{2}:\d{2}\.\d{3}) /);
    if (!fields) throw new Error(`missing msg-center log timestamp in ${position.path}`);
    const at = [year - 1, year, year + 1].some(y => {
      const ms = Date.parse(`${y}-${fields[1]}T${fields[2]}${logOffset}`);
      return ms >= startMs && ms <= endMs;
    });
    if (at) result[label]++;
  }
  return result;
}

// Let the invitation notification and its first cursor acknowledgement finish before sampling.
await sleep(1500);
const hostLog = await logPosition(process.env.BUCKYOS_TEST_HOST_LOG);
const memberLog = await logPosition(process.env.BUCKYOS_TEST_MEMBER_LOG);
const arrivals = [];
let firstAccepted;
let lastProjected;
let lastPost = 0;
for (let i = 0; i < count; i++) {
  const spacing = lastPost + 550 - Date.now();
  if (spacing > 0) await sleep(spacing);
  const at = Date.now();
  lastPost = at;
  const result = await rpc(host, 'msg-center', 'msg.post_send', {
    idempotency_key: `push737-${stamp}-${i}`,
    msg: { from: zones[host].did, to: [group], kind: 'group_msg', created_at_ms: at, nonce: at,
      content: { format: 'text/plain', content: `#737 ${stamp} sample ${i + 1}` } },
  }, hostToken);
  if (!result.ok) throw new Error(`host refused sample ${i + 1}: ${result.reason}`);
  const deadline = Date.now() + 60000;
  for (;;) {
    const page = await rpc(member, 'msg-center', 'msg.peek_box', {
      mailbox: `${zones[member].did}/${group}`, box_kind: 'INBOX', limit: 50, with_object: false,
    }, memberToken);
    const record = page.find(({ record }) => record.msg_id === result.msg_id)?.record;
    if (record) {
      firstAccepted ??= record.sort_key;
      lastProjected = record.created_at_ms;
      const latency = lastProjected - record.sort_key;
      if (!Number.isSafeInteger(latency) || latency < 0) throw new Error('invalid latency; verify both OOD clocks');
      console.log(JSON.stringify({ group, sample: i + 1, id: result.msg_id, host_accepted_ms: record.sort_key,
        member_projected_ms: record.created_at_ms, latency_ms: latency }));
      arrivals.push(latency);
      break;
    }
    if (Date.now() >= deadline) throw new Error(`sample ${i + 1} missing from member INBOX after 60 s`);
    await sleep(100);
  }
}
arrivals.sort((a, b) => a - b);
const percentile = p => arrivals[Math.ceil(p * arrivals.length) - 1];
const p95 = percentile(0.95);
const path = `/kapi/msg-center/groups/${group}`;
const hostCounters = { Rtcp: 0, Https: 0 };
const memberCounters = { fast_commits: 0, changes: 0, sessions: 0, objects: 0 };
const transport = line => {
  if (!line.includes(`POST ${path}/updates: 200`)) return null;
  return line.match(/cross-zone (Rtcp|Https) /)?.[1];
};
const readerEvent = line => {
  if (line.includes(`joined group ${group} fast_commits=`)) return 'fast_commits';
  for (const endpoint of ['changes', 'sessions', 'objects']) {
    const url = endpoint === 'objects' ? `${path}/objects/` : `${path}/${endpoint}`;
    if (line.includes(`GET ${url}${endpoint === 'objects' ? '' : ':'}`)
        || (line.includes(` ${url}`) && line.includes(' failed:'))) return endpoint;
  }
  return null;
};
const windowEnd = lastProjected + 1000;
let transports = null;
let memberStats = null;
if (hostLog || memberLog) {
  await sleep(1500);
  const drainDeadline = Date.now() + 5000;
  for (;;) {
    [transports, memberStats] = await Promise.all([
      countLogLines(hostLog, transport, hostCounters, firstAccepted, windowEnd),
      countLogLines(memberLog, readerEvent, memberCounters, firstAccepted, windowEnd),
    ]);
    if ((!transports || transports.Rtcp + transports.Https >= count)
        && (!memberStats || memberStats.fast_commits >= count)) break;
    if (Date.now() >= drainDeadline) break;
    await sleep(250);
  }
}
console.log(JSON.stringify({ host, member, group, samples: count, p50_ms: percentile(0.5), p95_ms: p95, target_ms: 3000,
  first_host_accepted_ms: firstAccepted, last_member_projected_ms: lastProjected, log_window_end_ms: windowEnd,
  transports, member_stats: memberStats }));
if (p95 > 3000 || (transports && transports.Rtcp + transports.Https < count)
    || (memberStats && (memberStats.fast_commits !== count || memberStats.changes || memberStats.sessions || memberStats.objects))) {
  process.exitCode = 1;
}
