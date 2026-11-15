#!/usr/bin/env node
// Step commands of the Agent browser DV (see README.md). Everything acts as the owner, through the Zone gateway.
//
//   node dv.mjs token                               print the owner's session token (renews it when it is old)
//   node dv.mjs krpc <service> <method> ['<json>']  one kRPC as the owner
//   node dv.mjs status [group]                      browser.status of the Agent
//   node dv.mjs new "<title>"                       a new conversation with the Agent; prints its id
//   node dv.mjs send <session> "<text>" | @file     a message of the owner in that conversation (costs tokens)
//   node dv.mjs wait <session> [seconds] [state]    until the conversation is idle, or its group's agent_state is <state>
//   node dv.mjs messages <session>                  the conversation as the owner sees it
//   node dv.mjs sessions <session>                  the UI session of the conversation and the work it started
//   node dv.mjs tree <session>                      the work the conversation started: state, rounds, tokens, one line
//   node dv.mjs worklog <sid> [--full]              a session's committed worklog
//   node dv.mjs usage <sid>                         rounds and tokens of a session
//   node dv.mjs grep <session> <string>             looks for a string in the conversation, the UI session's and its work's worklogs
//   node dv.mjs take-over|hand-back|hand-to-agent <group>
//   node dv.mjs take-over-when-working <group> [seconds]   take over the moment the Agent holds a lease on the group
//   node dv.mjs reflow <url> [out dir]              what the reflow library has for <url> (browser.reflow_state), then opens
//                                                   its page on reflow.<zone> as the owner (prepare_app_launch, the session
//                                                   cookie, GET) and, for a package's page, its results/<page>.json
//   node dv.mjs bucket [prefix]                     the keys of the owner's reflow bucket
//   node dv.mjs stop <sid> [turn|session]           session.stop
//   node dv.mjs archive <session>                   ui.rotate of the UI session, then msg.archive_session
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { OWNER, ZONE, agent, archiveSession, browserStatus, call, conversation, messages, newSession, ownerToken, send, waitIdle, worklog } from './lib.mjs'

const [command, ...rest] = process.argv.slice(2)
const print = (value) => console.log(typeof value === 'string' ? value : JSON.stringify(value, null, 2))
const clip = (text, n) => (text.length > n ? `${text.slice(0, n)} …(+${text.length - n})` : text)

async function tree(sessionId) {
  const view = await conversation(sessionId)
  const ui = [...(view.generations ?? [])].map((item) => item.session_id)
  const sids = []
  for (const sid of ui) {
    sids.push(sid)
    const walk = async (parents) => {
      if (!parents.length) return
      const children = await agent('sessions.children_of', { parents })
      const ids = children.map((entry) => entry.sid ?? entry.session_id ?? entry.id).filter(Boolean)
      sids.push(...ids)
      await walk(ids)
    }
    await walk([sid])
  }
  return { view, sids }
}

function entryLine(entry, full) {
  const { seq, kind, type, ts, at_ms, ...others } = entry
  const text = JSON.stringify(others)
  return `#${seq ?? ''} ${kind ?? type ?? ''} ${full ? text : clip(text, 600)}`
}

switch (command) {
  case 'token': print(await ownerToken()); break
  case 'krpc': print(await call(rest[0], rest[1], JSON.parse(rest[2] ?? '{}'))); break
  case 'status': print(await browserStatus(rest[0] ? { group_id: rest[0] } : {})); break
  case 'new': print(await newSession(rest[0])); break
  case 'send': {
    const text = rest[1].startsWith('@') ? readFileSync(rest[1].slice(1), 'utf8').trim() : rest[1]
    print(`sent ${await send(rest[0], text)} at ${new Date().toISOString()}`)
    break
  }
  case 'wait': {
    const want = rest[2]
    const result = await waitIdle(rest[0], { timeoutMs: Number(rest[1] ?? 900) * 1000, until: want ? ({ group }) => group?.group_id === rest[0] && group.agent_state === want : undefined })
    print(`=> ${result.reason}`)
    process.exitCode = result.reason === 'timeout' ? 1 : 0
    break
  }
  case 'messages':
    for (const item of await messages(rest[0])) print(`--- ${new Date(item.at).toISOString()} ${item.direction} ${item.from}\n${item.text}`)
    break
  case 'sessions': {
    const { view, sids } = await tree(rest[0])
    print(view)
    for (const sid of sids) {
      const entry = await agent('sessions.lookup', { sid })
      print(`${sid}: ${JSON.stringify(entry)}`)
    }
    break
  }
  case 'tree': {
    const { view } = await tree(rest[0])
    const total = { rounds: 0, input_tokens: 0, output_tokens: 0 }
    const show = (card, indent) => {
      const usage = card.usage ?? {}
      for (const key of Object.keys(total)) total[key] += usage[key] ?? 0
      print(`${' '.repeat(indent)}${card.session_id} [${card.shape}] ${card.state} rounds=${usage.rounds ?? 0} in=${usage.input_tokens ?? 0} out=${usage.output_tokens ?? 0} | ${card.title} | ${clip(card.line ?? '', 160)}`)
      for (const todo of card.todos?.items ?? []) {
        print(`${' '.repeat(indent + 2)}todo ${todo.todo_id} ${todo.status} (${todo.mode}) ${todo.title}`)
        if (todo.card) show(todo.card, indent + 4)
      }
      for (const child of card.children ?? []) show(child, indent + 2)
    }
    for (const generation of view.generations ?? []) {
      const works = await agent('session.work_tree', { sid: generation.session_id })
      print(`${generation.session_id}: complete=${works.complete}`)
      for (const card of works.sessions ?? []) show(card, 2)
    }
    print(`work sessions in total: ${JSON.stringify(total)}`)
    break
  }
  case 'worklog': {
    const full = rest.includes('--full')
    for (const entry of await worklog(rest[0])) print(entryLine(entry, full))
    break
  }
  case 'usage': {
    const read = await agent('session.read', { sid: rest[0], report: false, worklog: 1 })
    print({ statistics: read.statistics, runs: read.runs?.length })
    break
  }
  case 'grep': {
    const needle = rest[1]
    let hits = 0
    for (const item of await messages(rest[0])) if (JSON.stringify(item.raw).includes(needle)) { hits += 1; print(`conversation: ${clip(item.text, 200)}`) }
    const { sids } = await tree(rest[0])
    for (const sid of sids) {
      for (const entry of await worklog(sid)) if (JSON.stringify(entry).includes(needle)) { hits += 1; print(`${sid} worklog ${entryLine(entry, false)}`) }
    }
    print(`${hits} hit(s) for the string in ${sids.length} session(s) [${sids.join(', ')}] and the conversation`)
    process.exitCode = hits ? 3 : 0
    break
  }
  case 'take-over-when-working': {
    const until = Date.now() + Number(rest[1] ?? 600) * 1000
    let seen = ''
    while (Date.now() < until) {
      const status = await browserStatus({ group_id: rest[0] })
      const group = status.groups[0]
      const line = `${group.controller}/${group.agent_state} lease=${group.lease ? group.lease.session_id : '-'} tabs=${group.tabs.map((tab) => tab.url).join(',')}`
      if (line !== seen) { print(`${new Date().toISOString()} ${line}`); seen = line }
      if (group.lease) {
        const taken = await agent('browser.take_over', { group_id: rest[0] })
        print(`${new Date().toISOString()} took over while ${group.lease.session_id} held the lease: ${JSON.stringify({ controller: taken.controller, user_hold: taken.user_hold, agent_state: taken.agent_state, lease: taken.lease })}`)
        process.exit(0)
      }
      await new Promise((resolve) => setTimeout(resolve, 120))
    }
    print('the Agent never held a lease')
    process.exitCode = 1
    break
  }
  case 'take-over': print(await agent('browser.take_over', { group_id: rest[0] })); break
  case 'hand-back': print(await agent('browser.hand_back', { group_id: rest[0] })); break
  case 'hand-to-agent': print(await agent('browser.hand_to_agent', { group_id: rest[0] })); break
  case 'reflow': {
    const state = await agent('browser.reflow_state', { url: rest[0] })
    print(state)
    if (!state.view) { print('nothing to show for this address'); process.exitCode = 1; break }
    const host = `https://reflow.${ZONE}`
    const page = `${host}/${state.view}`
    const launch = await call('control-panel', 'auth.prepare_app_launch', { service_id: 'nfs-server', redirect_url: page })
    if (launch.status !== 'ready') { print(launch); process.exitCode = 1; break }
    const anonymous = await fetch(page, { redirect: 'manual' })
    print(`without a session: HTTP ${anonymous.status}`)
    const callback = await fetch(launch.callback_url, { redirect: 'manual' })
    const cookie = callback.headers.getSetCookie().map((line) => line.split(';')[0]).join('; ')
    print(`sso_callback: HTTP ${callback.status} -> ${callback.headers.get('location')} (${cookie ? 'session cookie set' : 'NO cookie'})`)
    const get = async (address) => {
      const response = await fetch(address, { headers: { cookie } })
      const text = await response.text()
      print(`GET ${address}\n  HTTP ${response.status} ${response.headers.get('content-type')} ${text.length} characters`)
      return text
    }
    const html = await get(page)
    const files = { 'view.html': html }
    if (state.result && state.package) files['result.json'] = await get(`${host}/${state.site}/results/${state.page}.json`)
    if (rest[1]) {
      mkdirSync(rest[1], { recursive: true })
      for (const [name, text] of Object.entries(files)) writeFileSync(join(rest[1], name), text)
    }
    print(clip(html, 1500))
    break
  }
  case 'bucket': {
    const bucket = `user/control-panel/${OWNER}/reflow`
    const listed = await call('nfs-server', 'bucket.list_objects', { bucket_id: bucket, ...(rest[0] ? { prefix: rest[0] } : {}) })
    for (const item of listed.items ?? []) print(`${item.key}  ${item.size} bytes  ${item.content_type}  by ${item.created_by}  at ${new Date(item.created_at).toISOString()}`)
    print(`${(listed.items ?? []).length} key(s) in ${bucket}${rest[0] ? ` under ${rest[0]}` : ''}`)
    break
  }
  case 'stop': print(await agent('session.stop', { sid: rest[0], scope: rest[1] ?? 'turn', reason: 'agent browser DV' })); break
  case 'archive': {
    const { current } = await conversation(rest[0])
    if (current) print(await agent('ui.rotate', { sid: current.session_id }))
    print(await archiveSession(rest[0]))
    break
  }
  default:
    console.error('usage: see the top of dv.mjs')
    process.exit(2)
}
