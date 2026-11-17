import {
  archiveObjectUrl,
  archiveRootOf,
  archiveRootUrl,
  archiveSplit,
  copyPathText,
  crumbsForUrl,
  fallbackTitle,
  parentTarget,
  parentUrl,
  parseArchiveObjectUrl,
} from '../../src/app/filebrowser/data/urls.ts'

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
}

// 1. Where a path enters an archive: only `@/archive` right after an archive file name.
assertEquals(archiveSplit('/home/a/x.zip/@/archive'), { outer: '/home/a/x.zip', member: '' }, 'root')
assertEquals(archiveSplit('/home/a/X.ZIP/@/archive/docs/a.txt'), { outer: '/home/a/X.ZIP', member: 'docs/a.txt' }, 'member, any case')
assertEquals(archiveSplit('/home/a/@/archive/x'), null, 'a folder named @ is a folder')
assertEquals(archiveSplit('/home/a/x.zip/@/content'), null, 'other branches are not archives')
assertEquals(archiveSplit('/home/a/x.zip'), null, 'the archive file itself')
assertEquals(archiveRootOf('dfs:///home/a/x.zip/@/archive/docs'), 'dfs:///home/a/x.zip/@/archive', 'root of a folder inside')

// 2. Up from an archive root goes to the archive file's folder and selects it.
assertEquals(parentUrl('dfs:///home/a/x.zip/@/archive'), 'dfs:///home/a', 'parent of the root')
assertEquals(parentUrl('dfs:///home/a/x.zip/@/archive/docs'), 'dfs:///home/a/x.zip/@/archive', 'parent inside')
assertEquals(parentTarget('dfs:///home/a/x.zip/@/archive'), { url: 'dfs:///home/a', select: '/home/a/x.zip' }, 'leaving selects the file')
assertEquals(parentTarget('dfs:///home/a/x.zip/@/archive/docs'), { url: 'dfs:///home/a/x.zip/@/archive' }, 'no selection inside')

// 3. Breadcrumbs show the archive file as one crumb; `@` and `archive` are not shown.
assertEquals(
  crumbsForUrl('dfs:///home/x.zip/@/archive/docs').map((c) => [c.label, c.url]),
  [['root', 'dfs:///'], ['home', 'dfs:///home'], ['x.zip', 'dfs:///home/x.zip/@/archive'], ['docs', 'dfs:///home/x.zip/@/archive/docs']],
  'path crumbs',
)
assertEquals(fallbackTitle('dfs:///home/x.zip/@/archive'), 'x.zip', 'tab title of a root')
assertEquals(copyPathText('/home/x.zip/@/archive/a #1.txt'), 'cyfs:///home/x.zip/@/archive/a %231.txt', 'copied path keeps the archive address')

// 4. Object archives: `archive://<objId>/archive/<member>` round-trips names with any characters.
const url = archiveObjectUrl('cyfile:abc', '/archive/文档 #1/a%b.txt')
assertEquals(parseArchiveObjectUrl(url), { objectId: 'cyfile:abc', innerPath: '/archive/文档 #1/a%b.txt' }, 'object url round trip')
assertEquals(parentUrl(archiveObjectUrl('cyfile:abc', '/archive/docs')), archiveObjectUrl('cyfile:abc'), 'object parent')
assertEquals(parentUrl(archiveObjectUrl('cyfile:abc')), null, 'no parent above an object archive')
assertEquals(crumbsForUrl(archiveObjectUrl('cyfile:abc', '/archive/docs'), 'photos.zip').map((c) => c.label), ['photos.zip', 'docs'], 'object crumbs')
assertEquals(archiveRootUrl('/home/x.zip', 'docs'), 'dfs:///home/x.zip/@/archive/docs', 'root url with member')

console.log('archive-urls: ok')
