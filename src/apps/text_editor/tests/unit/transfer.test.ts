import { test } from 'node:test'
import assert from 'node:assert/strict'
import { CONTENT_REFS_FORMAT, readContentRefs } from 'buckyos/transfer'
import { deliverToEditor, offeredSelection, probeAnswer, readTabDrag, TAB_DRAG_FORMAT, tabDragValue } from '../../src/platform/transfer.ts'

const phrases = { insert: 'Insert at the cursor', open: 'Open in Text Editor' }
const source = { kind: 'cyfs-path' as const, path: 'cyfs:///home/alice/notes.md' }

test('the tab drag uses the editor’s own format, never text/plain', () => {
  assert.equal(TAB_DRAG_FORMAT, 'application/vnd.buckyos-text-editor.tab+json')
  assert.deepEqual(readTabDrag(tabDragValue('d1', 'g1')), { doc: 'd1', group: 'g1' })
  // Text dragged in from elsewhere (an old drag that put JSON in text/plain, or plain words) is not a tab.
  assert.equal(readTabDrag('["d1","g1"]'), undefined)
  assert.equal(readTabDrag('hello'), undefined)
  assert.equal(readTabDrag(undefined), undefined)
})

test('offers the selected text, else the open Zone file, else nothing', () => {
  const text = offeredSelection({ selection: 'Hello', doc: { id: 'd', source, name: 'notes.md' } })
  assert.deepEqual(text?.item, { kind: 'text', formats: ['text/plain'], operations: ['copy'], inline: { 'text/plain': 'Hello' } })
  const file = offeredSelection({ doc: { id: 'd', source, name: 'notes.md' } })
  assert.equal(file?.item.kind, 'zone-file')
  assert.deepEqual(file?.item.formats, [CONTENT_REFS_FORMAT, 'text/uri-list', 'text/plain'])
  assert.deepEqual(readContentRefs(file?.values[CONTENT_REFS_FORMAT])?.[0].source, source)
  assert.equal(file?.values['text/plain'], 'notes.md')
  assert.notEqual(text?.key, file?.key)
  // An unsaved document, a preview tab: nothing to offer.
  assert.equal(offeredSelection({ doc: { id: 'd', name: 'untitled.txt' } }), undefined)
  assert.equal(offeredSelection({ doc: { id: 'd', source, name: 'a.png', preview: {} } }), undefined)
  // A large selection is read when delivered instead of travelling with the offer.
  assert.equal(offeredSelection({ selection: 'x'.repeat(70000) })?.item.inline, undefined)
})

test('takes text into a writable editor and opens Zone files; nothing else', () => {
  const probe = (items: Array<{ kind: 'text' | 'zone-file' | 'zone-folder'; formats: string[] }>) =>
    ({ operationId: 'op', seq: 1, at: 'focus' as const, items: items.map((item, i) => ({ ...item, itemKey: String(i), operations: ['copy' as const] })) })
  assert.deepEqual(probeAnswer(probe([{ kind: 'text', formats: ['text/plain'] }]), { canInsert: true, phrases }),
    { targetId: 'cursor', action: 'Insert at the cursor', accept: [{ itemKey: '0', format: 'text/plain', operation: 'copy' }] })
  assert.equal(probeAnswer(probe([{ kind: 'text', formats: ['text/plain'] }]), { canInsert: false, phrases }), null)
  assert.deepEqual(probeAnswer(probe([{ kind: 'zone-file', formats: [CONTENT_REFS_FORMAT, 'text/plain'] }, { kind: 'zone-folder', formats: [CONTENT_REFS_FORMAT] }]), { canInsert: true, phrases }),
    { targetId: 'tabs', action: 'Open in Text Editor', accept: [{ itemKey: '0', format: CONTENT_REFS_FORMAT, operation: 'copy' }] })
  assert.equal(probeAnswer(probe([{ kind: 'zone-folder', formats: [CONTENT_REFS_FORMAT] }]), { canInsert: true, phrases }), null)
})

test('delivers: text goes in once, files open one by one with their own results', async () => {
  const inserted: string[] = [], opened: string[] = []
  const actions = {
    insert: (text: string) => { inserted.push(text); return true },
    open: async (ref: { kind: string; path?: string }) => { if (ref.path?.endsWith('bad.md')) throw new Error('not_found'); opened.push(ref.path ?? '') },
  }
  assert.deepEqual(await deliverToEditor({ operationId: 'op', targetId: 'cursor', operation: 'copy', at: 'focus', items: [{ itemKey: '0', format: 'text/plain', value: 'Hi' }] }, actions),
    { status: 'done', items: [{ itemKey: '0', status: 'done' }] })
  assert.deepEqual(inserted, ['Hi'])
  const zone = (path: string) => ({ origin: 'zone', kind: 'file', source: { kind: 'cyfs-path', path }, hints: {} })
  const result = await deliverToEditor({ operationId: 'op2', targetId: 'tabs', operation: 'copy', at: 'focus', items: [
    { itemKey: '0', format: CONTENT_REFS_FORMAT, value: zone('cyfs:///home/alice/a.md') },
    { itemKey: '1', format: CONTENT_REFS_FORMAT, value: zone('cyfs:///home/alice/bad.md') },
    { itemKey: '2', format: CONTENT_REFS_FORMAT, value: { origin: 'zone' } },
  ] }, actions)
  assert.deepEqual(result, { status: 'done', items: [{ itemKey: '0', status: 'done' }, { itemKey: '1', status: 'failed', error: 'not_found' }, { itemKey: '2', status: 'skipped', error: 'not_a_file' }] })
  assert.deepEqual(opened, ['cyfs:///home/alice/a.md'])
})
