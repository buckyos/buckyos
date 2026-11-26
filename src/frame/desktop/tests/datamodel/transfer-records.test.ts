import { CONTENT_REFS_FORMAT, TRANSFER_HANDLE_FORMAT, transferHandleFromHtml } from 'buckyos/transfer'
import { desktopTransfers, withTransferHandle } from '../../src/transfer/records.ts'

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
}

const offer = (purpose: 'copy' | 'cut' | 'drag', windowId = 'w1') =>
  desktopTransfers.offer({ purpose, source: { app: 'files', windowId }, itemKeys: ['0', '1'], formats: { 'application/vnd.test+json': () => ({ n: 1 }) }, operations: ['copy', 'move'] })

Deno.test('a cut is held by one operation at a time and goes away with its last item', async () => {
  desktopTransfers.clear()
  const cut = offer('cut')
  assertEquals(desktopTransfers.cutOf('files', 'w1')?.offerId, cut.offerId, 'the window sees its cut')
  assertEquals(desktopTransfers.reserve(cut.offerId, ['0'], 'op-a'), { ok: true }, 'first paste reserves')
  assertEquals(desktopTransfers.reserve(cut.offerId, ['0'], 'op-b'), { ok: false, reason: 'busy' }, 'second paste is told it is moving')
  desktopTransfers.release(cut.offerId, ['0'], 'op-a')
  assertEquals(desktopTransfers.reserve(cut.offerId, ['0', '1'], 'op-b'), { ok: true }, 'released items can be taken again')
  desktopTransfers.consume(cut.offerId, ['0'], 'op-b')
  assertEquals([...desktopTransfers.get(cut.offerId)!.remaining], ['1'], 'consumed item is gone')
  desktopTransfers.consume(cut.offerId, ['1'], 'op-b')
  assertEquals(desktopTransfers.get(cut.offerId), undefined, 'the record goes with its last item')
  assertEquals(desktopTransfers.reserve(cut.offerId, ['1'], 'op-c'), { ok: false, reason: 'stale' }, 'a consumed cut is stale')
  assertEquals(await desktopTransfers.read(cut.offerId, 'application/vnd.test+json'), undefined, 'nothing to read')
})

Deno.test('a newer copy replaces an unused cut; reads are lazy and cached', async () => {
  desktopTransfers.clear()
  const cut = offer('cut')
  const copy = offer('copy')
  assertEquals(desktopTransfers.get(cut.offerId), undefined, 'older cut dropped')
  assertEquals(await desktopTransfers.read(copy.offerId, 'application/vnd.test+json'), { n: 1 }, 'lazy value')
  assertEquals(await desktopTransfers.read(copy.offerId, 'application/vnd.other+json'), undefined, 'unknown format')
  const reserved = offer('cut')
  desktopTransfers.reserve(reserved.offerId, ['0'], 'op')
  offer('copy')
  assertEquals(!!desktopTransfers.get(reserved.offerId), true, 'a cut in flight survives a newer copy')
  desktopTransfers.withdraw(reserved.offerId)
  assertEquals(!!desktopTransfers.get(reserved.offerId), true, 'and cannot be withdrawn while moving')
  desktopTransfers.withdrawSource('w1')
  desktopTransfers.bindSession('alice@zone')
  desktopTransfers.bindSession('bob@zone')
  assertEquals(desktopTransfers.get(copy.offerId), undefined, 'another user drops every record')
})

Deno.test('the handle travels in its own MIME type and inside the HTML', () => {
  const reps = withTransferHandle({ 'text/plain': 'a.pdf', 'text/html': '<meta charset="utf-8"><a>a.pdf</a>', [CONTENT_REFS_FORMAT]: '{}' }, 'AbCdEfGhIjKlMnOpQrStUv')
  assertEquals(JSON.parse(reps[TRANSFER_HANDLE_FORMAT]), { v: 1, offerId: 'AbCdEfGhIjKlMnOpQrStUv' }, 'MIME')
  assertEquals(transferHandleFromHtml(reps['text/html'])?.offerId, 'AbCdEfGhIjKlMnOpQrStUv', 'HTML')
  assertEquals(reps['text/plain'], 'a.pdf', 'text stays readable')
})
