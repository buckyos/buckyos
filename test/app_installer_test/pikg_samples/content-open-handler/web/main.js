import { buckyos } from 'buckyos'
import { NfspClient } from 'buckyos/nfsp'
import { AppFrameClient, embedderOrigin, originInZone } from 'buckyos/app-frame'
const appId = 'content-open-handler.buckyos.bns.did'
const params = Object.fromEntries(new URLSearchParams(location.search))
document.querySelector('#url').textContent = JSON.stringify(params, null, 2)
await buckyos.initBuckyOS(appId)
// The system entry is the Zone root; the shell is whichever window inside the Zone frames this page.
const zone = buckyos.getZoneHostName()
const systemOrigin = `${location.protocol}//${zone}`
const frame = new AppFrameClient({ shellOrigin: embedderOrigin(origin => originInZone(origin, zone)) ?? systemOrigin, systemOrigin, appId, onInit(init) {
  document.querySelector('#frame').textContent = JSON.stringify(init.launch, null, 2)
  frame.setTitle('Content handler fixture')
} })
if (!await buckyos.getAccountInfo()) await buckyos.login()
else if (params.src) {
  const client = new NfspClient({ baseUrl: location.origin, sessionToken: async () => (await buckyos.getAccountInfo())?.session_token ?? null })
  await client.hello()
  const info = await client.resolve(params.src, ['base', 'ident'])
  const response = await client.readFile(info.node_id, { range: { start: 0, end: 1023 } })
  document.querySelector('#content').textContent = await response.text()
  // Open with… is the system's: the shell chooser when embedded, the Zone root in a new tab otherwise.
  const button = document.querySelector('#open-with')
  button.hidden = false
  button.onclick = () => frame.openContent({ source: { kind: 'cyfs-path', path: params.src } }, 'chooser', { hints: { name: info.name, size: info.size } })
}
