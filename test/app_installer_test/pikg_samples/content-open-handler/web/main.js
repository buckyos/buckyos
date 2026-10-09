import { buckyos } from 'buckyos'
import { NfspClient } from 'buckyos/nfsp'
import { AppFrameClient } from 'buckyos/app-frame'
const params = Object.fromEntries(new URLSearchParams(location.search))
document.querySelector('#url').textContent = JSON.stringify(params, null, 2)
const frame = new AppFrameClient({ shellOrigin: `${location.protocol}//sys.${location.hostname.split('.').slice(1).join('.')}`, onInit(init) {
  document.querySelector('#frame').textContent = JSON.stringify(init.launch, null, 2)
  frame.setTitle('Content handler fixture')
} })
await buckyos.initBuckyOS('content-open-handler.buckyos.bns.did')
if (!await buckyos.getAccountInfo()) await buckyos.login()
else if (params.src) {
  const client = new NfspClient({ baseUrl: location.origin, sessionToken: async () => (await buckyos.getAccountInfo())?.session_token ?? null })
  await client.hello()
  const info = await client.resolve(params.src, ['base', 'ident'])
  const response = await client.readFile(info.node_id, { range: { start: 0, end: 1023 } })
  document.querySelector('#content').textContent = await response.text()
}
