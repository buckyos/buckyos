import { transcodeLostMeta } from '../../src/app/messagehub/conversation/input/imageNormalize.ts'

function equal(actual: unknown, expected: unknown) { if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`) }

// A 6000x4000 photo with EXIF: the `jpeg_exif_little_endian` vector of the SDK's tests/media_meta_vectors.json.
const PHOTO = Uint8Array.from(atob('/9j/4AAQSkZJRgABAgAAAQABAAD/4QAraHR0cDovL25zLmFkb2JlLmNvbS94YXAvMS4wLwA8eDp4bXBtZXRhLz7/4QHURXhpZgAASUkqAAgAAAAEAA8BAgAFAAAAPgAAABABAgAJAAAAQwAAAGmHBAABAAAATAAAACWIBAABAAAAHgEAAAAAAABTb255AElMQ0UtN00zAAoAmoIFAAEAAADKAAAAnYIFAAEAAADSAAAAJ4gDAAEAAACQAQAAA5ACABQAAADaAAAABJACABQAAADuAAAAEZACAAcAAAACAQAAkZICAAQAAAAxMjUACpIFAAEAAAAJAQAABaQDAAEAAABLAAAANKQCAA0AAAARAQAAAAAAAAoAAACwBAAAHAAAAAoAAAAyMDI0OjAyOjI5IDEzOjA1OjA5ADIwMjA6MDE6MDEgMDA6MDA6MDAAKzA4OjAwAIgTAABkAAAARkUgNTBtbSBGMS44AAgAAQACAAIAAABOAAAAAgAFAAMAAACEAQAAAwACAAIAAABFAAAABAAFAAMAAACcAQAABQABAAEAAAAAAAAABgAFAAEAAAC0AQAAEQAFAAEAAAC8AQAAHwAFAAEAAADEAQAAAAAAACIAAAABAAAAOwAAAAEAAABEEAAAZAAAAIcAAAABAAAALQAAAAEAAABsCQAAZAAAANIEAABkAAAAWiMAAGQAAABBAAAACgAAAP/AAAsID6AXcAEBEQD/2gAC/9k='), char => char.charCodeAt(0))

Deno.test('a transcode keeps the facts about the shot, and leaves type and size to the new bytes', async () => {
  const meta = await transcodeLostMeta(new File([PHOTO], 'IMG.heic', { type: 'image/heic' }))
  equal(Object.keys(meta).sort(), ['camera', 'captured_at', 'exposure', 'geo'])
  equal(meta.captured_at, '2024-02-29T13:05:09.125+08:00')
  equal(meta.camera, { make: 'Sony', model: 'ILCE-7M3', lens: 'FE 50mm F1.8' })
})

Deno.test('a photo without EXIF loses nothing', async () => {
  equal(await transcodeLostMeta(new File([new Uint8Array([0xff, 0xd8, 0xff, 0xd9])], 'plain.jpg')), {})
})
