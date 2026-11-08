import { test, expect } from '@playwright/test'
import { copyPathText } from '../../../src/app/filebrowser/data/urls'

test('Copy path gives a DFS location as its readable cyfs:/// URL and other locations as-is', () => {
  expect(copyPathText('/home/alice/English for Everyone (Audio).pdf')).toBe('cyfs:///home/alice/English for Everyone (Audio).pdf')
  expect(copyPathText('dfs:///home/alice/文档')).toBe('cyfs:///home/alice/文档')
  expect(copyPathText('dfs:///')).toBe('cyfs:///')
  // Characters that would end the path of a URL are escaped.
  expect(copyPathText('/home/alice/C# notes?100%.md')).toBe('cyfs:///home/alice/C%23 notes%3F100%25.md')
  expect(copyPathText('cyfs:///home/alice/a%20b.md')).toBe('cyfs:///home/alice/a%20b.md')
  expect(copyPathText('collection://trip/day-1')).toBe('collection://trip/day-1')
  expect(copyPathText('view://recent')).toBe('view://recent')
})
