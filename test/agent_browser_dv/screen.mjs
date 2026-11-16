#!/usr/bin/env node
// The owner at the screen of the Agent's browser (Agent Browser Protocol §3): what the Agent Browser app does when
// the user looks at a thing, taps and types. Runs a list of actions in order and prints what the service says.
//
//   node screen.mjs [--viewport 1280x800] [--mobile] [--out <dir>] <action>...   (the Agent's one tab group, `default`)
//
//   nav=<url>        go to an address, wait until it is loaded
//   tap=<x>,<y>      tap (touch on a phone viewport, a mouse click on a desktop one)
//   text=<text>      type text into what has the focus
//   key=<Key>        press a key (DOM key name: Enter, Tab, Backspace)
//   wait=<ms>
//   shot=<name>      save the newest frame as <out>/<name>.jpg (and .png when AB_DV_MAGICK names ImageMagick's magick)
//   close-tabs       close every tab of the browser
//   blocked          tap once and expect the service to refuse it with agent_in_control (exit code 1 otherwise)
//   watch=<seconds>  just look: print state changes and count frames
//   probe=<seconds>  look, and tap an empty corner of the page twice a second: counts the taps the service refused
//                    (agent_in_control) while the Agent had control, and keeps the frames seen then as probe-<n>.jpg
//
// The default viewport is the desktop window the browser starts with, so that looking does not reload the page.
import { execFileSync } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { AGENT_ID, ZONE, ownerToken, sleep } from './lib.mjs'

const argv = process.argv.slice(2)
const flags = { group: 'default', viewport: '1280x800', mobile: false, out: '.' }
const actions = []
for (let i = 0; i < argv.length; i++) {
  if (argv[i] === '--viewport') flags.viewport = argv[++i]
  else if (argv[i] === '--mobile') flags.mobile = true
  else if (argv[i] === '--out') flags.out = argv[++i]
  else actions.push(argv[i])
}
const [width, height] = flags.viewport.split('x').map(Number)
mkdirSync(flags.out, { recursive: true })

const ws = new WebSocket(`wss://${ZONE}/kapi/${AGENT_ID}/browser/screen`)
ws.binaryType = 'arraybuffer'
const say = (message) => ws.send(JSON.stringify(message))
let meta = null
let frame = null
let frames = 0
let state = null
const errors = []
let quiet = false
let ready
const whenReady = new Promise((resolve, reject) => { ready = { resolve, reject } })
const brief = (s) => JSON.stringify({ group: s.group.group_id, controller: s.group.controller, user_hold: s.group.user_hold, agent_state: s.group.agent_state, tabs: s.group.tabs.length, url: s.url, title: s.title, loading: s.loading })
let lastBrief = ''

ws.onopen = async () => say({ type: 'hello', token: await ownerToken(), group_id: flags.group, viewport: { width, height, mobile: flags.mobile } })
ws.onerror = (event) => console.error('socket error', event.message ?? '')
ws.onclose = (event) => { console.log(`closed (${event.code}${event.reason ? ` ${event.reason}` : ''})`); ready.reject(new Error('closed')) }
ws.onmessage = (event) => {
  if (typeof event.data !== 'string') {
    frame = Buffer.from(event.data)
    frames += 1
    if (meta) say({ type: 'ack', seq: meta.seq })
    return
  }
  const message = JSON.parse(event.data)
  if (message.type === 'frame_meta') { meta = message; return }
  if (message.type === 'ready' || message.type === 'state') {
    state = message.type === 'ready' ? message.state : message
    const line = brief(state)
    if (line !== lastBrief) { console.log(`< ${message.type} ${line}`); lastBrief = line }
    if (message.type === 'ready') ready.resolve()
    return
  }
  if (message.type === 'pong') return
  if (message.type === 'error') { errors.push(message); if (quiet) return }
  if (message.type === 'focus' && quiet) return
  if (message.type === 'request' && message.kind === 'dialog') setTimeout(() => say({ type: 'respond', request_id: message.request_id, result: { accept: true } }), 300)
  console.log('<', JSON.stringify(message))
}
const ping = setInterval(() => ws.readyState === WebSocket.OPEN && say({ type: 'ping' }), 10_000)

function tap(x, y) {
  if (flags.mobile) {
    say({ type: 'input', kind: 'touch', phase: 'start', points: [{ id: 0, x, y }] })
    say({ type: 'input', kind: 'touch', phase: 'end', points: [] })
  } else {
    say({ type: 'input', kind: 'mouse', action: 'move', x, y, button: 'none', click_count: 0, modifiers: 0 })
    say({ type: 'input', kind: 'mouse', action: 'down', x, y, button: 'left', click_count: 1, modifiers: 0 })
    say({ type: 'input', kind: 'mouse', action: 'up', x, y, button: 'left', click_count: 1, modifiers: 0 })
  }
}

async function loaded(timeoutMs = 20_000) {
  const until = Date.now() + timeoutMs
  await sleep(400)
  while (Date.now() < until) {
    if (state && state.url && state.url !== 'about:blank' && !state.loading) return
    await sleep(200)
  }
  throw new Error('the page did not finish loading')
}

let code = 0
try {
  await whenReady
  for (const action of actions) {
    const [name, value = ''] = action.split(/=(.*)/s)
    console.log(`> ${name === 'text' ? 'text=<…>' : action}`)
    if (name === 'nav') { say({ type: 'nav', action: 'url', url: value }); await loaded() }
    else if (name === 'tap') { const [x, y] = value.split(',').map(Number); tap(x, y); await sleep(500) }
    else if (name === 'text') { say({ type: 'input', kind: 'text', text: value }); await sleep(400) }
    else if (name === 'key') {
      say({ type: 'input', kind: 'key', action: 'down', key: value, code: value, modifiers: 0, text: '' })
      say({ type: 'input', kind: 'key', action: 'up', key: value, code: value, modifiers: 0, text: '' })
      await sleep(600)
    }
    else if (name === 'wait') await sleep(Number(value))
    else if (name === 'watch') await sleep(Number(value) * 1000)
    else if (name === 'probe') {
      const until = Date.now() + Number(value) * 1000
      let taps = 0
      let agentTaps = 0
      let kept = 0
      let lastFrame = null
      const refusedBefore = errors.filter((error) => error.code === 'agent_in_control').length
      quiet = true
      while (Date.now() < until) {
        const agentHas = state.group.controller === 'agent'
        tap(3, 3)
        taps += 1
        if (agentHas) agentTaps += 1
        if (agentHas && frame && frame !== lastFrame && kept < 40) {
          lastFrame = frame
          kept += 1
          const file = join(flags.out, `probe-${String(kept).padStart(2, '0')}.jpg`)
          writeFileSync(file, frame)
          if (process.env.AB_DV_MAGICK) execFileSync(process.env.AB_DV_MAGICK, [file, file.replace(/\.jpg$/, '.png')])
        }
        await sleep(500)
      }
      quiet = false
      const refused = errors.filter((error) => error.code === 'agent_in_control').length - refusedBefore
      console.log(`probe: ${taps} taps, ${agentTaps} while the Agent had control, ${refused} refused with agent_in_control (the service reports at most one a second), ${kept} frame(s) kept from then`)
    }
    else if (name === 'shot') {
      await sleep(800)
      if (!frame) { console.log('no frame yet'); code = 1; continue }
      const file = join(flags.out, `${value}.jpg`)
      writeFileSync(file, frame)
      if (process.env.AB_DV_MAGICK) execFileSync(process.env.AB_DV_MAGICK, [file, file.replace(/\.jpg$/, '.png')])
      console.log(`saved ${file} (${meta?.width}x${meta?.height}, ${frame.length} bytes, frame ${frames})`)
    }
    else if (name === 'close-tabs') {
      for (const tab of state.group.tabs) say({ type: 'tab', action: 'close', tab_id: tab.tab_id })
      await sleep(800)
    }
    else if (name === 'blocked') {
      const before = errors.length
      tap(10, 10)
      await sleep(1500)
      const refused = errors.slice(before).some((error) => error.code === 'agent_in_control')
      console.log(refused ? 'the tap was refused: agent_in_control' : `the tap was NOT refused (controller ${state.group.controller})`)
      if (!refused) code = 1
    }
    else { console.error(`unknown action ${action}`); code = 2 }
  }
} catch (error) {
  console.error(String(error.message))
  code = 1
}
console.log(`frames ${frames}`)
clearInterval(ping)
try { ws.close() } catch {}
setTimeout(() => process.exit(code), 150)
