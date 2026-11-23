const assert = require("node:assert/strict");
const { test } = require("node:test");
const { fakeClock, load } = require("./load.cjs");

const SECOND = 1000;
const MINUTE = 60 * SECOND;

async function monitor({ probe, cause = "service", trusted = true, activatedAt = 0, startAt = 0 } = {}) {
  const { createStartupMonitor } = await load("src/startup.ts");
  const clock = fakeClock(startAt);
  const states = [];
  const probes = [];
  const control = { ok: false, hang: false, cause };
  const instance = createStartupMonitor({
    activatedAt,
    certificateTrusted: trusted,
    now: clock.now,
    timers: clock.timers,
    probe: () => {
      probes.push(clock.now());
      if (probe) return probe(clock, probes.length);
      return control.hang ? new Promise(() => {}) : Promise.resolve(control.ok);
    },
    diagnose: async () => control.cause,
    onChange: (state) => states.push({ ...state }),
  });
  instance.start();
  await clock.settle();
  return { clock, states, probes, control, instance, last: () => states.at(-1) };
}

test("nothing is requested in the first two minutes, then every five seconds", async () => {
  const { clock, probes, last } = await monitor();
  await clock.advance(2 * MINUTE - 1);
  assert.equal(probes.length, 0);
  assert.equal(last().phase, "starting");
  await clock.advance(1);
  assert.deepEqual(probes, [2 * MINUTE]);
  assert.equal(last().phase, "checking");
  await clock.advance(10 * SECOND);
  assert.deepEqual(probes, [120, 125, 130].map((s) => s * SECOND));
});

test("the first success ends the checks", async () => {
  const { clock, probes, control, last } = await monitor();
  await clock.advance(2 * MINUTE + 12 * SECOND);
  control.ok = true;
  await clock.advance(5 * SECOND);
  assert.deepEqual(last(), { phase: "ready" });
  const count = probes.length;
  await clock.advance(10 * MINUTE);
  assert.equal(probes.length, count);
});

test("a rejected probe is not a success", async () => {
  const { clock, last } = await monitor({ probe: () => Promise.reject(new TypeError("Failed to fetch")) });
  await clock.advance(3 * MINUTE);
  assert.equal(last().phase, "checking");
});

for (const cause of ["node", "dns", "service"]) {
  test(`five minutes without success is a failure with its cause (${cause})`, async () => {
    const { clock, last } = await monitor({ cause });
    await clock.advance(5 * MINUTE - 1);
    assert.equal(last().phase, "checking");
    await clock.advance(1);
    assert.deepEqual(last(), { phase: "failed", cause });
  });
}

test("a probe that takes its whole timeout does not overlap and still ends the schedule", async () => {
  const { clock, probes, last } = await monitor({
    probe: (clock) => new Promise((resolve) => clock.timers.set(() => resolve(false), 8 * SECOND)),
  });
  await clock.advance(5 * MINUTE);
  assert.equal(last().phase, "checking");
  for (let i = 1; i < probes.length; i++) assert(probes[i] - probes[i - 1] >= 8 * SECOND);
  await clock.advance(8 * SECOND);
  assert.equal(last().phase, "failed");
});

test("checking again after a failure can turn into success", async () => {
  const { clock, control, instance, last, probes } = await monitor({ cause: "node" });
  await clock.advance(5 * MINUTE);
  control.cause = "dns";
  await instance.recheck();
  assert.deepEqual(last(), { phase: "failed", cause: "dns" });
  control.ok = true;
  const count = probes.length;
  await instance.recheck();
  assert.equal(probes.length, count + 1);
  assert.deepEqual(last(), { phase: "ready" });
  await instance.recheck();
  assert.equal(probes.length, count + 1);
});

test("an untrusted certificate is never reported as a failure", async () => {
  const first = await monitor({ trusted: false, cause: "node" });
  await first.clock.advance(5 * MINUTE);
  assert.deepEqual(first.last(), { phase: "unverifiable", nodeUp: false });
  const second = await monitor({ trusted: false, cause: "service" });
  await second.clock.advance(5 * MINUTE);
  assert.deepEqual(second.last(), { phase: "unverifiable", nodeUp: true });
  second.control.ok = true;
  await second.instance.recheck();
  assert.equal(second.last().phase, "ready");
});

test("an untrusted certificate the browser trusts succeeds as usual", async () => {
  const { clock, control, last } = await monitor({ trusted: false });
  control.ok = true;
  await clock.advance(2 * MINUTE);
  assert.equal(last().phase, "ready");
});

test("the schedule follows the clock when timers are held back", async () => {
  const { clock, probes, instance, last } = await monitor();
  clock.jump(10 * MINUTE);
  assert.equal(probes.length, 0);
  instance.wake();
  await clock.advance(0);
  assert.equal(probes.length, 1, "one probe before giving up");
  assert.equal(last().phase, "failed");
});

test("the clock starts at the activation, not when the page is shown", async () => {
  const { clock, probes, last } = await monitor({ activatedAt: 0, startAt: 150 * SECOND });
  await clock.advance(0);
  assert.equal(probes.length, 1);
  assert.equal(last().phase, "checking");
});

test("a stopped monitor neither probes nor reports", async () => {
  const { clock, probes, instance, states } = await monitor();
  instance.stop();
  await clock.advance(10 * MINUTE);
  assert.equal(probes.length, 0);
  assert.equal(states.length, 1);
});

const HOST = "home.example.com";
const DID = `did:web:${HOST}`;

async function probes(routes) {
  const requests = [];
  const api = await load("src/startupProbe.ts", {
    globals: {
      AbortController,
      setTimeout,
      clearTimeout,
      fetch: async (url, init) => {
        requests.push({ url, init });
        const answer = routes(url);
        if (!answer) throw new TypeError("Failed to fetch");
        return { status: answer.status ?? 200, ok: (answer.status ?? 200) < 300, json: async () => answer.body };
      },
    },
  });
  return { ...api, requests };
}

test("only a readable 200 of public.<domain>/readme counts", async () => {
  for (const [answer, expected] of [[{ status: 200 }, true], [{ status: 404 }, false], [{ status: 503 }, false], [null, false]]) {
    const { probeReadme, requests } = await probes(() => answer);
    assert.equal(await probeReadme(HOST), expected);
    const { url, init } = requests[0];
    assert.equal(url, `https://public.${HOST}/readme`);
    assert.equal(init.mode, "cors");
    assert.equal(init.cache, "no-store");
    assert.equal(init.redirect, "error");
    assert.equal(init.headers, undefined);
  }
});

const nodeUp = { body: { id: DID } };
const jwtNode = { body: { didDocument: `h.${btoa(JSON.stringify({ id: DID })).replace(/=+$/, "")}.s` } };
const route = ({ node, dns }) => (url) =>
  url.includes("/1.0/identifiers/") ? node : url.startsWith("https://dns.google/") ? dns : null;

test("the three layers are judged in order", async () => {
  const resolved = { body: { Answer: [{ type: 5 }, { type: 1 }] } };
  for (const [answers, expected] of [
    [{ node: null, dns: null }, "node"],
    [{ node: { status: 503, body: {} }, dns: resolved }, "node"],
    [{ node: { body: { id: "did:web:other.example.com" } }, dns: resolved }, "node"],
    [{ node: nodeUp, dns: { body: { Answer: [{ type: 5 }] } } }, "dns"],
    [{ node: nodeUp, dns: { body: {} } }, "dns"],
    [{ node: nodeUp, dns: resolved }, "service"],
    [{ node: jwtNode, dns: resolved }, "service"],
    [{ node: nodeUp, dns: null }, "service"],
  ]) {
    const { diagnoseStartup, requests } = await probes(route(answers));
    assert.equal(await diagnoseStartup("192.168.1.10", DID, HOST), expected);
    assert(requests[0].url.startsWith("http://192.168.1.10/1.0/identifiers/"));
    assert.equal(requests[0].init.headers, undefined);
  }
});
