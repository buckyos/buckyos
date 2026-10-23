import {
  buildDesktopLayout,
  defaultSlots,
  desktopEntryTarget,
  desktopSeed,
  fileNameStem,
  parseDesktopEntry,
  resolveShortcutApp,
  sameDesktopEntries,
  uniqueFileName,
  type DesktopEntry,
} from '../../src/models/desktopFolder.ts'
import type { AppDefinition, LayoutState } from '../../src/models/ui.ts'

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
}

const deadZone = { top: 0, bottom: 8, left: 5, right: 5 }
const grid = { cols: 12, rows: 8 }

function app(id: string, extra: Partial<AppDefinition> = {}): AppDefinition {
  return {
    id,
    iconKey: id,
    labelKey: id,
    summaryKey: '',
    accent: '',
    tier: 'system',
    manifest: {
      defaultMode: 'windowed', allowMinimize: true, allowMaximize: true, allowClose: true, allowFullscreen: false,
      mobileFullscreenBehavior: 'keep_dead_zone', mobileStatusBarMode: 'standard', titleBarMode: 'system', placement: 'inplace',
    },
    ...extra,
  }
}

const builtins = ['agent-guide', 'messagehub', 'my-network', 'aiworkspace', 'ai-center', 'homestation', 'files', 'app-service',
  'settings', 'users-agents', 'task-center', 'preview', 'workflow'].map((id) => app(id))
// An installed App; Control Panel adds its shortcut when the install settings declare one.
const editor = app('editor.example.did@alice', { logicalAppId: 'editor.example.did' })
const editorShortcut = parseDesktopEntry({ name: 'Editor.lnk', kind: 'file', size: 1, mtime: 0, text: '{"app":"editor.example.did"}' })

function seedEntries(): DesktopEntry[] {
  return desktopSeed((_key, fallback) => fallback ?? _key)
    .map((item) => parseDesktopEntry({ name: item.name, kind: 'file', size: item.text.length, mtime: 0, text: item.text }))
}

function positionsOf(layout: LayoutState) {
  return Object.fromEntries(layout.pages.flatMap((page, index) => page.items.map((item) => [item.id, [index, item.x, item.y, item.w, item.h]])))
}

Deno.test('desktop entries: shortcuts, widgets, documents and folders', () => {
  assertEquals(parseDesktopEntry({ name: 'Files.lnk', kind: 'file', size: 1, mtime: 0, text: '{"app":"files"}' }),
    { name: 'Files.lnk', kind: 'app', app: 'files', raw: { app: 'files' } }, 'shortcut')
  assertEquals(parseDesktopEntry({ name: 'Notes.WIDGET', kind: 'file', size: 1, mtime: 0, text: '{"widget":"notepad","text":"hi"}' }).kind,
    'widget', 'widget, any case')
  // Content that is not understood leaves the plain file.
  assertEquals(parseDesktopEntry({ name: 'Broken.lnk', kind: 'file', size: 3, mtime: 5, text: 'bad' }),
    { name: 'Broken.lnk', kind: 'file', size: 3, mtime: 5 }, 'broken shortcut')
  assertEquals(parseDesktopEntry({ name: 'x.lnk', kind: 'file', size: 3, mtime: 5, text: '["files"]' }).kind, 'file', 'array is not a shortcut')
  assertEquals(parseDesktopEntry({ name: 'Projects', kind: 'dir', size: 0, mtime: 5 }).kind, 'folder', 'folder')
  assertEquals(parseDesktopEntry({ name: 'Old.lnk', kind: 'dir', size: 0, mtime: 5 }).kind, 'folder', 'a folder named .lnk')
})

Deno.test('shortcuts open a built-in id, an AppId or an instance id', () => {
  const apps = [...builtins, editor]
  assertEquals(resolveShortcutApp('files', apps), 'files', 'built-in')
  assertEquals(resolveShortcutApp('editor.example.did', apps), editor.id, 'AppId')
  assertEquals(resolveShortcutApp(editor.id, apps), editor.id, 'instance id')
  assertEquals(resolveShortcutApp('content-store', [app('content-store@a', { logicalAppId: 'content-store' })]), 'content-store@a', 'alias source')
  assertEquals(resolveShortcutApp('market', [app('content-store@a', { logicalAppId: 'content-store' })]), 'content-store@a', 'catalog alias')
  assertEquals(resolveShortcutApp('missing', apps), undefined, 'unknown app')
})

Deno.test('the seed: the clock, the getting-started note and the default shortcuts, one file each', () => {
  const seed = desktopSeed((key, fallback) => (key === 'desktop.guide.text' ? 'GUIDE' : fallback ?? key))
  assertEquals(seed.length, 15, 'item count')
  assertEquals(seed[0], { name: 'Clock.widget', text: '{\n  "widget": "clock"\n}\n' }, 'clock')
  assertEquals(JSON.parse(seed[1].text), { widget: 'notepad', text: 'GUIDE' }, 'note text')
  assertEquals(new Set(seed.map((item) => item.name.toLowerCase())).size, 15, 'names are unique')
  // Localized labels with separators stay one file name.
  assertEquals(fileNameStem('Users / Agents', 'x'), 'Users Agents', 'separator')
  assertEquals(fileNameStem('..', 'Fallback'), 'Fallback', 'dot name')
  assertEquals(uniqueFileName('Files', '.lnk', new Set(['files.lnk', 'Files (2).lnk'])), 'Files (3).lnk', 'numbering ignores case')
})

Deno.test('a device without saved positions shows the default arrangement', () => {
  const layout = buildDesktopLayout({
    entries: [...seedEntries(), editorShortcut], folderPath: '/home/alice/Desktop', apps: [...builtins, editor], formFactor: 'desktop', deadZone, positions: null, ...grid,
  })
  const at = positionsOf(layout)
  assertEquals(at['Clock.widget'], [0, 0, 0, 2, 1], 'clock')
  assertEquals(at['Getting Started.widget'], [0, 0, 1, 2, 3], 'note')
  assertEquals([at['Jarvis.lnk'], at['MessageHub.lnk'], at['My Network.lnk']], [[0, 2, 0, 1, 1], [0, 3, 0, 1, 1], [0, 4, 0, 1, 1]], 'row 1')
  assertEquals(at['Task Center.lnk'], [0, 6, 2, 1, 1], 'row 3 end')
  // Installed Apps' shortcuts sit between Preview and Workflow.
  assertEquals([at['Preview.lnk'], at['Editor.lnk'], at['Workflow.lnk']], [[1, 0, 0, 1, 1], [1, 1, 0, 1, 1], [1, 2, 0, 1, 1]], 'second screen')
  assertEquals(layout.pages.length, 2, 'two screens')
  // Default items carry their slot (column-major on the desktop grid) and order.
  const messageHub = layout.pages[0].items.find((item) => item.id === 'MessageHub.lnk')
  assertEquals([messageHub?.slotIndex, messageHub?.preferredPage, messageHub?.placementType, typeof messageHub?.seq],
    [3 * grid.rows, 0, 'auto', 'number'], 'slot fields')
})

Deno.test('missing apps leave no gap; shortcuts to them stay hidden', () => {
  // A limited account has no Jarvis guide, and the shortcut's App is not installed.
  const apps = builtins.filter((entry) => entry.id !== 'agent-guide')
  const layout = buildDesktopLayout({ entries: [...seedEntries(), editorShortcut], folderPath: '/d', apps, formFactor: 'desktop', deadZone, positions: null, ...grid })
  const at = positionsOf(layout)
  assertEquals(at['Jarvis.lnk'], undefined, 'hidden shortcut')
  assertEquals(at['Editor.lnk'], undefined, 'hidden shortcut to an App that is not installed')
  assertEquals([at['MessageHub.lnk'], at['My Network.lnk']], [[0, 2, 0, 1, 1], [0, 3, 0, 1, 1]], 'row closes up')
  assertEquals(at['Workflow.lnk'], [1, 1, 0, 1, 1], 'second screen closes up')
})

Deno.test('saved positions win; new items are appended; documents point into the folder', () => {
  const first = buildDesktopLayout({ entries: seedEntries(), folderPath: '/home/alice/Desktop', apps: builtins, formFactor: 'desktop', deadZone, positions: null, ...grid })
  const moved: LayoutState = {
    ...first,
    pages: first.pages.map((page) => ({ ...page, items: page.items.map((item) => (item.id === 'Settings.lnk' ? { ...item, x: 9, y: 5 } : item)) })),
  }
  const entries = [
    ...seedEntries().filter((entry) => entry.name !== 'Files.lnk'),
    parseDesktopEntry({ name: 'photo.jpg', kind: 'file', size: 10, mtime: 1 }),
  ]
  const next = buildDesktopLayout({ entries, folderPath: '/home/alice/Desktop/', apps: builtins, formFactor: 'desktop', deadZone, positions: moved, ...grid })
  const at = positionsOf(next)
  assertEquals(at['Settings.lnk'], [0, 9, 5, 1, 1], 'kept its place')
  assertEquals(at['Files.lnk'], undefined, 'deleted file is gone')
  assertEquals(at['photo.jpg'], [0, undefined, undefined, 1, 1], 'new item is placed later')
  const photo = next.pages[0].items.find((item) => item.id === 'photo.jpg')
  assertEquals(photo?.type === 'file' ? photo.path : null, '/home/alice/Desktop/photo.jpg', 'document path')
})

Deno.test('a phone has its own arrangement of the same items', () => {
  const slots = defaultSlots('mobile', seedEntries().map(desktopEntryTarget))
  assertEquals(slots.get('widget:notepad'), { target: 'widget:notepad', page: 0, x: 0, y: 1, w: 4, h: 2 }, 'note spans the width')
  assertEquals(slots.get('app:messagehub'), { target: 'app:messagehub', page: 0, x: 3, y: 0 }, 'next to the clock')
})

Deno.test('listings compare by content', () => {
  const a = seedEntries()
  const b = seedEntries()
  assertEquals(sameDesktopEntries(a, b), true, 'same')
  const edited = b.map((entry) => (entry.kind === 'widget' && entry.widget === 'notepad' ? { ...entry, raw: { ...entry.raw, text: 'x' } } : entry))
  assertEquals(sameDesktopEntries(a, edited), false, 'note text changed')
})
