import {
  chooseWindow,
  relevanceScore,
  type PreviewRequestInfo,
  type PreviewWindowMeta,
} from '../../src/app/preview/windowPolicy.ts'

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
}

const NOW = 1_700_000_000_000

function win(overrides: Partial<PreviewWindowMeta> & { windowId: string }): PreviewWindowMeta {
  return {
    createdBy: 'auto',
    createdAt: NOW - 60_000,
    lastActiveAt: NOW - 60_000,
    pinned: false,
    allowAutoReuse: true,
    ...overrides,
  }
}

function req(overrides: Partial<PreviewRequestInfo> = {}): PreviewRequestInfo {
  return { currentKey: 'path:cyfs:///home/Pictures/z.jpg', originApp: 'files', ...overrides }
}

const smart = { windowMode: 'smart' as const, autoWindowLimit: 8 }

// 1. Same session id → reuse by jumping to the item (§13.4, §13.6).
{
  const w = win({ windowId: 'w1', sessionId: 's-1', sessionKind: 'list', originApp: 'files' })
  const decision = chooseWindow([w], req({ sessionId: 's-1', sessionKind: 'list' }), smart, NOW)
  assertEquals(decision.action, 'reuse', 'same session reuses')
  assertEquals(decision.action === 'reuse' ? decision.mode : null, 'jump', 'same session jumps')
}

// 2. Unrelated request below the cap → new window.
{
  const w = win({ windowId: 'w1', containerKey: 'path:cyfs:///home/Docs', originApp: 'messagehub' })
  const decision = chooseWindow([w], req({ containerKey: 'path:cyfs:///home/Pictures', originApp: 'files' }), smart, NOW)
  assertEquals(decision.action, 'create', 'unrelated request creates')
}

// 3. Sibling in the same container → reuse with a replaced session.
{
  const w = win({ windowId: 'w1', containerKey: 'path:cyfs:///home/Pictures', sessionKind: 'container', originApp: 'files' })
  const decision = chooseWindow([w], req({ containerKey: 'path:cyfs:///home/Pictures', sessionKind: 'container' }), smart, NOW)
  assertEquals(decision.action === 'reuse' ? decision.windowId : null, 'w1', 'same container reuses')
}

// 4. Manual and pinned windows are never targets of unrelated requests (§13.3, §13.7).
{
  const manual = win({ windowId: 'manual', createdBy: 'manual', allowAutoReuse: false, sessionId: 's-1' })
  const pinned = win({ windowId: 'pinned', pinned: true, allowAutoReuse: false, sessionId: 's-1' })
  const decision = chooseWindow([manual, pinned], req({ sessionId: 's-1' }), smart, NOW)
  assertEquals(decision.action, 'create', 'protected windows are skipped')
}

// 5. At the cap: no new automatic window; the most relevant one is reused (§13.5, §14.7).
{
  const windows = Array.from({ length: 8 }, (_, i) =>
    win({ windowId: `w${i}`, containerKey: `path:cyfs:///c${i}`, lastActiveAt: NOW - i * 1000, originApp: 'files' }),
  )
  windows[5] = win({ ...windows[5], currentMediaType: 'image/png' })
  const decision = chooseWindow(windows, req({ containerKey: 'path:cyfs:///elsewhere', mediaType: 'image/jpeg', originApp: 'files' }), smart, NOW)
  assertEquals(decision.action, 'reuse', 'cap reached → reuse')
  const chosen = decision.action === 'reuse' ? decision.windowId : ''
  // All share the origin app (30); w0 is most recent → wins the recency tiebreaker.
  assertEquals(chosen, 'w0', 'most recent related window wins ties')
}

// 6. Manual windows do not count towards the cap.
{
  const windows = [
    ...Array.from({ length: 7 }, (_, i) => win({ windowId: `a${i}`, containerKey: `path:cyfs:///c${i}` })),
    ...Array.from({ length: 5 }, (_, i) => win({ windowId: `m${i}`, createdBy: 'manual', allowAutoReuse: false })),
  ]
  const decision = chooseWindow(windows, req({ containerKey: 'path:cyfs:///new' }), smart, NOW)
  assertEquals(decision.action, 'create', 'manual windows excluded from the cap')
}

// 7. Single Window mode: everything goes to the main (oldest auto) window.
{
  const windows = [
    win({ windowId: 'old', createdAt: NOW - 10_000 }),
    win({ windowId: 'new', createdAt: NOW - 1_000 }),
    win({ windowId: 'manual', createdBy: 'manual', allowAutoReuse: false }),
  ]
  const decision = chooseWindow(windows, req(), { windowMode: 'single', autoWindowLimit: 8 }, NOW)
  assertEquals(decision.action === 'reuse' ? decision.windowId : null, 'old', 'single mode uses the main window')
}

// 8. Append: a single item from the same host joins an explicit list session.
{
  const w = win({ windowId: 'list', sessionKind: 'list', originApp: 'files', hostContext: 'dfs:///home', itemKeys: ['a', 'b'] })
  const decision = chooseWindow([w], req({ currentKey: 'c', sessionKind: 'single', originApp: 'files', hostContext: 'dfs:///home' }), { windowMode: 'smart', autoWindowLimit: 1 }, NOW)
  assertEquals(decision.action === 'reuse' ? decision.mode : null, 'append', 'single item appends to a list window at the cap')
}

// 9. Relevance ordering sanity: session > container > parent/child > origin app.
{
  const request = req({ sessionId: 's', containerKey: 'C', parentContainerKey: 'P', originApp: 'files' })
  const bySession = relevanceScore(win({ windowId: 'x', sessionId: 's' }), request, NOW)
  const byContainer = relevanceScore(win({ windowId: 'x', containerKey: 'C' }), request, NOW)
  const byParent = relevanceScore(win({ windowId: 'x', containerKey: 'P' }), request, NOW)
  const byApp = relevanceScore(win({ windowId: 'x', originApp: 'files' }), request, NOW)
  if (!(bySession > byContainer && byContainer > byParent && byParent > byApp)) {
    throw new Error(`relevance order broken: ${bySession} ${byContainer} ${byParent} ${byApp}`)
  }
}

console.log('preview-window-policy: ok')
