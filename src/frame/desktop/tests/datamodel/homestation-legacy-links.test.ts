import { parseEntry, portalPath } from '../../src/app/homestation/links.ts'
Deno.test('historical and current HomeStation entries retain the same public page', () => {
  for (const scheme of ['cyfs', 'https']) {
    const entry = `${scheme}://demo.buckyos.io/home/felica/feed/@/p-46f99a9818ed42f8`
    const result = parseEntry(entry)
    if (!result || result.user !== 'felica' || result.key !== 'p-46f99a9818ed42f8') throw new Error(entry)
    if (portalPath(result.user,result.key) !== '/homestation/felica/p-46f99a9818ed42f8') throw new Error('bad page')
    if (!entry.startsWith(`${scheme}://`)) throw new Error('rewritten source')
  }
  for (const entry of ['http://demo/home/felica/feed/@/p', 'obj://homestation/home/felica/feed/@/p', 'cyfs://demo/home/felica/feed/@/p/extra']) {
    if (parseEntry(entry) !== null) throw new Error(`unexpected alias: ${entry}`)
  }
})
