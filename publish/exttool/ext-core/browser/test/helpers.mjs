// What the tests share: running the commands, a real Chromium with a debugging port, a raw CDP client, a fake Agent
// Service (kRPC browser.*), and a small web site.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import zlib from 'node:zlib';

import { loadPlaywright } from '../runtime.mjs';

export const BROWSER_DIR = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
export const AGENT_BROWSER = path.join(BROWSER_DIR, 'agent-browser.mjs');
export const PAGE_RENDER = path.join(BROWSER_DIR, 'page-render.mjs');

// Resolved, because a command reports paths as it sees them and the temporary directory may be behind a symlink.
export function tempDir(name) {
  return fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), `${name}-`)));
}

// Runs `node script args` to its end: { code, stdout, stderr }.
export function node(script, args, { env = {}, cwd } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [script, ...args], { env: { ...process.env, ...env }, cwd, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => (stdout += chunk));
    child.stderr.on('data', (chunk) => (stderr += chunk));
    child.on('error', reject);
    child.on('close', (code) => resolve({ code, stdout, stderr }));
  });
}

export function listen(server) {
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server.address().port)));
}

export function close(server) {
  server.closeAllConnections();
  return new Promise((resolve) => server.close(resolve));
}

export function png(width, height, [red, green, blue]) {
  const row = Buffer.alloc(1 + width * 3);
  for (let x = 0; x < width; x++) row.set([red, green, blue], 1 + x * 3);
  const chunk = (type, data) => {
    const body = Buffer.concat([Buffer.from(type), data]);
    const frame = Buffer.alloc(8 + body.length);
    frame.writeUInt32BE(data.length, 0);
    body.copy(frame, 4);
    frame.writeUInt32BE(zlib.crc32(body), 4 + body.length);
    return frame;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;
  header[9] = 2;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', zlib.deflateSync(Buffer.concat(Array.from({ length: height }, () => row)))),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

export function pngSize(bytes) {
  return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) };
}

export function jpegSize(bytes) {
  for (let at = 2; at < bytes.length; at += 2 + bytes.readUInt16BE(at + 2)) {
    const marker = bytes[at + 1];
    if (marker >= 0xc0 && marker <= 0xc3) return { height: bytes.readUInt16BE(at + 5), width: bytes.readUInt16BE(at + 7) };
  }
  throw new Error('not a JPEG with a frame header');
}

// A raw DevTools connection to one tab.
export class Cdp {
  static async open(url) {
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => {
      socket.onopen = resolve;
      socket.onerror = () => reject(new Error(`cannot connect to ${url}`));
    });
    return new Cdp(socket);
  }

  constructor(socket) {
    this.socket = socket;
    this.nextId = 0;
    this.pending = new Map();
    this.events = [];
    socket.onmessage = (message) => {
      const data = JSON.parse(message.data);
      if (!data.id) return this.events.push(data.method);
      const { resolve, reject } = this.pending.get(data.id);
      this.pending.delete(data.id);
      return data.error ? reject(new Error(data.error.message)) : resolve(data.result);
    };
  }

  send(method, params = {}) {
    const id = ++this.nextId;
    this.socket.send(JSON.stringify({ id, method, params }));
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }

  async waitFor(method, ms = 5000) {
    for (const until = Date.now() + ms; Date.now() < until; await sleep(20)) {
      if (this.events.includes(method)) return;
    }
    throw new Error(`no ${method} within ${ms} ms`);
  }

  close() {
    this.socket.close();
  }
}

// A headless Chromium as the Agent Service would start it.
export async function startChromium() {
  const profile = tempDir('agent-browser-profile');
  const executable = process.env.OPENDAN_CHROMIUM ?? loadPlaywright().chromium.executablePath();
  // The Agent Service's two screens (opendan `browser/process.rs`): a viewer on a sharp screen gets its tab painted on the second.
  const args = ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`, '--no-first-run', '--no-default-browser-check', '--disable-dev-shm-usage', '--screen-info={0,0 800x600}{1600,0 1600x1200 devicePixelRatio=2}'];
  if (process.getuid?.() === 0) args.push('--no-sandbox');
  const child = spawn(executable, [...args, 'about:blank'], { stdio: 'ignore' });
  let port;
  for (let i = 0; i < 300 && !port; i++) {
    await sleep(100);
    try {
      port = fs.readFileSync(path.join(profile, 'DevToolsActivePort'), 'utf8').split('\n')[0];
    } catch {}
  }
  if (!port) {
    child.kill('SIGKILL');
    throw new Error(`${executable} did not open a debugging port`);
  }
  const cdpUrl = `http://127.0.0.1:${port}`;
  const targets = async () => (await (await fetch(`${cdpUrl}/json/list`)).json()).filter((target) => target.type === 'page');
  return {
    cdpUrl,
    targets,
    // In the last window, in front of that window's other tabs; resolves once the page has loaded.
    async newTab(url) {
      const target = await (await fetch(`${cdpUrl}/json/new?${encodeURIComponent(url)}`, { method: 'PUT' })).json();
      const cdp = await Cdp.open(target.webSocketDebuggerUrl);
      for (;;) {
        const { result } = await cdp.send('Runtime.evaluate', { expression: 'document.readyState + " " + location.href' });
        if (result.value === `complete ${url}`) break;
        await sleep(20);
      }
      cdp.close();
      return target;
    },
    closeTab: (id) => fetch(`${cdpUrl}/json/close/${id}`),
    async stop() {
      const exited = new Promise((resolve) => child.once('exit', resolve));
      try {
        const { webSocketDebuggerUrl } = await (await fetch(`${cdpUrl}/json/version`)).json();
        (await Cdp.open(webSocketDebuggerUrl)).send('Browser.close').catch(() => {});
      } catch {
        child.kill();
      }
      await Promise.race([exited, sleep(5000).then(() => child.kill('SIGKILL'))]);
      await exited;
      fs.rmSync(profile, { recursive: true, force: true, maxRetries: 5 });
    },
  };
}

// The browser.* methods of the protocol, over the real browser's tabs. Tests set `groups`, `sessions`, `ttlMs` and
// read `calls`.
export async function startAgentService(chromium) {
  const service = {
    groups: new Map(),
    sessions: new Map(),
    ttlMs: 60_000,
    calls: [],
    unavailable: null,
    nextLease: 0,
    group(id) {
      if (!this.groups.has(id)) {
        this.groups.set(id, { tabs: [], active: null, user_hold: false, agent_state: 'idle', note: null, handover: null, lease: null });
      }
      return this.groups.get(id);
    },
    reset() {
      this.groups.clear();
      this.sessions.clear();
      this.calls.length = 0;
      this.ttlMs = 60_000;
      this.unavailable = null;
    },
    revoke(groupId) {
      this.group(groupId).lease = null;
    },
  };

  const fail = (code, message) => Object.assign(new Error(message), { code });
  const view = async (groupId) => {
    const group = service.group(groupId);
    const live = new Map((await chromium.targets()).map((target) => [target.id, target]));
    group.tabs = group.tabs.filter((id) => live.has(id));
    if (!group.tabs.includes(group.active)) group.active = group.tabs[0] ?? null;
    return {
      group_id: groupId,
      controller: group.lease && !group.user_hold ? 'agent' : 'user',
      user_hold: group.user_hold,
      agent_state: group.handover ? 'waiting_takeover' : group.lease ? 'working' : group.agent_state,
      note: group.note,
      handover: group.handover,
      lease: group.lease && { session_id: group.lease.session_id, expires_at_ms: group.lease.expires_at_ms },
      tabs: group.tabs.map((id) => ({ tab_id: id, url: live.get(id).url, title: live.get(id).title, loading: false, active: id === group.active })),
    };
  };
  const status = async (groupId) => ({
    enabled: true,
    available: !service.unavailable,
    running: !service.unavailable,
    unavailable: service.unavailable,
    version: service.unavailable ? null : { browser: '159.0.8154.6', playwright: '1.65.1' },
    groups: await Promise.all((groupId ? [groupId] : [...service.groups.keys()]).map(view)),
  });
  const groupOf = (sessionId) => service.sessions.get(sessionId) ?? 'default';
  const leased = (leaseId) => {
    for (const [id, group] of service.groups) {
      if (group.lease?.lease_id === leaseId) return [id, group];
    }
    throw fail('lease_lost', 'the lease is gone');
  };

  const methods = {
    'browser.status': (params) => status(params.group_id ?? (params.session_id && groupOf(params.session_id))),
    'browser.ensure': () => status(),
    async 'browser.acquire'(params) {
      const groupId = groupOf(params.session_id);
      const group = service.group(groupId);
      if (group.user_hold) throw fail('user_in_control', 'the user has taken over this group');
      group.lease = {
        lease_id: `lease-${++service.nextLease}`,
        session_id: params.session_id,
        expires_at_ms: Date.now() + service.ttlMs,
        known: new Set((await chromium.targets()).map((target) => target.id)),
      };
      group.agent_state = 'idle';
      group.note = null;
      const { tabs } = await view(groupId);
      return { lease_id: group.lease.lease_id, group_id: groupId, ttl_ms: service.ttlMs, expires_at_ms: group.lease.expires_at_ms, cdp_url: chromium.cdpUrl, tabs, active_tab_id: group.active };
    },
    'browser.renew'(params) {
      const [, group] = leased(params.lease_id);
      group.lease.expires_at_ms = Date.now() + service.ttlMs;
      return { expires_at_ms: group.lease.expires_at_ms };
    },
    // Tabs opened while the lease was held join its group.
    async 'browser.release'(params) {
      const [, group] = leased(params.lease_id);
      for (const target of await chromium.targets()) {
        if (!group.lease.known.has(target.id)) group.tabs.push(target.id);
      }
      group.lease = null;
      group.agent_state = params.state ?? 'idle';
      group.note = params.note ?? null;
      return {};
    },
    'browser.request_handover'(params) {
      const groupId = groupOf(params.session_id);
      const group = service.group(groupId);
      group.lease = null;
      group.user_hold = true;
      group.handover = { reason: params.reason, note: params.note ?? null, at_ms: Date.now() };
      return { group_id: groupId };
    },
  };

  const server = http.createServer(async (request, response) => {
    let text = '';
    for await (const chunk of request) text += chunk;
    const { method, params, sys } = JSON.parse(text);
    service.calls.push({ method, params, token: sys[1] });
    let body;
    try {
      if (service.unavailable && method !== 'browser.status') throw fail('browser_unavailable', service.unavailable);
      if (!methods[method]) throw fail(undefined, `unknown method ${method}`);
      body = { result: await methods[method](params), sys: [sys[0]] };
    } catch (e) {
      // As opendan sends it: RPCErrors::ReasonError(error_to_wire(..)).
      const wire = { kind: 'other', message: e.message, at_ms: Date.now(), code: e.code };
      body = { error: `Failed due to reason: ${JSON.stringify(wire)}`, sys: [sys[0]] };
    }
    response.setHeader('content-type', 'application/json');
    response.end(JSON.stringify(body));
  });
  service.url = `http://127.0.0.1:${await listen(server)}/kapi/agent`;
  service.stop = () => close(server);
  return service;
}

// Pages for the tests, and named signals a script and a test hand each other over HTTP.
export async function startSite() {
  const signals = new Map();
  const signal = (name) => {
    if (!signals.has(name)) signals.set(name, Promise.withResolvers());
    return signals.get(name);
  };
  const pages = {
    '/form': `<!doctype html><html lang="zh"><meta charset="utf-8"><title>\u767b\u8bb0\u8868</title>
<h1>\u767b\u8bb0\u8868</h1>
<form onsubmit="event.preventDefault(); result.textContent = '\u5df2\u63d0\u4ea4\uff1a' + who.value + '\uff0c' + city.value">
  <label>\u59d3\u540d <input id="who"></label>
  <label>\u57ce\u5e02 <input id="city"></label>
  <button>\u63d0\u4ea4</button>
</form>
<p id="result"></p></html>`,
    '/article': `<!doctype html><html lang="zh"><meta charset="utf-8"><title>\u65e7\u7f51\u7ad9</title>
<h1 id="headline">\u539f\u6807\u9898</h1><img id="photo" src="/big.png">
<button id="more" onclick="headline.textContent = '\u70b9\u8fc7\u4e4b\u540e\u7684\u6807\u9898'">\u66f4\u591a</button></html>`,
    '/blank': '<!doctype html><title>blank</title>',
  };
  const picture = png(2000, 500, [200, 30, 30]);
  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url, 'http://site');
    const [, kind, name] = url.pathname.split('/');
    if (kind === 'signal') {
      signal(name).resolve();
      return response.end('ok');
    }
    if (kind === 'wait') {
      await signal(name).promise;
      return response.end('ok');
    }
    if (url.pathname === '/big.png') {
      response.setHeader('content-type', 'image/png');
      return response.end(picture);
    }
    if (!pages[url.pathname]) {
      response.statusCode = 404;
      return response.end('not found');
    }
    response.setHeader('content-type', 'text/html; charset=utf-8');
    return response.end(pages[url.pathname]);
  });
  const url = `http://127.0.0.1:${await listen(server)}`;
  return {
    url,
    received: (name) => signal(name).promise,
    send: (name) => signal(name).resolve(),
    clear: () => signals.clear(),
    stop: () => close(server),
  };
}
