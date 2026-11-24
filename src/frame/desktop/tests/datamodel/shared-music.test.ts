import { folderPath, parentFolder, ZoneFolders } from '../../src/shared/music/zoneFolders.ts'
import { formatTime, isAudioFile, parseSavedPlayback, playOrder, resumePoint, sortTracks, trackAlbum, trackTitle } from '../../src/shared/music/playlist.ts'
import type { ZoneChannel } from '../../src/shared/channel.ts'

function equal(actual: unknown, expected: unknown, what = '') {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`${what}\nexpected ${JSON.stringify(expected)}\nreceived ${JSON.stringify(actual)}`)
}

async function rejects(work: Promise<unknown>, what: string) {
  try { await work } catch { return }
  throw Error(`${what}: did not reject`)
}

Deno.test('the music player plays audio files in folder order, or a shuffle that a seed brings back', () => {
  equal(['a.MP3', 'b.flac', 'c.opus', 'cover.jpg', 'notes.txt', '.mp3'].filter(isAudioFile), ['a.MP3', 'b.flac', 'c.opus'], 'audio files only')
  const tracks = sortTracks(['/m/B/1 x.mp3', '/m/A/10 c.mp3', '/m/A/2 b.mp3', '/m/A/1 a.mp3'])
  equal(tracks, ['/m/A/1 a.mp3', '/m/A/2 b.mp3', '/m/A/10 c.mp3', '/m/B/1 x.mp3'], 'numbers sort as numbers, albums together')
  equal(playOrder(tracks, 'sequential', 7), tracks, 'in order')
  const shuffled = playOrder(tracks, 'shuffle', 7)
  equal([...shuffled].sort(), [...tracks].sort(), 'a shuffle has every track once')
  equal(playOrder(tracks, 'shuffle', 7), shuffled, 'the same seed gives the same order')
  equal(resumePoint(shuffled, { folder: '/m', track: shuffled[2], position: 42, seed: 7 }, '/m'), { index: 2, position: 42 }, 'picks up where it stopped')
  equal(resumePoint(shuffled, { folder: '/other', track: shuffled[2], position: 42, seed: 7 }, '/m'), { index: 0, position: 0 }, 'another folder starts over')
  equal(resumePoint(shuffled, { folder: '/m', track: '/m/gone.mp3', position: 42, seed: 7 }, '/m'), { index: 0, position: 0 }, 'a removed track starts over')
  equal([trackTitle('/m/A/2 b.mp3'), trackAlbum('/m/A/2 b.mp3', '/m'), trackAlbum('/m/top.mp3', '/m/')], ['2 b', 'A', 'm'], 'names')
  equal([formatTime(0), formatTime(65.9), formatTime(3725), formatTime(Number.NaN)], ['0:00', '1:05', '1:02:05', '0:00'], 'times')
})

Deno.test('a saved playback is read back whole or not at all', () => {
  equal(parseSavedPlayback({ folder: '/m', track: '/m/a.mp3', position: 3, seed: 9 }), { folder: '/m', track: '/m/a.mp3', position: 3, seed: 9 })
  equal(parseSavedPlayback({ folder: '/m', track: '/m/a.mp3', position: -1, seed: 9 })?.position, 0, 'a bad position starts the track')
  equal((parseSavedPlayback({ folder: '/m', track: '/m/a.mp3', seed: 'x' })?.seed ?? 0) > 0, true, 'a bad seed is a new one')
  equal([parseSavedPlayback(null), parseSavedPlayback('x'), parseSavedPlayback({ folder: '/m' })], [null, null, null])
})

Deno.test('paths of a Zone', () => {
  equal(['cyfs:///home/a/', '/home//a', 'home/a', '/'].map(folderPath), ['/home/a', '/home/a', '/home/a', '/'])
  equal([parentFolder('/home/a'), parentFolder('/home'), parentFolder('/')], ['/home', '/', null])
})

interface Node { name: string; kind: 'dir' | 'file'; state?: string; canonical?: string }

/** A Zone of these folders; `pageSize` entries a page, sessions that end after `sessionCalls` calls. */
function zone(tree: Record<string, Node[]>, options: { pageSize?: number; sessionCalls?: number; user?: string | null; cutAfter?: number } = {}) {
  const calls: { method: string; at?: string; session?: string }[] = []
  let sessions = 0
  let left = 0
  const channel: ZoneChannel = {
    rpc: () => Promise.reject(new Error('unused')),
    fetch: (path, init) => {
      const method = path.replace('/nfs/v1/', '')
      const body = JSON.parse(String(init?.body)) as { session?: string; at?: { path?: string }; args: { cursor?: string } }
      const answer = (result: unknown) => Promise.resolve(Response.json({ ok: true, result }))
      const fail = (code: string, message: string) => Promise.resolve(Response.json({ ok: false, error: { code, message } }, { status: 403 }))
      const at = body.at?.path
      calls.push({ method, at, session: body.session })
      if (method === 'hello') {
        sessions += 1
        left = options.sessionCalls ?? Infinity
        return answer({ version: '1', session: `s${sessions}`, features: [], limits: { max_batch: 1, max_list: 1000, replay_window: 1, attr_ttl_ms: 1 }, realms: [], identity: { user_id: options.user === undefined ? 'alice' : options.user, role: 'user' } })
      }
      if (calls.length > (options.cutAfter ?? Infinity)) return Promise.reject(new TypeError('offline'))
      if (left-- <= 0 || body.session !== `s${sessions}`) return fail('PERMISSION_DENIED', 'session expired')
      if (method === 'list') {
        const nodes = at ? tree[at] : undefined
        if (!nodes) return fail('NOT_FOUND', 'no such folder')
        const size = options.pageSize ?? 100
        const start = Number(body.args.cursor ?? 0)
        return answer({
          container: {},
          entries: nodes.slice(start, start + size).map((node) => ({ name: node.name, binding: 'native', target: { kind: node.kind, ref: { type: 'live', node_id: node.name }, ...(node.state ? { target_state: node.state } : {}) }, ...(node.canonical ? { canonical_path: node.canonical } : {}) })),
          ...(start + size < nodes.length ? { next_cursor: String(start + size) } : {}),
        })
      }
      return answer({ kind: 'file', ref: { type: 'live', node_id: 'n 1/2' }, ...(at?.endsWith('signed.mp3') ? { access_urls: [{ kind: 'thumbnail', url: '/t' }, { kind: 'read', url: '/nfs/v1/read/x?sig=a%2Bb' }] } : {}) })
    },
  }
  return { folders: new ZoneFolders(channel), calls }
}

const library = {
  '/home/alice': [{ name: 'Music', kind: 'dir' }, { name: 'a.txt', kind: 'file' }, { name: '.hidden', kind: 'dir' }, { name: 'Books 10', kind: 'dir' }, { name: 'Books 2', kind: 'dir' }, { name: 'Gone', kind: 'dir', state: 'stale' }],
  '/home/alice/Music': [{ name: 'B', kind: 'dir' }, { name: 'A', kind: 'dir' }, { name: 'top.mp3', kind: 'file' }, { name: 'cover.jpg', kind: 'file' }, { name: 'Locked', kind: 'dir' }, { name: 'Shared', kind: 'dir', canonical: 'cyfs:///srv/library/Shared' }],
  '/home/alice/Music/A': [{ name: '10.mp3', kind: 'file' }, { name: '2.mp3', kind: 'file' }],
  '/home/alice/Music/B': [{ name: '1.flac', kind: 'file' }],
  '/srv/library/Shared': [{ name: 's.ogg', kind: 'file' }],
} satisfies Record<string, Node[]>

Deno.test('folders to choose from: no files, hidden or broken ones, over pages', async () => {
  const { folders, calls } = zone(library, { pageSize: 2 })
  equal(await folders.home(), '/home/alice')
  equal((await folders.folders('cyfs:///home/alice/')).map((folder) => folder.path), ['/home/alice/Books 2', '/home/alice/Books 10', '/home/alice/Music'])
  equal(calls.filter((call) => call.method === 'list').length, 3, 'three pages')
  equal(calls.filter((call) => call.method === 'hello').length, 1, 'one session')
  await rejects(zone(library, { user: null }).folders.folders('/home/alice'), 'an anonymous session')
})

Deno.test('the tracks under a folder: albums together, what cannot be read is skipped', async () => {
  const { folders } = zone(library, { pageSize: 3 })
  equal(await folders.tracks('/home/alice/Music'), ['/home/alice/Music/A/2.mp3', '/home/alice/Music/A/10.mp3', '/home/alice/Music/B/1.flac', '/home/alice/Music/top.mp3', '/srv/library/Shared/s.ogg'])
  await rejects(folders.tracks('/home/alice/Nowhere'), 'the folder itself')
  await rejects(zone(library, { cutAfter: 3 }).folders.tracks('/home/alice/Music'), 'a Zone that stops answering')
  const abort = new AbortController()
  abort.abort()
  await rejects(folders.tracks('/home/alice/Music', abort.signal), 'aborted')
})

Deno.test('a session the server has forgotten is made again, once', async () => {
  const { folders, calls } = zone(library, { sessionCalls: 1 })
  await folders.folders('/home/alice')
  equal((await folders.folders('/home/alice/Music')).length, 4)
  equal(calls.filter((call) => call.method === 'hello').length, 2)
  await rejects(zone(library, { sessionCalls: 0 }).folders.folders('/home/alice'), 'a session that never works')
})

Deno.test('where a track is read', async () => {
  const { folders } = zone(library)
  equal(await folders.readPath('/home/alice/Music/signed.mp3'), '/nfs/v1/read/x?sig=a%2Bb', 'the address the server signed')
  equal(await folders.readPath('/home/alice/Music/top.mp3'), '/nfs/v1/read/n%201%2F2', 'else the node')
})
