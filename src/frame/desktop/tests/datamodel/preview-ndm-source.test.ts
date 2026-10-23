import { NDM_PREVIEW_MAX_BYTES, resolveNdmSource } from '../../src/components/preview/ndmSource.ts'
import { PreviewError, type PreviewErrorCode } from '../../src/components/preview/types.ts'

const objectId = `cyfile:${'a'.repeat(64)}`
const chunkId = `sha256:${'b'.repeat(64)}`
const source = { kind: 'object-id' as const, objectId }
const endpoint = 'https://test.buckyos.io'
const resolve = (signal?: AbortSignal) => resolveNdmSource(source, endpoint, 'viewer-token', signal)

function equal(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`)
}

function metadata(value: Record<string, unknown> = {}) {
  return Response.json({ obj_id: objectId, obj_data: JSON.stringify({ name: 'result.mp4', size: 5, content: chunkId, mime_type: 'video/mp4', ...value }) })
}

async function fails(operation: () => Promise<unknown>, code: PreviewErrorCode) {
  try {
    await operation()
  } catch (error) {
    if (error instanceof PreviewError && error.code === code) return
    throw error
  }
  throw new Error(`Expected ${code}`)
}

async function withFetch(fetcher: typeof fetch, run: () => Promise<void>) {
  const original = globalThis.fetch
  globalThis.fetch = fetcher
  try { await run() } finally { globalThis.fetch = original }
}

Deno.test('cyfile preview reads authenticated NDM metadata and file bytes, preserving the content identity', async () => {
  const requests: string[] = []
  const controller = new AbortController()
  await withFetch(async (url, init) => {
    requests.push(String(url))
    equal(init?.method, 'POST')
    equal(new Headers(init?.headers).get('authorization'), 'Bearer viewer-token')
    equal(init?.credentials, 'include')
    equal(init?.body, JSON.stringify({ obj_id: objectId }))
    equal(init?.signal === controller.signal, true)
    return requests.length === 1 ? metadata() : new Response('video', { headers: { 'content-length': '5' } })
  }, async () => {
    const result = await resolve(controller.signal)
    equal(requests, [`${endpoint}/ndm/v1/store/get_object`, `${endpoint}/ndm/proxy/v1/read/object/open`])
    equal([result.displayName, result.size, result.sourceObjectId, result.inputObjectId, result.mediaTypeHints], ['result.mp4', 5, objectId, chunkId, ['video/mp4']])
    if (result.readRef.kind !== 'blob') throw new Error('Expected a Blob read reference')
    equal(result.readRef.blob.type, 'video/mp4')
    equal(await result.readRef.blob.text(), 'video')
  })
})

Deno.test('chunk-list file content is resolved by the server through its file ID', async () => {
  const content = `clist:${'c'.repeat(64)}`
  let calls = 0
  await withFetch(async (_url, init) => {
    calls++
    equal(JSON.parse(String(init?.body)).obj_id, objectId)
    return calls === 1 ? metadata({ content, name: 'movie.mp4', mime_type: null }) : new Response('video')
  }, async () => {
    const result = await resolve()
    equal(result.inputObjectId, content)
    equal(result.mediaTypeHints, ['video/mp4'])
  })
})

Deno.test('raw chunks use the authenticated reader without file metadata', async () => {
  await withFetch(async (url, init) => {
    equal(url, `${endpoint}/ndm/proxy/v1/read/object/open`)
    equal(JSON.parse(String(init?.body)).obj_id, chunkId)
    equal(new Headers(init?.headers).has('authorization'), false)
    return new Response('raw')
  }, async () => {
    const result = await resolveNdmSource({ kind: 'object-id', objectId: chunkId }, endpoint, null)
    equal(result.size, 3)
    equal(result.inputObjectId, chunkId)
  })
})

Deno.test('object read failures preserve permission, missing, timeout and server errors', async () => {
  for (const stage of ['metadata', 'content']) {
    for (const [status, code] of [[401, 'PERMISSION_DENIED'], [403, 'PERMISSION_DENIED'], [404, 'NOT_FOUND'], [413, 'TOO_LARGE'], [415, 'UNSUPPORTED'], [504, 'TIMEOUT'], [503, 'NETWORK']] as const) {
      let calls = 0
      await withFetch(async () => stage === 'content' && calls++ === 0 ? metadata() : new Response('denied', { status }), () => fails(resolve, code))
    }
  }
})

Deno.test('invalid input and corrupt file metadata never open a content stream', async () => {
  await withFetch(async () => { throw new Error('must not fetch') }, async () => {
    await fails(() => resolveNdmSource({ kind: 'object-id', objectId: 'cyfile:not-hex' }, endpoint, null), 'INVALID_SOURCE')
    await fails(() => resolveNdmSource({ kind: 'object-id', objectId: `cydir:${'a'.repeat(64)}` }, endpoint, null), 'UNSUPPORTED')
  })
  for (const response of [() => new Response('not json'), () => metadata({ size: -1 }), () => metadata({ content: `cydir:${'d'.repeat(64)}` })]) {
    let calls = 0
    await withFetch(async () => { calls++; return response() }, async () => {
      await fails(resolve, 'CORRUPTED')
      equal(calls, 1)
    })
  }
})

Deno.test('metadata and response lengths reject oversized previews before buffering', async () => {
  let calls = 0
  await withFetch(async () => { calls++; return metadata({ size: NDM_PREVIEW_MAX_BYTES + 1 }) }, async () => {
    await fails(resolve, 'TOO_LARGE')
    equal(calls, 1)
  })
  let cancelled = false
  calls = 0
  await withFetch(async () => ++calls === 1 ? metadata() : new Response(new ReadableStream({ cancel() { cancelled = true } }), { headers: { 'content-length': String(NDM_PREVIEW_MAX_BYTES + 1) } }), async () => {
    await fails(resolve, 'TOO_LARGE')
    equal(cancelled, true)
  })
})

Deno.test('an oversized stream without a length is stopped and an incomplete file is rejected', async () => {
  const chunk = new Uint8Array(1024 * 1024)
  let cancelled = false
  let calls = 0
  await withFetch(async () => ++calls === 1 ? metadata() : new Response(new ReadableStream({
    pull(controller) { controller.enqueue(chunk) },
    cancel() { cancelled = true },
  })), async () => {
    await fails(resolve, 'TOO_LARGE')
    equal(cancelled, true)
  })
  calls = 0
  await withFetch(async () => ++calls === 1 ? metadata() : new Response('bad'), () => fails(resolve, 'CORRUPTED'))
})

Deno.test('cancelling a preview stops a pending content stream and never reports it as ready', async () => {
  const controller = new AbortController()
  let cancelled = false
  let calls = 0
  let reading!: () => void
  const started = new Promise<void>(done => { reading = done })
  await withFetch(async () => ++calls === 1 ? metadata() : new Response(new ReadableStream({
    pull() { reading() },
    cancel() { cancelled = true },
  })), async () => {
    const pending = fails(() => resolve(controller.signal), 'CANCELLED')
    await started
    controller.abort()
    await pending
    equal(cancelled, true)
  })
  await withFetch(async () => { throw new Error('must not fetch') }, () => fails(() => resolve(controller.signal), 'CANCELLED'))
})
