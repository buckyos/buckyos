import { test, expect } from '@playwright/test'
import { pathToFileURL } from 'node:url'
import { copyPathText, crumbsForUrl, deviceUrl, displayPath, fileUriOf, normalizeUrl, parentUrl, parseDeviceUrl, pathOfFileUri } from '../../../src/app/filebrowser/data/urls'

const S = 'w1'

test('Copy path gives a computer location as the standard file URI Node would build', () => {
  for (const path of [
    '/Users/alice/My Doc.txt',
    '/Users/alice/文档/照片 2024.jpg',
    '/tmp/C# notes?100%.md',
    '/tmp/a&b=c+d,e;f@g$h[i]^j|k`l{m}~n\'o!(p)*q',
    '/tmp/tab\tand\\backslash',
  ]) {
    expect(copyPathText(deviceUrl(S, path))).toBe(pathToFileURL(path).href)
  }
  expect(copyPathText(deviceUrl(S, '/'))).toBe('file:///')
  expect(copyPathText(deviceUrl(S, '/C:/Users/alice/a b.txt'))).toBe('file:///C:/Users/alice/a%20b.txt')
  expect(copyPathText(deviceUrl(S, '/C:'))).toBe('file:///C:/')
  // DFS paths keep their own readable cyfs:/// form.
  expect(copyPathText('/home/alice/a b.txt')).toBe('cyfs:///home/alice/a b.txt')
})

test('A pasted file URI, encoded or not, names the same computer path', () => {
  expect(pathOfFileUri('file:///Users/alice/My%20Doc.txt')).toBe('/Users/alice/My Doc.txt')
  expect(pathOfFileUri('file:///Users/alice/My Doc.txt')).toBe('/Users/alice/My Doc.txt')
  expect(pathOfFileUri(pathToFileURL('/tmp/中文 #1%?.txt').href)).toBe('/tmp/中文 #1%?.txt')
  expect(pathOfFileUri('file:///C:/Users/alice/')).toBe('/C:/Users/alice')
  expect(pathOfFileUri('file://localhost/etc/hosts')).toBe('/etc/hosts')
  expect(pathOfFileUri('file:///')).toBe('/')
  expect(pathOfFileUri('file://server/share/a')).toBeNull()
  expect(pathOfFileUri('/home/alice')).toBeNull()
  for (const path of ['/Users/a b/文档#1/100%.txt', '/C:/x y']) {
    expect(pathOfFileUri(fileUriOf(path))).toBe(path)
  }
})

test('Computer locations display as readable file URIs and walk up to the computer root', () => {
  const url = deviceUrl(S, '/Users/alice/My Doc')
  expect(url).toBe('device://w1/Users/alice/My Doc')
  expect(normalizeUrl('device://w1/Users/alice/')).toBe('device://w1/Users/alice')
  expect(parseDeviceUrl(url)).toEqual({ session: S, path: '/Users/alice/My Doc' })
  expect(displayPath(url)).toBe('file:///Users/alice/My Doc')
  expect(displayPath(deviceUrl(S))).toBe('file:///')
  expect(parentUrl(url)).toBe('device://w1/Users/alice')
  expect(parentUrl(deviceUrl(S, '/Users'))).toBe('device://w1')
  expect(parentUrl(deviceUrl(S))).toBeNull()
  expect(crumbsForUrl(deviceUrl(S, '/C:/Users'), undefined, 'Studio PC').map((crumb) => crumb.label)).toEqual(['Studio PC', 'C:', 'Users'])
})
