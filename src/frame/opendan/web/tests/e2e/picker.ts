import type { Page } from '@playwright/test'

type Payload = string | { name: string; mimeType: string; buffer: Buffer }

// A stand-in for the system file picker page (`/embed/sysdlg/select`) that `vite dev` "serves" as the system entry:
// it speaks buckyos.sysdlg-embed by hand and answers with a file of this device or a Zone path, as the test says.
const PICKER = `<!doctype html><title>Picker embed harness</title><pre data-testid="fake-picker"></pre>
<input data-testid="fake-local" type="file"><input id="path" aria-label="Path"><button id="zone">Answer zone</button><button id="cancel">Answer cancel</button><script>
const params = new URLSearchParams(location.hash.slice(1)); const nonce = params.get('bse'); const parentOrigin = params.get('po')
const post = (type, payload) => parent.postMessage({ protocol: 'buckyos.sysdlg-embed', version: 1, type, payload }, parentOrigin)
const hello = setInterval(() => post('embed.hello', { nonce }), 250); post('embed.hello', { nonce })
addEventListener('message', e => {
  const m = e.data; if (e.source !== parent || e.origin !== parentOrigin || m?.protocol !== 'buckyos.sysdlg-embed' || m.type !== 'embed.init' || m.payload.nonce !== nonce) return
  clearInterval(hello); document.querySelector('pre').textContent = JSON.stringify(m.payload.params)
})
document.querySelector('[type=file]').onchange = e => { const file = e.target.files[0]; post('embed.result', { result: { action: 'selected', items: [{ origin: 'local', kind: 'file', file, hints: { name: file.name, mime: file.type, size: file.size } }] } }) }
document.querySelector('#zone').onclick = () => { const path = document.querySelector('#path').value; post('embed.result', { result: { action: 'selected', items: [{ origin: 'zone', kind: 'file', source: { kind: 'cyfs-path', path: 'cyfs://' + path }, hints: { name: path.split('/').pop() } }] } }) }
document.querySelector('#cancel').onclick = () => post('embed.result', { result: { action: 'cancelled' } })
</script>`

/**
 * Serves the fake system picker page and a fake Zone file service (NFSP `hello` / `resolve` / `read`) holding these
 * files, by DFS path.
 */
export async function fakeZone(page: Page, files: Record<string, Buffer> = {}) {
  await page.route('**/embed/sysdlg/select', (route) => route.fulfill({ contentType: 'text/html; charset=utf-8', body: PICKER }))
  await page.route('**/nfs/v1/**', async (route) => {
    const path = decodeURIComponent(new URL(route.request().url()).pathname)
    const json = (body: unknown, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) })
    if (path.startsWith('/nfs/v1/read/')) {
      const bytes = files[path.slice('/nfs/v1/read/'.length)]
      return bytes ? route.fulfill({ body: bytes }) : json({ ok: false, error: { code: 'NOT_FOUND' } }, 404)
    }
    const method = path.split('/').pop()
    if (method === 'hello') return json({ ok: true, result: { version: 'nfsp/0', session: 'test-session', features: [], limits: {}, realms: [] } })
    if (method === 'resolve' || method === 'stat') {
      const at = (route.request().postDataJSON() ?? {}).at ?? {}
      const target = String(at.uri ?? at.path ?? at.ref?.node_id ?? '').replace(/^cyfs:\/\//, '')
      if (!files[target]) return json({ ok: false, error: { code: 'NOT_FOUND', message: target } }, 404)
      return json({ ok: true, result: { kind: 'file', state: 'live', ref: { type: 'live', node_id: target }, node_id: target, name: target.split('/').pop(), size: files[target].length, etag: '1',
        capabilities: { list: false, read: true, accepts_content: false, accepts_references: false, remove_semantics: 'destroy', ordered: false } } })
    }
    return json({ ok: false, error: { code: 'UNSUPPORTED', message: method } }, 409)
  })
}

/** The picker framed over a standalone page (path C). */
export const picker = (page: Page) => page.frameLocator('[data-buckyos-sysdlg="select"] iframe')

export async function pickLocal(page: Page, file: Payload) {
  await picker(page).getByTestId('fake-local').setInputFiles(file)
}

export async function pickZone(page: Page, path: string) {
  await picker(page).getByLabel('Path').fill(path)
  await picker(page).getByRole('button', { name: 'Answer zone' }).click()
}

export async function pickerParams(page: Page): Promise<Record<string, unknown>> {
  const text = picker(page).getByTestId('fake-picker')
  await text.filter({ hasText: /\{/ }).waitFor()
  return JSON.parse(await text.textContent() ?? '{}')
}
