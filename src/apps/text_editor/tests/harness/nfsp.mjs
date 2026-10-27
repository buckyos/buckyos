import http from 'node:http'
import { randomUUID } from 'node:crypto'
let clock = 0
let files = new Map()
// Bucket bindings `<bucket_id>/<key>` of the editor's buffers / recovery buckets (data plane PUT/GET + kRPC list/delete).
const objects = new Map()
const uploads = new Map(), leases = new Map(), sessions = new Set(), streams = new Set()
let failure = '', leasedPath = '', dropCommits = 0
const replies = new Map()
// A stand-in for the system Preview page (`/embed/preview`): speaks buckyos.preview-embed by hand, from another origin.
const EMBED = `<!doctype html><title>Preview embed harness</title><div id="view" data-testid="fake-preview"></div><button id="openwith">Open with…</button><script>
const params = new URLSearchParams(location.hash.slice(1)); const nonce = params.get('bpe'); const parentOrigin = params.get('po')
const post = (type, payload) => parent.postMessage({ protocol: 'buckyos.preview-embed', version: 1, type, payload }, parentOrigin)
let init; const hello = setInterval(() => post('embed.hello', { nonce }), 250); post('embed.hello', { nonce })
addEventListener('message', async e => {
  const m = e.data; if (e.source !== parent || e.origin !== parentOrigin || m?.protocol !== 'buckyos.preview-embed') return
  const view = document.querySelector('#view')
  if (m.type === 'embed.update' && m.payload.theme) view.dataset.theme = m.payload.theme
  if (m.type !== 'embed.init' || m.payload.nonce !== nonce) return
  clearInterval(hello); init = m.payload; view.dataset.theme = init.theme; view.dataset.kind = init.source.kind
  if (init.source.kind === 'blob') { view.textContent = init.source.name + ':' + await init.source.blob.text(); return post('embed.state', { status: 'ready', title: init.source.name }) }
  const path = init.source.path.replace(/^cyfs:\\/\\//, ''); view.dataset.path = path
  const state = await (await fetch('/test/state')).json()
  if (!(path in state)) { view.textContent = 'missing ' + path; post('embed.state', { status: 'error', error: { kind: 'not-found', code: 'NOT_FOUND' } }) }
  else { view.textContent = 'preview ' + path; post('embed.state', { status: 'ready', title: path.split('/').pop() }) }
})
document.querySelector('#openwith').onclick = () => post('embed.openWith', { request: { source: init.source }, hints: { name: init.source.path.split('/').pop(), size: 3 } })
</script>`
const normalize = path => path?.replace(/^cyfs:\/\//, '').replace(/\/$/, '') || '/'
const ref = path => ({ type: 'live', node_id: path })
const info = path => {
  const f = files.get(path); if (!f) throw { code: 'NOT_FOUND', message: path }
  return { kind: f.dir ? 'dir' : 'file', state: 'live', ref: ref(path), node_id: path, name: path.split('/').pop(), size: f.bytes?.length ?? 0, etag: String(f.version), capabilities: { list: !!f.dir, read: !f.dir, accepts_content: true, accepts_references: false, remove_semantics: 'destroy', ordered: false } }
}
function put(path, bytes, dir = false) { files.set(path, { dir, bytes: Buffer.from(bytes), version: ++clock }); for (const stream of streams) stream.write(`event: container_changed\ndata: {}\n\n`) }
function reset() { files = new Map(); objects.clear(); uploads.clear(); leases.clear(); replies.clear(); failure = ''; leasedPath = ''; dropCommits = 0; for (const path of ['/', '/home', '/home/test-user', '/home/test-user/notes']) put(path, '', true); put('/home/test-user/notes/demo.md', '# Hello\n\nOriginal\n'); put('/home/test-user/notes/other.txt', 'Other file\n'); put('/home/test-user/notes/page.html', '<h1>Heading</h1>\n'); put('/home/test-user/notes/binary.bin', Buffer.from([0, 1, 2])); }
reset()
const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost'); const path = decodeURIComponent(url.pathname)
  let data = Buffer.alloc(0); for await (const chunk of req) data = Buffer.concat([data, chunk])
  const send = value => { res.setHeader('Content-Type', 'application/json'); res.end(JSON.stringify(value)) }
  if (path === '/test/reset') { reset(); return send({}) }
  if (path === '/test/state') return send(Object.fromEntries([...[...files].map(([key, f]) => [key, f.dir ? null : f.bytes.toString()]), ...[...objects].map(([key, o]) => [`bucket:${key}`, o.bytes.toString()])]))
  if (path === '/test/modify') { const body = JSON.parse(data); put(body.path, body.text); return send({}) }
  if (path === '/test/delete') { files.delete(JSON.parse(data).path); return send({}) }
  if (path === '/test/fail') { failure = JSON.parse(data).path; return send({}) }
  if (path === '/test/lease') { leasedPath = JSON.parse(data).path; return send({}) }
  // The next `count` commit_file answers are lost after the commit was applied (the client sees a network error).
  if (path === '/test/drop-commits') { dropCommits = JSON.parse(data).count; return send({}) }
  if (path === '/embed/preview') { res.setHeader('Content-Type', 'text/html; charset=utf-8'); return res.end(EMBED) }
  if (path.startsWith('/bucket/v1/')) {
    const id = path.slice('/bucket/v1/'.length)
    if (req.method === 'PUT') {
      if (failure && id.includes(failure)) { res.statusCode = 403; return send({ error: 'permission_denied', message: 'injected failure' }) }
      const created = !objects.has(id); objects.set(id, { bytes: data, created_at: Date.now() })
      return send({ version: ++clock, etag: `cyfile:${clock}`, created, obj_id: `cyfile:${clock}` })
    }
    const o = objects.get(id)
    if (!o) { res.statusCode = 404; return send({ error: 'no_such_key', message: id }) }
    res.setHeader('Content-Length', o.bytes.length); return res.end(o.bytes)
  }
  if (path === '/kapi/nfs-server') {
    const { method, params, sys } = JSON.parse(data); const prefix = `${params.bucket_id}/`
    const reply = result => send({ result, sys: [sys[0]] })
    if (method === 'bucket.list_objects') return reply({ common_prefixes: [], items: [...objects].filter(([k]) => k.startsWith(prefix + (params.prefix ?? ''))).sort(([a], [b]) => a < b ? -1 : 1)
      .map(([k, o]) => ({ bucket_id: params.bucket_id, key: k.slice(prefix.length), version: 1, is_current: true, size: o.bytes.length, etag: '', created_at: o.created_at, created_by: 'test-user|text-editor' })) })
    if (method === 'bucket.delete') return reply({ deleted: objects.delete(prefix + params.key) })
    return send({ error: `unknown method ${method}`, code: 'invalid_argument', sys: [sys[0]] })
  }
  try {
    if (path.startsWith('/nfs/v1/read/')) {
      const target = path.slice('/nfs/v1/read/'.length); const node = info(target)
      res.setHeader('ETag', `W/"${node.etag}"`); return res.end(files.get(target).bytes)
    }
    if (path.startsWith('/nfs/v1/uploads/')) {
      const id = path.split('/').pop(); const upload = uploads.get(id)
      if (!upload) throw { code: 'NOT_FOUND' }
      if (req.method === 'PATCH') { if (Number(req.headers['upload-offset']) !== upload.bytes.length) throw { code: 'TARGET_MISMATCH' }; upload.bytes = Buffer.concat([upload.bytes, data]); res.statusCode = 204 }
      res.setHeader('Upload-Offset', upload.bytes.length); return res.end()
    }
    if (path === '/nfs/v1/watch') { res.writeHead(200, { 'Content-Type': 'text/event-stream' }); streams.add(res); res.write('event: resync\ndata: {}\n\n'); req.on('close', () => streams.delete(res)); return }
    const method = path.split('/').pop(); const body = data.length ? JSON.parse(data) : {}; const a = body.args ?? {}
    if (method === 'hello') { const session = randomUUID(); sessions.add(session); return send({ ok: true, result: { version: 'nfsp/0', session, features: [], limits: {}, realms: [] } }) }
    if (!sessions.has(body.session)) throw { code: 'PERMISSION_DENIED', message: 'invalid session' }
    const replay = body.seq !== undefined && replies.get(`${body.session}:${body.seq}`)
    if (replay) { if (method === 'commit_file' && dropCommits > 0) { dropCommits--; return req.socket.destroy() } return send(replay) }
    const locate = at => normalize(at?.uri ?? at?.path ?? at?.ref?.node_id)
    let result = {}
    if (method === 'resolve' || method === 'stat') result = info(locate(body.at))
    if (method === 'list') { const parent = locate(body.at); result = { container: info(parent), entries: [...files].filter(([p]) => p !== parent && p.slice(0, p.lastIndexOf('/')) === parent).map(([p]) => ({ name: p.split('/').pop(), binding: 'native', target: { ref: ref(p), kind: files.get(p).dir ? 'dir' : 'file', attrs: { size: files.get(p).bytes.length } } })), watch_token: parent } }
    if (method === 'mkdir' && a.name) { const target = `${normalize(a.parent_ref?.node_id ?? locate(body.at))}/${a.name}`; if (!files.has(target)) put(target, '', true); result = { ref: ref(target), existed: true } }
    if (method === 'mkdir' && !a.name) {
      const target = locate(body.at); let existed = true; let at = ''
      for (const part of target.split('/').filter(Boolean)) { at += '/' + part; const f = files.get(at); if (f && !f.dir) throw { code: 'NAMESPACE_CONFLICT', message: `a non-directory exists at '${at}'` }; if (!f) { put(at, '', true); existed = false } }
      result = { ref: ref(target), existed }
    }
    if (method === 'delete') { const target = `${locate(body.at)}/${a.name}`; for (const p of files.keys()) if (p === target || a.recursive && p.startsWith(target + '/')) files.delete(p) }
    if (method === 'open_write') {
      const target = a.ref ? normalize(a.ref.node_id) : `${normalize(a.parent_ref.node_id)}/${a.name}`
      if (target === leasedPath || leases.has(target)) throw { code: 'LEASE_CONFLICT' }
      const id = randomUUID(); const lease = randomUUID(); leases.set(target, lease); uploads.set(id, { path: target, bytes: Buffer.alloc(0), base: files.get(target)?.version, lease, session: body.session })
      result = { fb_handle: id, lease: { lease_id: lease, ttl_ms: 600000 }, target: { path: target, exists: files.has(target) } }
    }
    if (method === 'abort_write') { for (const [id, upload] of uploads) if (upload.lease === a.lease_id) { if (upload.session !== body.session) throw { code: 'LEASE_CONFLICT' }; leases.delete(upload.path); uploads.delete(id) } }
    if (method === 'commit_file') {
      const upload = uploads.get(a.fb_handle)
      if (!upload) throw { code: 'NOT_FOUND' }
      if (failure && upload.path.includes(failure)) throw { code: 'PERMISSION_DENIED', message: 'injected failure' }
      if (files.get(upload.path)?.version !== upload.base) throw { code: 'TARGET_MISMATCH', details: { reason: 'bypass_modified' } }
      put(upload.path, upload.bytes); leases.delete(upload.path); uploads.delete(a.fb_handle); result = { ref: ref(upload.path), obj: { size: upload.bytes.length } }
    }
    if (body.seq !== undefined) replies.set(`${body.session}:${body.seq}`, { ok: true, result })
    if (method === 'commit_file' && dropCommits > 0) { dropCommits--; return req.socket.destroy() }
    send({ ok: true, result })
  } catch (error) { res.statusCode = error.code === 'NOT_FOUND' ? 404 : error.code === 'LEASE_CONFLICT' ? 423 : 409; send({ ok: false, error }) }
})
server.listen(3260, '127.0.0.1')
