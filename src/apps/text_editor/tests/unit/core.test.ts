import { test } from 'node:test'
import assert from 'node:assert/strict'
import { decodeText, encodeText, normalizeEtag, MAX_FILE_SIZE } from '../../src/fs/codec.ts'
import { encodeBuffer, decodeBuffer, pathKey } from '../../src/fs/bufferStore.ts'
import { conflictCopyName } from '../../src/fs/recoveryStore.ts'
import { saveDocument, archiveMine } from '../../src/fs/saveFlow.ts'
import { SaveConflict } from '../../src/fs/documentStore.ts'
import { parseLaunch } from '../../src/platform/launch.ts'
import { encodeSession, contentDescriptor, resolveContentHandlers, emptyDefaults } from 'buckyos/content'
const source = { kind: 'cyfs-path' as const, path: 'cyfs:///home/alice/a.md' }
test('UTF-8/BOM/UTF-16 and line endings round trip without a final newline', () => {
  for (const encoding of ['utf-8', 'utf-16le', 'utf-16be']) for (const eol of ['\n', '\r\n', '\r'] as const) {
    for (const bom of encoding === 'utf-8' ? [false, true] : [true]) {
      const bytes = encodeText('中文 🙂\nNext', { encoding, bom, eol, mixedEol: false })
      const decoded = decodeText(bytes)
      assert.equal(decoded.text, '中文 🙂\nNext'); assert.equal(decoded.eol, eol); assert.deepEqual(encodeText(decoded.text, decoded), bytes)
    }
  }
  assert.equal(decodeText(new TextEncoder().encode('a\r\nb\nc\r\n')).mixedEol, true)
  assert.equal(normalizeEtag('W/"12-34"'), '12-34')
})
test('binary and oversized files are rejected; invalid UTF-8 is read only', () => {
  assert.throws(() => decodeText(new Uint8Array([0, 1])), /binary/)
  assert.throws(() => decodeText(new Uint8Array(MAX_FILE_SIZE + 1)), /too-large/)
  assert.equal(decodeText(new Uint8Array([0xff])).readOnly, true)
})
test('buffer format preserves newline content and stable path identity', async () => {
  const header = { format: 'buckyos.text-editor.buffer/1' as const, bufferId: 'id', source, displayName: 'a.md', encoding: 'utf-8', bom: false, eol: '\n' as const, createdAt: 1, updatedAt: 2, origin: { device: 'device' } }
  const record = { header, text: '\n中文\n' }
  assert.deepEqual(decodeBuffer(encodeBuffer(record)), record)
  assert.equal((await pathKey(source)).length, 16); assert.equal(await pathKey(), 'untitled')
  assert.throws(() => decodeBuffer(new TextEncoder().encode('{}\nx')), /invalid-buffer/)
})
test('launch round trip and large sessions fall back to their common parent', () => {
  const session = { kind: 'list' as const, currentIndex: 0, items: Array.from({ length: 100 }, (_, i) => ({ source: { ...source, path: `cyfs:///home/alice/${i}.md` } })) }
  const encoded = encodeSession(session, source)
  const request = parseLaunch(`https://editor.example/open?src=${encodeURIComponent(source.path)}&session=${encoded}`)!
  assert.equal(request.session?.kind, 'container')
  session.items[1].source.path = 'cyfs:///elsewhere/a.md'
  assert.equal(encodeSession(session, source), '')
  assert.throws(() => parseLaunch('https://editor.example/open?src=javascript:alert(1)'))
})
test('conflict copy keeps extension and uses collision sequence', () => {
  assert.equal(conflictCopyName('a.tar.gz', new Date(2026, 9, 6, 10, 22, 33), 'en', 2), 'a.tar (conflict copy 2026-10-06 102233) 2.gz')
})
function fixture() {
  const events: string[] = []; let disk = { etag: 'theirs', bytes: new TextEncoder().encode('theirs') }; let failArchive = false; let collisions = 0
  const doc = { source, name: 'a.md', text: 'mine', base: { etag: 'base', size: 4 }, readOnly: false, encoding: 'utf-8', bom: false, eol: '\n' as const, mixedEol: false }
  const services: any = { origin: { device: 'test' }, flush: async () => { events.push('flush') },
    store: { stat: async () => ({ ...disk, source, size: disk.bytes.length }), read: async () => ({ ...disk, source, size: disk.bytes.length }),
      write: async (path: string, bytes: Uint8Array, expected: string | null) => {
        events.push(path === source.path ? 'write-original' : 'write-copy')
        if (path !== source.path && collisions++ === 0) throw new SaveConflict('external-modified')
        if (path === source.path) { assert.equal(expected, disk.etag); disk = { etag: 'saved', bytes }; }
        return { source, etag: disk.etag, size: bytes.length }
      } }, archive: async (bytes: Uint8Array) => { events.push('archive:' + new TextDecoder().decode(bytes)); if (failArchive) throw new Error('archive-failed'); return {} }, annotate: async () => { events.push('annotate') } }
  return { doc, services, events, fail: () => { failArchive = true } }
}
test('save detects a disk conflict without writing', async () => {
  const f = fixture(); await assert.rejects(saveDocument(f.doc, f.services), /external-modified/); assert.deepEqual(f.events, ['flush'])
})
test('overwrite archives disk and copy before modifying original, including collisions', async () => {
  const f = fixture(); await saveDocument(f.doc, f.services, { overwrite: true, conflictCopy: true }); assert.deepEqual(f.events, ['flush', 'archive:theirs', 'write-copy', 'write-copy', 'annotate', 'write-original'])
})
test('archival failure prevents overwrite and discard', async () => {
  const f = fixture(); f.fail(); await assert.rejects(saveDocument(f.doc, f.services, { overwrite: true }), /archive-failed/); assert.ok(!f.events.includes('write-original'))
  await assert.rejects(archiveMine(f.doc, f.services, 'discarded'), /archive-failed/)
})
test('resolver respects defaults, visibility, disabled handlers, and wildcard fallback', () => {
  const handler = { provider: 'app' as const, app_instance_id: 'editor@alice', handler_id: 'text', handler_version: 1, enabled: true, selectors: [{ mime: 'text/*', maxSize: 16777216 }], intents: { open: { entry: { type: 'web' as const, path: '/open' }, priority: 60 } } }
  const registry = { schema_version: 1 as const, handlers: { 'editor@alice#text': handler, 'other@alice#any': { ...handler, app_instance_id: 'other@alice', selectors: [{ mime: '*/*' }] } } }
  const defaults = emptyDefaults(); const desc = contentDescriptor(source)
  const resolve = (apps = ['editor@alice', 'other@alice']) => resolveContentHandlers(registry, defaults, desc, 'open', {}, apps)
  assert.equal(resolve()[0].handlerKey, 'editor@alice#text')
  assert.equal(resolve([])[0].handlerKey, 'system#preview')
  defaults.disabled.push('editor@alice#text'); assert.equal(resolve()[0].handlerKey, 'system#preview')
  defaults.defaults.open = { 'mime:text/markdown': 'other@alice#any' }; assert.equal(resolve()[0].reason, 'user-default')
})

test('an older queued upload never replaces a newer local prewrite', async () => {
  const { BufferStore } = await import('../../src/fs/bufferStore.ts')
  const records = new Map<string, any>()
  const local: any = { get: async (key: string) => records.get(key), put: async (key: string, value: unknown) => { records.set(key, value) }, delete: async (key: string) => { records.delete(key) } }
  const store: any = { stat: async () => ({ etag: 'base' }), write: async () => ({}) }
  const buffers = new BufferStore('cyfs:///home/alice/.local/share/app', store, local)
  const first = { header: { format: 'buckyos.text-editor.buffer/1' as const, bufferId: 'id', source, displayName: 'a.md', encoding: 'utf-8', bom: false, eol: '\n' as const, createdAt: 1, updatedAt: 2, origin: { device: 'test' } }, text: 'first', path: 'cyfs:///buffer.buf', version: 1 }
  await buffers.localWrite(first)
  const upload = buffers.flush(first)
  await buffers.localWrite({ ...first, text: 'latest', version: 2 })
  await upload
  assert.equal(records.get('buffer:' + first.path).text, 'latest')
})

test('Markdown and HTML outlines come from parsed syntax nodes', async () => {
  const { EditorState } = await import('@codemirror/state')
  const { languages } = await import('@codemirror/language-data')
  const { extractOutline } = await import('../../src/editor/outline/index.ts')
  for (const [language, text, titles, levels] of [
    ['Markdown', '# One\n\n## Two\n\n```md\n# Not a heading\n```', ['One', 'Two'], [1, 2]],
    ['HTML', '<h1>One</h1><p>Body</p><h3><em>Two</em></h3>', ['One', 'Two'], [1, 3]],
  ] as const) {
    const support = await languages.find(l => l.name === language)!.load()
    const outline = extractOutline(EditorState.create({ doc: text, extensions: [support] }))
    assert.deepEqual(outline.map(h => h.title), titles)
    assert.deepEqual(outline.map(h => h.level), levels)
  }
})
