const assert = require("node:assert/strict");
const { test } = require("node:test");
const { fakeClock, load } = require("./load.cjs");

const topology = { net_id: "nat", rtcp_port: 2980, support_container: true, sn_url: "sn", public_ips: [] };

async function ready(overrides = {}) {
  const lib = await load("src/domain.ts");
  const data = {
    use_self_domain: true,
    self_domain: "home.example.com",
    owner_document: { name: "alice" },
    domain: { ...lib.emptyDomainState(), mode: "sn", ...overrides },
  };
  const setup = lib.buildDomainSetup(data, topology);
  data.domain.check = { ready: true, items: [] };
  data.domain.check_key = lib.checkKey(setup, data.domain);
  return { lib, data };
}

test("a passed check counts only for the settings it was made with", async () => {
  const { lib, data } = await ready();
  const ok = (topo = topology) => lib.domainReady(data, lib.buildDomainSetup(data, topo));
  assert.equal(ok(), true);
  assert.equal(ok({ ...topology }), true, "the same route again keeps the result");
  assert.equal(ok({ ...topology, net_id: "wan_dyn" }), false);
  assert.equal(ok({ ...topology, net_id: "wan", public_ips: ["203.0.113.7"] }), false);
  data.domain.certificate = "upload";
  assert.equal(ok(), false);
  data.domain.certificate = "acme";
  assert.equal(ok(), true);
  data.self_domain = "other.example.com";
  assert.equal(ok(), false);
  data.self_domain = "home.example.com";
  data.domain.check = { ready: false, items: [] };
  assert.equal(ok(), false);
});

test("manual hosting always uses an uploaded certificate", async () => {
  const { lib, data } = await ready({ mode: "manual" });
  const setup = lib.buildDomainSetup(data, topology);
  assert.equal(setup.certificate, "upload");
  assert.equal(lib.domainReady(data, setup), false, "no certificate yet");
});

test("DNS API hosting needs its credential and warnings need a confirmation", async () => {
  const noCredential = await ready({ mode: "dns_api" });
  assert.equal(noCredential.lib.domainReady(noCredential.data, noCredential.lib.buildDomainSetup(noCredential.data, topology)), false);
  const { lib, data } = await ready({
    mode: "dns_api", credential_id: "c", certificate: "upload", certificate_id: "x",
    certificate_inspection: { items: [{ level: "warn" }], publicly_trusted: false },
  });
  const setup = lib.buildDomainSetup(data, topology);
  assert.equal(lib.domainReady(data, setup), false);
  data.domain.accept_warnings = true;
  assert.equal(lib.domainReady(data, setup), true);
  assert.equal(lib.certificateUntrusted(data), true);
  data.domain.certificate = "acme";
  assert.equal(lib.certificateUntrusted(data), false);
});

test("each record card shows its own part of the checks", async () => {
  const { lib } = await ready();
  const domain = "home.example.com";
  const items = [
    { id: "ownership", status: "pass" },
    { id: "delegation", status: "waiting" },
    { id: "entry", status: "fail", targets: [
      { host: domain, status: "pass" },
      { host: `x7f3.${domain}`, status: "waiting" },
      { host: `ood1.${domain}`, status: "fail" },
    ] },
  ];
  const status = (id, name = domain) => lib.recordStatus({ id, name }, domain, items);
  assert.equal(status("pkx"), "pass");
  assert.equal(status("ns"), "waiting");
  assert.equal(status("entry"), "pass");
  assert.equal(status("wildcard"), "waiting");
  assert.equal(lib.recordStatus({ id: "wildcard" }, domain, []), null);
  assert.equal(lib.recordStatus({ id: "wildcard" }, domain, [{ id: "entry", status: "waiting", targets: [] }]), "waiting");
  const other = [{ id: "entry", status: "fail", targets: [{ host: `public.${domain}`, status: "fail" }] }];
  assert.equal(lib.recordStatus({ id: "entry" }, domain, other), "pass", "only the older public record is wrong");
});

test("names are shown the way DNS consoles want them", async () => {
  const { lib } = await ready();
  assert.equal(lib.relativeName("_pkx.home.example.com", "example.com"), "_pkx.home");
  assert.equal(lib.relativeName("example.com", "example.com"), "@");
  assert.equal(lib.relativeName("home.example.com", "example.org"), null);
});

test("only public addresses are accepted for a fixed IP", async () => {
  const { lib } = await ready();
  for (const ip of ["203.0.113.7", "8.8.8.8", "172.32.0.1"]) assert.equal(lib.isPublicIPv4(ip), true, ip);
  for (const ip of ["10.0.0.1", "192.168.1.2", "172.16.0.1", "100.64.0.1", "127.0.0.1", "169.254.1.1", "224.0.0.1", "1.2.3", "1.2.3.256", "01.2.3.4", "a.b.c.d", ""]) {
    assert.equal(lib.isPublicIPv4(ip), false, ip);
  }
  for (const ip of ["2001:db8::1", "2606:4700::1111"]) assert.equal(lib.isPublicIPv6(ip), true, ip);
  for (const ip of ["fe80::1", "fd00::1", "::1", "1.2.3.4", "2001:::1", "xyz"]) assert.equal(lib.isPublicIPv6(ip), false, ip);
});

test("node errors are recognized behind the RPC prefix", async () => {
  const { lib } = await ready();
  const error = new Error('Failed due to reason: [active:domain_not_ready] [{"id":"entry","status":"fail"}]');
  const parsed = lib.activeErrorOf(error);
  assert.equal(parsed.code, "domain_not_ready");
  assert.equal(lib.checkItemsOf(parsed.detail)[0].id, "entry");
  assert.equal(lib.checkItemsOf('{"items":[{"id":"caa"}]}')[0].id, "caa");
  assert.equal(lib.checkItemsOf('[{"id":"ownership"}] (request 7)')[0].id, "ownership");
  assert.equal(lib.checkItemsOf("not json").length, 0);
  assert.equal(lib.activeErrorOf(new Error("network down")), null);
});

async function poller(options = {}) {
  const { createPoller } = await load("src/poller.ts");
  const clock = fakeClock();
  const log = { runs: [], results: [], errors: 0, open: 0, overlap: false, hidden: false };
  const pending = [];
  const instance = createPoller({
    intervalMs: 10_000,
    timers: clock.timers,
    hidden: () => log.hidden,
    finished: (result) => result === "ready",
    run: () => {
      log.runs.push(clock.now());
      log.open++;
      if (log.open > 1) log.overlap = true;
      const done = new Promise((resolve, reject) => pending.push({ resolve, reject }));
      return done.finally(() => log.open--);
    },
    onResult: (result) => log.results.push(result),
    onError: () => log.errors++,
    ...options,
  });
  const answer = async (value) => {
    pending.shift().resolve(value);
    await clock.settle();
  };
  return { instance, clock, log, answer, pending };
}

test("domain checks run one at a time, ten seconds after the last answer", async () => {
  const { instance, clock, log, answer } = await poller();
  instance.restart();
  await clock.advance(0);
  assert.equal(log.runs.length, 1);
  await clock.advance(60_000);
  assert.equal(log.runs.length, 1, "no second request while one is open");
  await answer("waiting");
  await clock.advance(10_000);
  assert.deepEqual(log.runs, [0, 70_000]);
  await answer("ready");
  await clock.advance(60_000);
  assert.equal(log.runs.length, 2, "everything passed");
});

test("changed settings discard the open answer and check again at once", async () => {
  const { instance, clock, log, answer } = await poller();
  instance.restart();
  await clock.advance(0);
  instance.restart();
  await clock.advance(5_000);
  assert.equal(log.runs.length, 1);
  await answer("ready");
  await clock.advance(0);
  assert.deepEqual(log.results, []);
  assert.equal(log.runs.length, 2);
  assert.equal(log.overlap, false);
});

test("a hidden page pauses and is checked the moment it is shown", async () => {
  const { instance, clock, log, answer } = await poller();
  instance.restart();
  await clock.advance(0);
  await answer("waiting");
  log.hidden = true;
  await clock.advance(60_000);
  assert.equal(log.runs.length, 1);
  log.hidden = false;
  instance.visible();
  await clock.advance(0);
  assert.equal(log.runs.length, 2);
});

test("a failed request is tried again, a stopped poller is not", async () => {
  const { instance, clock, log, pending } = await poller();
  instance.restart();
  await clock.advance(0);
  pending.shift().reject(new Error("network"));
  await clock.settle();
  assert.equal(log.errors, 1);
  await clock.advance(10_000);
  assert.equal(log.runs.length, 2);
  instance.stop();
  pending.shift().resolve("waiting");
  await clock.advance(60_000);
  assert.equal(log.runs.length, 2);
  assert.deepEqual(log.results, []);
});
