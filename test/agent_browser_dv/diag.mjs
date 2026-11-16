#!/usr/bin/env node
// The attributable comparison of notepads/agent-runtime-browser-compatibility-todo.md §4.1 on the
// diagnostic page (diag_site.mjs): every condition starts a new browser process with a new profile,
// the page reports what it sees to the site, and report.md lists what differs from the native baseline A.
//
//   node diag.mjs --out <dir> [--chromium <exe>] [--conditions A,A2,B,C,D-ua,...] [--display :N | --xvfb :N]
//                 [--site http://127.0.0.1:<port>] [--wait 40000]
//
// A and B are never connected to: their pages report by themselves, the popup is opened by a click of the
// X server (xdotool) when one is there. C–E connect to the debugging endpoint and say so in their phases.
// Nothing here signs in anywhere; no password, cookie or token is read or kept.
import { spawn, spawnSync, execFileSync } from 'node:child_process'
import { mkdirSync, writeFileSync, readFileSync, existsSync, readdirSync, rmSync } from 'node:fs'
import { join, dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createServer } from 'node:net'
import os from 'node:os'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '../..')
const argv = process.argv.slice(2)
const opt = (name, fallback) => { const i = argv.indexOf(name); return i >= 0 ? argv[i + 1] : fallback }
const OUT = resolve(opt('--out', join(os.tmpdir(), `ab-diag-${Date.now()}`)))
const CHROMIUM = opt('--chromium', process.env.OPENDAN_CHROMIUM || '/opt/buckyos/tools/bin/chromium')
const WAIT = Number(opt('--wait', '40000'))
const NAVIGATE_AFTER = Number(opt('--navigate-after', '9000'))
const XVFB = opt('--xvfb', null)
const DISPLAY = XVFB || opt('--display', process.env.DISPLAY || null)
const ROOT = process.getuid?.() === 0
const WINDOW = { width: 1280, height: 800 }
mkdirSync(OUT, { recursive: true })

const ALL = ['A', 'A2', 'B', 'C', 'D-ua', 'D-mobile', 'D-inject', 'D-focus', 'D-screencast', 'D-permissions', 'D-playwright', 'H', 'E']
const CONDITIONS = (opt('--conditions', ALL.join(','))).split(',').map((c) => c.trim()).filter(Boolean)

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const log = (...a) => console.log('[diag]', ...a)
const has = (cmd) => spawnSync('sh', ['-c', `command -v ${cmd}`]).status === 0
const freePort = () => new Promise((ok) => { const s = createServer(); s.listen(0, '127.0.0.1', () => { const { port } = s.address(); s.close(() => ok(port)) }) })

// ---------------------------------------------------------------- the site and the display

async function startSite() {
  const given = opt('--site', null)
  if (given) return { base: given, stop: () => {} }
  const child = spawn(process.execPath, [join(HERE, 'site.mjs'), '0', join(OUT, 'site')], { stdio: ['ignore', 'pipe', 'inherit'] })
  const line = await new Promise((ok) => child.stdout.once('data', (d) => ok(String(d))))
  const port = /:(\d+)\//.exec(line)[1]
  return { base: `http://127.0.0.1:${port}`, stop: () => child.kill() }
}

function startXvfb() {
  if (!XVFB) return null
  const child = spawn('Xvfb', [XVFB, '-screen', '0', `${WINDOW.width}x${WINDOW.height}x24`, '-nolisten', 'tcp'], { stdio: 'ignore', detached: true })
  return child
}

// ---------------------------------------------------------------- the browser

// The arguments of the Agent's browser today (src/frame/opendan/src/browser/process.rs `arguments`).
const productionArgs = (profile) => ['--headless=new', `--user-data-dir=${profile}`, `--disk-cache-dir=${profile}-cache`, '--remote-debugging-port=0', '--no-first-run', '--no-default-browser-check', '--disable-dev-shm-usage', '--no-startup-window', '--window-size=1280,800', '--screen-info={0,0 800x600}{1600,0 1600x1200 devicePixelRatio=2}', '--disable-blink-features=CaretBlinking']

function chromiumVersion() {
  try { return execFileSync(CHROMIUM, ['--version'], { encoding: 'utf8', env: { ...process.env } }).trim() } catch (e) { return `unknown (${e.message})` }
}
const majorOf = (version) => (/(\d+)\.\d+\.\d+\.\d+/.exec(version) || [])[1]
function desktopUA(version) {
  return `Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${majorOf(version)}.0.0.0 Safari/537.36`
}

function launch(args, display) {
  const env = { ...process.env }
  if (display) env.DISPLAY = display; else delete env.DISPLAY
  const child = spawn(CHROMIUM, args, { env, stdio: ['ignore', 'ignore', 'pipe'], detached: true })
  let stderr = ''
  child.stderr.on('data', (d) => { stderr = (stderr + d).slice(-20000) })
  return { child, stderr: () => stderr, started: Date.now() }
}

function groupStats(pgid) {
  let rssKiB = 0, cpuTicks = 0, processes = 0
  for (const pid of readdirSync('/proc').filter((p) => /^\d+$/.test(p))) {
    try {
      const stat = readFileSync(`/proc/${pid}/stat`, 'utf8')
      const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ')
      if (Number(fields[2]) !== pgid) continue
      processes++
      cpuTicks += Number(fields[11]) + Number(fields[12])
      rssKiB += Number(/^Pss:\s+(\d+)/m.exec(readFileSync(`/proc/${pid}/smaps_rollup`, 'utf8'))?.[1] ?? 0)
    } catch (_) { /* gone */ }
  }
  return { processes, pssMiB: Math.round(rssKiB / 1024), cpuSeconds: cpuTicks / 100 }
}

async function stop(b) {
  try { process.kill(-b.child.pid, 'SIGTERM') } catch (_) {}
  for (let i = 0; i < 40 && b.child.exitCode === null && b.child.signalCode === null; i++) await sleep(100)
  try { process.kill(-b.child.pid, 'SIGKILL') } catch (_) {}
}

// ---------------------------------------------------------------- a minimal DevTools client

async function devtoolsEndpoint(profile, port) {
  for (let i = 0; i < 100; i++) {
    try {
      if (port) return (await (await fetch(`http://127.0.0.1:${port}/json/version`)).json()).webSocketDebuggerUrl
      const [p, path] = readFileSync(join(profile, 'DevToolsActivePort'), 'utf8').split('\n')
      return `ws://127.0.0.1:${p.trim()}${path.trim()}`
    } catch (_) { await sleep(100) }
  }
  throw new Error('no debugging endpoint')
}

async function cdpConnect(url) {
  const ws = new WebSocket(url)
  await new Promise((ok, fail) => { ws.onopen = ok; ws.onerror = () => fail(new Error('cdp connect')) })
  let id = 0
  const waiting = new Map()
  const listeners = []
  const sent = []
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data)
    if (msg.id && waiting.has(msg.id)) { const w = waiting.get(msg.id); waiting.delete(msg.id); msg.error ? w.fail(new Error(msg.error.message)) : w.ok(msg.result) }
    else if (msg.method) for (const l of listeners) l(msg)
  }
  const send = (method, params = {}, sessionId) => new Promise((ok, fail) => {
    const n = ++id
    sent.push(sessionId ? `${method} (tab)` : method)
    waiting.set(n, { ok, fail })
    ws.send(JSON.stringify({ id: n, method, params, ...(sessionId ? { sessionId } : {}) }))
    setTimeout(() => { if (waiting.has(n)) { waiting.delete(n); fail(new Error(`${method}: timeout`)) } }, 10000)
  })
  return { send, on: (f) => listeners.push(f), close: () => ws.close(), sent }
}

async function pageTarget(cdp, base) {
  for (let i = 0; i < 50; i++) {
    const { targetInfos } = await cdp.send('Target.getTargets')
    const page = targetInfos.find((t) => t.type === 'page' && t.url.startsWith(`${base}/diag`) && !t.url.includes('/diag/popup'))
    if (page) return page.targetId
    await sleep(200)
  }
  throw new Error('the diagnostic tab is not there')
}

// The commands the Agent's browser sends to every tab it attaches to (mod.rs `attach`, requests.rs `instrument`).
const PAGE_SCRIPT = readFileSync(join(REPO, 'src/frame/opendan/src/browser/screen_page.js'), 'utf8')
const PERMISSION_SCRIPT = readFileSync(join(REPO, 'src/frame/opendan/src/browser/screen_permissions.js'), 'utf8')
const REFUSED = ['geolocation', 'notifications', 'camera', 'microphone', 'midi', 'clipboard-read', 'idle-detection', 'window-management', 'local-fonts', 'nfc', 'storage-access', 'top-level-storage-access', 'display-capture', 'payment-handler', 'captured-surface-control', 'speaker-selection']
const tryAll = async (cdp, list, session) => { for (const [m, p] of list) await cdp.send(m, p, session).catch(() => {}) }
const ATTACH = [['Page.enable', {}], ['Page.setLifecycleEventsEnabled', { enabled: true }], ['Runtime.enable', {}]]
const ITEMS = {
  inject: async (cdp, s) => tryAll(cdp, [['Runtime.addBinding', { name: '__opendanScreen', executionContextName: 'opendan-screen' }], ['Page.addScriptToEvaluateOnNewDocument', { source: PAGE_SCRIPT, worldName: 'opendan-screen' }], ['Page.addScriptToEvaluateOnNewDocument', { source: PERMISSION_SCRIPT }], ['Page.setInterceptFileChooserDialog', { enabled: true }]], s),
  focus: async (cdp, s) => tryAll(cdp, [['Emulation.setFocusEmulationEnabled', { enabled: true }]], s),
  permissions: async (cdp) => { for (const name of REFUSED) await cdp.send('Browser.setPermission', { permission: { name }, setting: 'denied' }).catch(() => {}) },
  mobile: async (cdp, s, version) => {
    const full = (/(\d+\.\d+\.\d+\.\d+)/.exec(version) || [])[1]; const major = majorOf(version)
    await tryAll(cdp, [
      ['Emulation.setDeviceMetricsOverride', { width: 390, height: 760, deviceScaleFactor: 2, mobile: true, screenWidth: 390, screenHeight: 760 }],
      ['Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 }],
      ['Emulation.setUserAgentOverride', { userAgent: `Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${major}.0.0.0 Mobile Safari/537.36`, userAgentMetadata: { brands: [{ brand: 'Chromium', version: major }, { brand: 'Google Chrome', version: major }, { brand: 'Not_A Brand', version: '99' }], fullVersion: full, platform: 'Android', platformVersion: '10', architecture: '', model: 'K', mobile: true } }],
    ], s)
  },
  desktop: async (cdp, s) => tryAll(cdp, [['Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false }], ['Emulation.setTouchEmulationEnabled', { enabled: false, maxTouchPoints: 5 }], ['Emulation.setUserAgentOverride', { userAgent: '' }]], s),
  screencast: async (cdp, s) => {
    cdp.on((m) => { if (m.method === 'Page.screencastFrame' && m.sessionId === s) cdp.send('Page.screencastFrameAck', { sessionId: m.params.sessionId }, s).catch(() => {}) })
    await cdp.send('Page.startScreencast', { format: 'jpeg', quality: 70, everyNthFrame: 1 }, s).catch(() => {})
  },
}

// ---------------------------------------------------------------- one condition

async function results(site, runId) { return (await fetch(`${site.base}/diag/results?run=${encodeURIComponent(runId)}`)).json() }
async function setPhase(site, runId, phase) { await fetch(`${site.base}/diag/phase?run=${encodeURIComponent(runId)}&phase=${encodeURIComponent(phase)}`, { method: 'POST' }) }
async function waitFor(site, runId, test, ms = WAIT) {
  const until = Date.now() + ms
  while (Date.now() < until) { const r = await results(site, runId); if (test(r)) return r; await sleep(300) }
  return results(site, runId)
}
const got = (where, phase) => (r) => r.some((x) => x.where === where && (!phase || x.phase === phase))

function xClick(display, x, y) {
  if (!display || !has('xdotool')) return 'no xdotool'
  const r = spawnSync('xdotool', ['mousemove', String(x), String(y), 'click', '1'], { env: { ...process.env, DISPLAY: display } })
  return r.status === 0 ? 'clicked' : `xdotool failed: ${r.stderr}`
}
function xShot(display, file) {
  if (!display || !has('xwd')) return null
  const r = spawnSync('sh', ['-c', `xwd -root -silent -display ${display} > '${file}.xwd'`])
  return r.status === 0 ? `${file}.xwd` : null
}

async function runCondition(name, site, version) {
  const runId = `${name}-${Date.now()}`
  const profile = join(OUT, 'profiles', name)
  rmSync(profile, { recursive: true, force: true }); rmSync(`${profile}-cache`, { recursive: true, force: true })
  mkdirSync(profile, { recursive: true })
  const url = `${site.base}/diag?run=${encodeURIComponent(runId)}&navigate_after=${NAVIGATE_AFTER}`
  // H: headless with nothing else (no debugging port, no override); E: the Agent's browser as it is started today.
  const headless = name === 'E' || name === 'H'
  const display = headless ? null : DISPLAY
  if (!headless && !display) return { name, skipped: 'no X display (--display or --xvfb)' }
  const port = ['A', 'A2', 'H'].includes(name) ? null : await freePort()
  const base = [`--user-data-dir=${profile}`, '--no-first-run', '--no-default-browser-check', '--window-position=0,0', `--window-size=${WINDOW.width},${WINDOW.height}`]
  if (ROOT) base.push('--no-sandbox')
  let args
  if (name === 'H') args = ['--headless=new', ...base, url]
  else if (headless) args = [...productionArgs(profile).map((a) => a === '--remote-debugging-port=0' ? `--remote-debugging-port=${port}` : a), ...(ROOT ? ['--no-sandbox'] : []), `--user-agent=${desktopUA(version)}`]
  else args = [...base, ...(port ? [`--remote-debugging-port=${port}`] : []), ...(name === 'D-ua' ? [`--user-agent=${desktopUA(version)}`] : []), url]
  log(`${name}: starting (${headless ? 'headless' : `headed on ${display}`}${port ? `, debugging port ${port}` : ', no debugging port'})`)
  const b = launch(args, display)
  const record = { name, runId, args: args.map((a) => a.replace(OUT, '<out>')), display: display ?? 'none', profile: 'new', phases: [], cdp_commands: [] }
  let cdp = null
  try {
    let session = null
    if (name === 'E') {
      cdp = await cdpConnect(await devtoolsEndpoint(profile, port))
      await cdp.send('Target.setDiscoverTargets', { discover: true })
      await cdp.send('Browser.setDownloadBehavior', { behavior: 'allowAndName', downloadPath: join(OUT, 'downloads'), eventsEnabled: true }).catch(() => {})
      await ITEMS.permissions(cdp)
      const { targetId } = await cdp.send('Target.createTarget', { url: 'about:blank', newWindow: true })
      session = (await cdp.send('Target.attachToTarget', { targetId, flatten: true })).sessionId
      await tryAll(cdp, [...ATTACH, ['Emulation.setFocusEmulationEnabled', { enabled: true }]], session)
      await ITEMS.inject(cdp, session)
      await ITEMS.desktop(cdp, session)
      await ITEMS.screencast(cdp, session)
      await cdp.send('Page.navigate', { url }, session)
    }
    const first = await waitFor(site, runId, (r) => got('top', 'load')(r) && got('frame', 'load')(r))
    record.first_report_ms = (first.find((x) => x.where === 'top')?.at ?? Date.now()) - b.started
    record.phases.push('load')

    if (name === 'C') {
      cdp = await cdpConnect(await devtoolsEndpoint(profile, port))
      await setPhase(site, runId, 'c1-connected'); record.phases.push('c1-connected'); await waitFor(site, runId, got('top', 'c1-connected'), 8000)
      await cdp.send('Target.setDiscoverTargets', { discover: true })
      await setPhase(site, runId, 'c2-discover'); record.phases.push('c2-discover'); await waitFor(site, runId, got('top', 'c2-discover'), 8000)
      session = (await cdp.send('Target.attachToTarget', { targetId: await pageTarget(cdp, site.base), flatten: true })).sessionId
      await setPhase(site, runId, 'c3-attached'); record.phases.push('c3-attached'); await waitFor(site, runId, got('top', 'c3-attached'), 8000)
      await tryAll(cdp, ATTACH, session)
      await setPhase(site, runId, 'c4-enabled'); record.phases.push('c4-enabled'); await waitFor(site, runId, got('top', 'c4-enabled'), 8000)
    } else if (name.startsWith('D-') && name !== 'D-ua') {
      const item = name.slice(2)
      if (item === 'playwright') {
        const pw = await import(process.env.AB_DIAG_PLAYWRIGHT || '/opt/buckyos/tools/store/ext-core/playwright/node_modules/playwright-core/index.mjs')
        const browser = await pw.chromium.connectOverCDP(`http://127.0.0.1:${port}`)
        record.playwright = browser.version()
        await setPhase(site, runId, 'd-applied'); record.phases.push('d-applied'); await waitFor(site, runId, got('top', 'd-applied'), 8000)
        const page = browser.contexts()[0]?.pages().find((p) => p.url().includes('/diag'))
        if (page) await page.reload().catch(() => {})
        await setPhase(site, runId, 'd-reloaded'); record.phases.push('d-reloaded'); await waitFor(site, runId, got('top', 'd-reloaded'), 12000)
      } else {
        cdp = await cdpConnect(await devtoolsEndpoint(profile, port))
        await cdp.send('Target.setDiscoverTargets', { discover: true })
        session = (await cdp.send('Target.attachToTarget', { targetId: await pageTarget(cdp, site.base), flatten: true })).sessionId
        await tryAll(cdp, ATTACH, session)
        await ITEMS[item](cdp, session, version)
        await setPhase(site, runId, 'd-applied'); record.phases.push('d-applied'); await waitFor(site, runId, got('top', 'd-applied'), 8000)
        // What applies to new documents only shows after a reload.
        await cdp.send('Page.reload', {}, session).catch(() => {})
        await setPhase(site, runId, 'd-reloaded'); record.phases.push('d-reloaded'); await waitFor(site, runId, got('top', 'd-reloaded'), 12000)
      }
    }

    // The popup: a click of the X server on the page's button, or of the debugging protocol when there is no X server.
    if (headless && session) {
      for (const type of ['mousePressed', 'mouseReleased']) await cdp.send('Input.dispatchMouseEvent', { type, x: 100, y: 50, button: 'left', clickCount: 1 }, session).catch(() => {})
      record.popup_click = 'cdp'
    } else {
      // Where the page's button is on the X screen, from what the page reported of its window (no protocol involved).
      const w = (await results(site, runId)).filter((x) => x.where === 'top').at(-1)?.window
      const left = w ? w.screenX + Math.max(0, (w.outerWidth - w.innerWidth) / 2) : 0
      const top = w ? w.screenY + Math.max(0, w.outerHeight - w.innerHeight) : 120
      record.popup_click = xClick(display, Math.round(left + 110), Math.round(top + 60))
    }
    await waitFor(site, runId, got('popup'), 8000)
    record.shot = xShot(display, join(OUT, `${name}`))
    if (!['C'].includes(name) && !name.startsWith('D-')) await waitFor(site, runId, got('top-after-navigation'), NAVIGATE_AFTER + 10000)
    record.stats = groupStats(b.child.pid)
  } catch (e) {
    record.error = String(e?.stack || e)
  } finally {
    if (cdp) { record.cdp_commands = cdp.sent; cdp.close() }
    await stop(b)
    record.stderr_tail = b.stderr().split('\n').filter(Boolean).slice(-8)
  }
  record.reports = await results(site, runId)
  writeFileSync(join(OUT, `${name}.json`), JSON.stringify(record, null, 2))
  log(`${name}: ${record.reports.length} reports${record.error ? `, error: ${record.error.split('\n')[0]}` : ''}`)
  return record
}

// ---------------------------------------------------------------- the comparison

const VOLATILE = /^(at|href|phase|where|report_headers|page_headers\.sec-fetch|window\.historyLength|navigator\.userActivation|navigator\.connection\.(downlink|rtt)|focus\.)/
function flat(value, prefix = '', out = {}) {
  if (value && typeof value === 'object' && !Array.isArray(value)) for (const [k, v] of Object.entries(value)) flat(v, prefix ? `${prefix}.${k}` : k, out)
  else out[prefix] = JSON.stringify(value)
  return out
}
const pick = (rec, where, phase) => rec?.reports?.filter((r) => r.where === where && (!phase || r.phase === phase)).at(-1)
function diff(a, b, all = false) {
  const fa = flat(a), fb = flat(b), keys = new Set([...Object.keys(fa), ...Object.keys(fb)])
  return [...keys].filter((k) => (all || !VOLATILE.test(k)) && fa[k] !== fb[k]).sort().map((k) => [k, fa[k], fb[k]])
}
const cell = (v) => (v === undefined ? '—' : String(v)).replace(/\|/g, '\\|').slice(0, 160)

function report(records, version) {
  const A = records.find((r) => r.name === 'A' && !r.skipped)
  const lines = [`# Browser diagnostics`, '', `- browser: \`${version}\` (\`${CHROMIUM}\`)`, `- host: ${os.platform()} ${os.arch()} ${os.release()}, uid ${process.getuid?.()}, display ${DISPLAY ?? 'none'}${XVFB ? ' (Xvfb started by the runner)' : ''}`, `- generated: ${new Date().toISOString()}`, '', '## Conditions', '', '| condition | display | debugging | CDP commands sent | first report ms | processes / PSS MiB / CPU s | reports | error |', '| --- | --- | --- | --- | --- | --- | --- | --- |']
  for (const r of records) {
    if (r.skipped) { lines.push(`| ${r.name} | — | — | — | — | — | — | skipped: ${r.skipped} |`); continue }
    const dbg = r.args.find((a) => a.startsWith('--remote-debugging-port')) ?? 'none'
    const cmds = [...new Set(r.cdp_commands)].join(', ') || 'none'
    lines.push(`| ${r.name} | ${r.display} | ${dbg} | ${cell(cmds)} | ${r.first_report_ms ?? '—'} | ${r.stats ? `${r.stats.processes} / ${r.stats.pssMiB} / ${r.stats.cpuSeconds}` : '—'} | ${r.reports.length} (${[...new Set(r.reports.map((x) => `${x.where}:${x.phase}`))].join(' ')}) | ${cell(r.error?.split('\n')[0] ?? '')} |`)
  }
  if (!A) { lines.push('', 'No baseline A: nothing to compare.'); return lines.join('\n') }
  const baseTop = pick(A, 'top', 'load')
  lines.push('', '## Baseline A (top page, at load)', '', '```json', JSON.stringify({ navigator: baseTop?.navigator, uaData: baseTop?.uaData, page_headers: baseTop?.page_headers, report_headers: baseTop?.report_headers }, null, 1), '```')
  for (const r of records.filter((x) => x !== A && !x.skipped)) {
    lines.push('', `## ${r.name} against A`, '')
    for (const where of ['top', 'frame', 'popup', 'top-after-navigation']) {
      const phases = [...new Set(r.reports.filter((x) => x.where === where).map((x) => x.phase))]
      for (const phase of phases) {
        const a = pick(A, where, 'load') ?? pick(A, where), b = pick(r, where, phase)
        if (!a || !b) continue
        const d = diff(a, b)
        lines.push(`### ${where}, phase ${phase}: ${d.length} differences`, '')
        if (d.length) { lines.push('| field | A | this |', '| --- | --- | --- |'); for (const [k, x, y] of d) lines.push(`| ${k} | ${cell(x)} | ${cell(y)} |`); lines.push('') }
      }
    }
  }
  // Within A: the frame, the popup and the navigated page against the top page.
  lines.push('', '## A: other documents against its top page', '')
  for (const where of ['frame', 'popup', 'top-after-navigation']) {
    const b = pick(A, where)
    if (!b) { lines.push(`- ${where}: no report`); continue }
    const d = diff(baseTop, b).filter(([k]) => !/^(window|media_queries|focus|permissions|storage|devices)\./.test(k))
    lines.push(`- ${where}: ${d.length ? d.map(([k, x, y]) => `${k} ${cell(x)} → ${cell(y)}`).join('; ') : 'same identity'}`)
  }
  return lines.join('\n')
}

// ---------------------------------------------------------------- main

const xvfb = startXvfb()
if (xvfb) await sleep(800)
const site = await startSite()
const version = chromiumVersion()
log(`browser ${version}; out ${OUT}; site ${site.base}`)
const records = []
try {
  for (const name of CONDITIONS) records.push(await runCondition(name, site, version))
} finally {
  if (xvfb) { const s = groupStats(xvfb.pid); records.xvfb = s; try { process.kill(-xvfb.pid, 'SIGTERM') } catch (_) {} }
  site.stop()
}
const md = report(records, version)
writeFileSync(join(OUT, 'report.md'), md + (xvfb ? `\n\nXvfb at the end: ${JSON.stringify(records.xvfb)}\n` : '\n'))
log(`report: ${join(OUT, 'report.md')}`)
