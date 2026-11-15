// Shared by the Agent browser DV scripts: owner login, kRPC through the Zone gateway, the owner's conversation
// with the Agent (msg-center), and what the Agent Service tells about its sessions and its browser.
import { createHash, randomUUID } from 'node:crypto'
import { existsSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

process.env.NODE_TLS_REJECT_UNAUTHORIZED = '0'   // the DV Zone has a self-signed certificate
process.removeAllListeners('warning')

export const ZONE = process.env.AB_DV_ZONE ?? 'test.buckyos.io'
export const OWNER = process.env.AB_DV_OWNER ?? 'devtest'
export const PASSWORD = process.env.AB_DV_PASSWORD ?? 'bucky2025'
export const AGENT_ID = process.env.AB_DV_AGENT ?? `agenty.${ZONE}`
export const OUT = process.env.AB_DV_OUT ?? '.'
export const origin = `https://${ZONE}`

const passwordHash = (user, password, nonce) => {
  const first = createHash('sha256').update(`${password}${user}.buckyos`, 'utf8').digest('base64')
  return createHash('sha256').update(`${first}${nonce}`, 'utf8').digest('base64')
}

export async function rawCall(service, method, params, token) {
  const seq = Date.now()
  const response = await fetch(`${origin}/kapi/${service}`, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ method, params, sys: token ? [seq, token] : [seq] }),
  })
  const text = await response.text()
  let body
  try { body = JSON.parse(text) } catch { throw new Error(`${method}: HTTP ${response.status}: ${text.slice(0, 300)}`) }
  if (body.error) { const error = new Error(`${method}: ${body.error}`); error.wire = body.error; throw error }
  return body.result
}

/** The owner's session token: kept in <out>/owner.token and renewed after 10 minutes (it lasts about 15). */
export async function ownerToken(user = OWNER, password = PASSWORD) {
  const file = join(OUT, `${user}.token`)
  if (existsSync(file) && Date.now() - statSync(file).mtimeMs < 10 * 60_000) return readFileSync(file, 'utf8').trim()
  const nonce = Date.now()
  const result = await rawCall('control-panel', 'auth.login', { username: user, password: passwordHash(user, password, nonce), target: { kind: 'system', service_id: 'control-panel' }, login_nonce: nonce })
  writeFileSync(file, result.session_token, { mode: 0o600 })
  return result.session_token
}

export const call = async (service, method, params = {}) => rawCall(service, method, params, await ownerToken())
export const agent = (method, params = {}) => call(AGENT_ID, method, params)

let ids
export async function identities() {
  if (!ids) {
    const user = await call('control-panel', 'user.get', { user_id: OWNER })
    const info = await agent('agent.info')
    ids = { ownerDid: user.local_profile?.did ?? user.profile?.did ?? user.did_document.id, agentDid: info.agent_did }
  }
  return ids
}

/** A new named conversation of the owner with the Agent: one "thing" of the Agent Browser app. */
export async function newSession(title) {
  const { ownerDid, agentDid } = await identities()
  const state = await call('msg-center', 'msg.create_session', { owner: ownerDid, peer_did: agentDid, title, binding: { kind: 'native', targetDid: agentDid }, origin: 'manual' })
  return state.session_id
}

export async function send(sessionId, text) {
  const { ownerDid, agentDid } = await identities()
  const msg = { from: ownerDid, to: [agentDid], kind: 'chat', to_session: sessionId, created_at_ms: Date.now(), nonce: Math.floor(Math.random() * 2 ** 32), content: { format: 'text/plain', content: text } }
  const result = await call('msg-center', 'msg.post_send', { msg, idempotency_key: randomUUID() })
  if (!result?.ok) throw new Error(`msg.post_send refused: ${JSON.stringify(result)}`)
  return result.msg_id
}

export async function messages(sessionId) {
  const { ownerDid } = await identities()
  const page = await call('msg-center', 'msg.list_session', { owner: ownerDid, session_id: sessionId, limit: 500, descending: false, with_object: true })
  return (page?.items ?? []).map((item) => ({ at: item.msg?.created_at_ms ?? item.sort_key, from: item.from ?? item.msg?.from, direction: item.direction, text: item.msg?.content?.content ?? '', raw: item }))
}

export async function archiveSession(sessionId) {
  const { ownerDid } = await identities()
  return call('msg-center', 'msg.archive_session', { owner: ownerDid, session_id: sessionId })
}

/** The UI session of a conversation (`ui.conversation`): `current` is null before the first message arrives. */
export async function conversation(sessionId) {
  const { ownerDid } = await identities()
  return agent('ui.conversation', { peer_did: ownerDid, session_id: sessionId })
}

/** Every committed worklog entry of a session, oldest first. */
export async function worklog(sid) {
  const pages = []
  let before
  for (;;) {
    const page = await agent('session.worklog', { sid, limit: 200, ...(before === undefined ? {} : { before }) })
    pages.unshift(page.entries ?? [])
    if (page.next_before === null || page.next_before === undefined) break
    before = page.next_before
  }
  return pages.flat()
}

export const browserStatus = (params = {}) => agent('browser.status', params)
export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

/** Waits until the conversation has no open Turn and no unfinished work, or `timeoutMs` passed. Prints what changes.
 * A message just sent takes a moment to open its Turn: idle only counts once the conversation was seen busy, or
 * stayed idle for `graceMs`. */
export async function waitIdle(sessionId, { timeoutMs = 15 * 60_000, settleMs = 6000, graceMs = 90_000, log = console.log, until } = {}) {
  const started = Date.now()
  let last = ''
  let idleSince = null
  let wasBusy = false
  while (Date.now() - started < timeoutMs) {
    const { current } = await conversation(sessionId)
    const status = await browserStatus()
    const group = status.groups.find((item) => item.group_id === sessionId) ?? status.groups.find((item) => item.group_id === 'default')
    const line = current
      ? `${current.session_id} run=${current.run_state} turn=${current.open_turn?.turn ?? '-'} works=${current.unfinished_works ?? 0} needs_input=${current.needs_input} | ${current.one_line_status ?? ''} | browser ${group?.group_id}: ${group?.controller}/${group?.agent_state}${group?.user_hold ? ' hold' : ''} tabs=${group?.tabs?.length}`
      : 'no ui session yet'
    if (line !== last) { log(`[${((Date.now() - started) / 1000).toFixed(0)}s] ${line}`); last = line }
    if (until && await until({ current, status, group })) return { current, status, group, reason: 'until' }
    const idle = current && !current.open_turn && !current.unfinished_works
    if (idle) {
      idleSince ??= Date.now()
      if (Date.now() - idleSince >= (wasBusy ? settleMs : graceMs)) return { current, status, group, reason: 'idle' }
    } else {
      idleSince = null
      if (current) wasBusy = true
    }
    await sleep(2000)
  }
  return { reason: 'timeout' }
}
