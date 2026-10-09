import http from 'node:http'
import { randomUUID } from 'node:crypto'
let clock = 0
let files = new Map()
const uploads = new Map(), leases = new Map(), sessions = new Set(), streams = new Set()
let failure = '', leasedPath = ''
const normalize = path => path?.replace(/^cyfs:\/\//, '').replace(/\/$/, '') || '/'
const ref = path => ({ type: 'live', node_id: path })
const info = path => {
  const f = files.get(path); if (!f) throw { code: 'NOT_FOUND', message: path }
  return { kind: f.dir ? 'dir' : 'file', state: 'live', ref: ref(path), node_id: path, name: path.split('/').pop(), size: f.bytes?.length ?? 0, etag: String(f.version), capabilities: { list: !!f.dir, read: !f.dir, accepts_content: true, accepts_references: false, remove_semantics: 'destroy', ordered: false } }
}
function put(path, bytes, dir = false) { files.set(path, { dir, bytes: Buffer.from(bytes), version: ++clock }); for (const stream of streams) stream.write(`event: container_changed\ndata: {}\n\n`) }
function reset() { files = new Map(); uploads.clear(); leases.clear(); failure = ''; leasedPath = ''; for (const path of ['/', '/home', '/home/test-user', '/home/test-user/notes']) put(path, '', true); put('/home/test-user/notes/demo.md', '# Hello\n\nOriginal\n'); put('/home/test-user/notes/other.txt', 'Other file\n'); put('/home/test-user/notes/page.html', '<h1>Heading</h1>\n'); put('/home/test-user/notes/binary.bin', Buffer.from([0, 1, 2])); }
reset()
const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost'); const path = decodeURIComponent(url.pathname)
  let data = Buffer.alloc(0); for await (const chunk of req) data = Buffer.concat([data, chunk])
  const send = value => { res.setHeader('Content-Type', 'application/json'); res.end(JSON.stringify(value)) }
  if (path === '/test/reset') { reset(); return send({}) }
  if (path === '/test/state') return send(Object.fromEntries([...files].map(([key, f]) => [key, f.dir ? null : f.bytes.toString()])))
  if (path === '/test/modify') { const body = JSON.parse(data); put(body.path, body.text); return send({}) }
  if (path === '/test/delete') { files.delete(JSON.parse(data).path); return send({}) }
  if (path === '/test/fail') { failure = JSON.parse(data).path; return send({}) }
  if (path === '/test/lease') { leasedPath = JSON.parse(data).path; return send({}) }
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
    const locate = at => normalize(at?.uri ?? at?.path ?? at?.ref?.node_id)
    let result = {}
    if (method === 'resolve' || method === 'stat') result = info(locate(body.at))
    if (method === 'list') { const parent = locate(body.at); result = { container: info(parent), entries: [...files].filter(([p]) => p !== parent && p.slice(0, p.lastIndexOf('/')) === parent).map(([p]) => ({ name: p.split('/').pop(), binding: 'native', target: { ref: ref(p), kind: files.get(p).dir ? 'dir' : 'file', attrs: { size: files.get(p).bytes.length } } })), watch_token: parent } }
    if (method === 'mkdir') { const target = a.name ? `${normalize(a.parent_ref?.node_id ?? locate(body.at))}/${a.name}` : locate(body.at); if (!files.has(target)) put(target, '', true); result = { ref: ref(target), existed: true } }
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
    send({ ok: true, result })
  } catch (error) { res.statusCode = error.code === 'NOT_FOUND' ? 404 : error.code === 'LEASE_CONFLICT' ? 423 : 409; send({ ok: false, error }) }
})
server.listen(3260, '127.0.0.1')
