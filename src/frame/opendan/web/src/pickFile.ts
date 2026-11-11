import { AppFrameClient, embedderOrigin, originInZone } from 'buckyos/app-frame'
import { SystemDialogError, type ContentSelector, type ZoneContent } from 'buckyos/content'
import { NfspClient } from 'buckyos/nfsp'
import { lang } from './i18n'
import { dataModel } from './model'
import { sessionToken } from './model/krpc'

// The shell's handshake nonce (`#bfp=`) is in the hash the page was opened with. The Zone is known only once the
// profile is read, and the hash router may have moved on by then: the client is given the hash it was opened with.
const bootHash = window.location.hash
const setHash = (hash: string) => history.replaceState(history.state, '', `${location.pathname}${location.search}${hash}`)

// The Zone's system entry (`desktop_url`, null outside a Zone). `vite dev` serves the picker page itself (tests).
async function systemOrigin(): Promise<string | null> {
  if (import.meta.env.DEV) return import.meta.env.VITE_SYSTEM_ORIGIN || location.origin
  const url = (await dataModel.profile()).desktop_url
  return url ? new URL(url).origin : null
}

let frame: Promise<AppFrameClient | null> | undefined

/**
 * The App Frame link to the Desktop: the shell draws the system dialogs over this page's window when it embeds the
 * page (path B); a standalone tab frames the system page over itself instead (path C). Null outside a Zone.
 */
export function startAppFrame(): Promise<AppFrameClient | null> {
  frame ??= systemOrigin().catch(() => null).then((system) => {
    if (!system) return null
    const zone = new URL(system).hostname
    const shell = import.meta.env.DEV ? import.meta.env.VITE_SHELL_ORIGIN || location.origin : embedderOrigin((origin) => originInZone(origin, zone))
    const current = location.hash
    const restore = /[#&]bfp=/.test(bootHash) && current !== bootHash
    if (restore) setHash(bootHash)
    try {
      return new AppFrameClient({ shellOrigin: shell ?? system, systemOrigin: system })
    } finally {
      if (restore) setHash(current)
    }
  })
  return frame
}

let files: { client: NfspClient; hello?: Promise<unknown> } | undefined

/** A Zone file the user chose, read with this page's own session (the Zone file service answers on every host). */
async function readZoneFile(item: ZoneContent): Promise<File> {
  if (item.source.kind !== 'cyfs-path') throw new Error('only a file of the Zone can be read')
  files ??= { client: new NfspClient({ baseUrl: location.origin, sessionToken }) }
  const { client } = files
  files.hello ??= client.hello().catch((error: unknown) => {
    if (files) files.hello = undefined
    throw error
  })
  await files.hello
  const info = await client.resolve(item.source.path, ['base'])
  if (info.kind !== 'file') throw new Error('not a file')
  const node = info.node_id ?? (info.ref.type === 'live' ? info.ref.node_id : info.ref.obj_id)
  const blob = await (await client.readFile(node)).blob()
  const name = item.hints.name ?? item.source.path.slice(item.source.path.lastIndexOf('/') + 1)
  return new File([blob], name, { type: item.hints.mime ?? blob.type })
}

// Outside a Zone (`--dev`) there is no system picker: the device's own.
function pickOnDevice(accept: string): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement('input')
    input.type = 'file'
    if (accept) input.accept = accept
    input.addEventListener('change', () => resolve(input.files?.[0] ?? null), { once: true })
    input.addEventListener('cancel', () => resolve(null), { once: true })
    input.click()
  })
}

const inputAccept = (accept: ContentSelector[]): string =>
  accept.flatMap((s) => [...(s.ext ?? []), ...(s.mime === undefined ? [] : Array.isArray(s.mime) ? s.mime : [s.mime])]).join(',')

/**
 * One file chosen in the system file picker: from the Zone first, or from this device (the picker's "This device").
 * Resolves with null when the user cancels.
 */
export async function pickFile(accept: ContentSelector[] = []): Promise<File | null> {
  const client = await startAppFrame()
  if (!client) return pickOnDevice(inputAccept(accept))
  let result
  try {
    result = await client.selectContent({ mode: 'open', accept, allowLocal: true }, {
      locale: lang === 'zh' ? 'zh-CN' : 'en',
      theme: matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
    })
  } catch (error) {
    if (error instanceof SystemDialogError && error.code === 'unsupported') return pickOnDevice(inputAccept(accept))
    throw error
  }
  if (result.action !== 'selected' || !result.items.length) return null
  const item = result.items[0]
  if (item.origin === 'local') return item.kind === 'file' ? item.file : null
  return readZoneFile(item)
}
