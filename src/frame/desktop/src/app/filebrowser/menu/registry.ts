import { commandState } from './commands'
/**
 * Context-menu registry and the built-in (default) providers.
 *
 * `fileBrowserMenuRegistry` is the shared instance the file browser uses;
 * other apps/extensions extend the menu by registering providers on it, and
 * system settings shape it through `setConfig`.
 */

import type {
  FileMenuAction,
  FileMenuConfig,
  FileMenuContext,
  FileMenuLabel,
  FileMenuProvider,
  FileMenuSection,
} from './types'

export class FileMenuRegistry {
  private providers = new Map<string, FileMenuProvider>()
  private config: FileMenuConfig = {}

  /** Register a provider; returns a disposer that unregisters it. */
  register(provider: FileMenuProvider): () => void {
    this.providers.set(provider.id, provider)
    return () => {
      this.providers.delete(provider.id)
    }
  }

  unregister(id: string) {
    this.providers.delete(id)
  }

  setConfig(config: FileMenuConfig) {
    this.config = config
  }

  getConfig(): FileMenuConfig {
    return this.config
  }

  /** Resolve the menu for one invocation context. */
  build(context: FileMenuContext): FileMenuSection[] {
    const { hiddenProviders = [], hiddenItems = [], providerOrder = {} } = this.config
    return [...this.providers.values()]
      .filter((provider) => !hiddenProviders.includes(provider.id))
      .sort(
        (a, b) => (providerOrder[a.id] ?? a.order) - (providerOrder[b.id] ?? b.order),
      )
      .filter((provider) => !provider.when || provider.when(context))
      .flatMap((provider) => provider.build(context))
      .map((section) => section.filter((item) => !hiddenItems.includes(item.id)))
      .filter((section) => section.length > 0)
      .map((section) => section.map((item) => {
        if (item.type !== 'action') return item
        const state = commandState(item.command, context)
        return { ...item, disabled: item.disabled || state.state !== 'available', disabledReason: state.reason }
      }))
  }
}

function label(
  key: string,
  fallback: string,
  vars?: Record<string, string | number>,
): FileMenuLabel {
  return { key: `filebrowser.menu.${key}`, fallback, vars }
}

function action(
  id: string,
  labelValue: FileMenuLabel,
  extra?: Partial<Omit<FileMenuAction, 'type' | 'id' | 'label'>>,
): FileMenuAction {
  return { type: 'action', id, command: extra?.command ?? id, label: labelValue, ...extra }
}

const isItem = (ctx: FileMenuContext) => ctx.target === 'item'
const isSelection = (ctx: FileMenuContext) => ctx.target === 'selection'
const isView = (ctx: FileMenuContext) => ctx.target === 'view'
const hasItems = (ctx: FileMenuContext) => isItem(ctx) || isSelection(ctx)

/** Open / preview — single item only. */
const openProvider: FileMenuProvider = {
  id: 'default.open',
  order: 100,
  when: hasItems,
  build: (ctx) => {
    if (isSelection(ctx)) {
      // Several files → one stable Preview session over exactly those files.
      if (ctx.entries.some((entry) => entry.kind === 'folder')) return []
      const count = ctx.entries.length
      return [[action('preview-selection', label('previewN', 'Preview {{count}} items', { count }), { icon: 'preview' })]]
    }
    const entry = ctx.entries[0]
    if (!entry) return []
    if (entry.kind === 'folder') {
      const section: FileMenuSection = [
        action('open', label('open', 'Open'), { icon: 'open' }),
      ]
      if (ctx.pane.canOpenInNewTab) {
        section.push(
          action('open-new-tab', label('openNewTab', 'Open in new tab'), {
            icon: 'open-new-tab',
          }),
        )
      }
      if (ctx.pane.canOpenInRightPane) {
        section.push(
          action('open-right', label('openRight', 'Open in right pane'), {
            icon: 'open-right',
          }),
        )
      }
      return [section]
    }
    return [
      [
        action('open', label('preview', 'Preview'), { icon: 'preview' }),
        action('preview-new-window', label('previewNewWindow', 'Open in new Preview window'), {
          icon: 'open-new-tab',
        }),
      ],
    ]
  },
}

/** Download / share — files and multi-selections. */
const transferProvider: FileMenuProvider = {
  id: 'default.transfer',
  order: 200,
  when: hasItems,
  build: (ctx) => {
    const section: FileMenuSection = []
    const count = ctx.entries.length
    if (isSelection(ctx)) {
      section.push(
        action('download', label('downloadN', 'Download {{count}} items', { count }), {
          icon: 'download',
        }),
      )
    } else if (ctx.entries[0]?.kind !== 'folder') {
      section.push(action('download', label('download', 'Download'), { icon: 'download' }))
    }
    if (isItem(ctx)) {
      section.push(action('share', label('share', 'Share…'), { icon: 'share' }))
    }
    return section.length ? [section] : []
  },
}

/** Clipboard — copy paths / public URL; collection items expose both paths. */
const clipboardProvider: FileMenuProvider = {
  id: 'default.clipboard',
  order: 300,
  when: hasItems,
  build: (ctx) => {
    const count = ctx.entries.length
    const item = ctx.items[0]
    const section: FileMenuSection = [
      action('cut', label('cut', 'Cut'), { icon: 'move', shortcut: 'Ctrl/⌘X' }),
      action('copy', label('copyFiles', 'Copy files'), { icon: 'copy', shortcut: 'Ctrl/⌘C' }),
      action('copy-references', label('copyReferences', 'Copy references for a collection'), { icon: 'collection' }),
      isSelection(ctx)
        ? action('copy-path', label('copyPaths', 'Copy {{count}} paths', { count }), {
            icon: 'copy',
          })
        : action(
            'copy-path',
            item?.ref
              ? label('copyOriginalPath', 'Copy original path')
              : label('copyPath', 'Copy path'),
            { icon: 'copy' },
          ),
    ]
    if (isItem(ctx) && item?.ref) {
      section.push(
        action('copy-ref-path', label('copyCollectionPath', 'Copy collection path'), {
          icon: 'copy',
        }),
      )
    }
    if (isItem(ctx) && ctx.entries[0]?.publicUrl) {
      section.push(
        action('copy-public-url', label('copyPublicUrl', 'Copy public URL'), {
          icon: 'link',
        }),
      )
    }
    section.push(action('copy-to', label('copyTo', 'Copy to…'), { icon: 'copy' }))
    if (ctx.otherPath) section.push(action('copy-other', label('copyOther', 'Copy to other pane'), { icon: 'copy', args: { path: ctx.otherPath } }))
    section.push(action('details', label('details', 'Details'), { icon: 'preview' }))
    return [section]
  },
}

/** Rename / move — storage reorganisation, folders only (real children). */
const organizeProvider: FileMenuProvider = {
  id: 'default.organize',
  order: 400,
  when: (ctx) => hasItems(ctx) && (ctx.capabilities.kind === 'folder' || !!ctx.searching),
  build: (ctx) => {
    const section: FileMenuSection = []
    if (isItem(ctx)) {
      section.push(action('rename', label('rename', 'Rename…'), { icon: 'rename' }))
    }
    section.push(action('move-to', label('moveTo', 'Move to…'), { icon: 'move' }))
    if (ctx.otherPath) section.push(action('move-other', label('moveOther', 'Move to other pane'), { icon: 'move', args: { path: ctx.otherPath } }))
    return section.length ? [section] : []
  },
}

/**
 * "Add to Collection ▸" — the core entry point for (AI-driven) organising.
 * Available on any selection in folders and views; building references never
 * moves or copies the underlying files.
 */
const addToCollectionProvider: FileMenuProvider = {
  id: 'default.add-to-collection',
  order: 450,
  when: (ctx) => hasItems(ctx) && ctx.capabilities.kind !== 'collection',
  build: (ctx) => [
    [
      {
        type: 'submenu',
        id: 'add-to-collection',
        label: label('addToCollection', 'Add to Collection'),
        icon: 'collection',
        items: [
          ...ctx.collections.map((collection) =>
            action(
              `add-to-collection:${collection.id}`,
              { key: '', fallback: collection.title },
              {
                command: 'add-to-collection',
                args: { collectionId: collection.id },
                icon: 'collection',
              },
            ),
          ),
          action('new-collection', label('newCollection', 'New collection…'), {
            icon: 'new-folder',
          }),
        ],
      },
    ],
  ],
}

/** Collection member management — order, dangling refs. */
const collectionMemberProvider: FileMenuProvider = {
  id: 'default.collection-member',
  order: 500,
  when: (ctx) => hasItems(ctx) && ctx.capabilities.kind === 'collection',
  build: (ctx) => {
    const section: FileMenuSection = []
    if (ctx.capabilities.canReorder && ctx.sortKey === 'manual') {
      section.push(
        action('move-item-up', label('moveUp', 'Move up'), { icon: 'move-up' }),
        action('move-item-down', label('moveDown', 'Move down'), { icon: 'move-down' }),
      )
    }
    if (ctx.items.some((item) => item.ref?.broken)) {
      section.push(
        action('remove-broken', label('removeBroken', 'Remove dangling reference'), {
          icon: 'broken',
          danger: true,
        }),
      )
    }
    return section.length ? [section] : []
  },
}

/** Jump from a reference (collection member, view hit, folder link) to the original. */
const jumpToOriginalProvider: FileMenuProvider = {
  id: 'default.jump-original',
  order: 550,
  when: (ctx) =>
    isItem(ctx) &&
    !!(ctx.searching || ctx.items[0]?.ref || ctx.entries[0]?.link || ctx.capabilities.kind === 'view') &&
    !ctx.items[0]?.ref?.broken &&
    !ctx.entries[0]?.link?.broken,
  build: () => [
    [
      action('jump-to-original', label('jumpToOriginal', 'Jump to original location'), {
        icon: 'jump',
      }),
    ],
  ],
}

/**
 * Destructive actions — last section. The wording follows the location's
 * removal semantics: folders destroy data, collections only drop references
 * (the original file is untouched), views offer nothing.
 */
const dangerProvider: FileMenuProvider = {
  id: 'default.danger',
  order: 900,
  when: (ctx) => hasItems(ctx) && ctx.capabilities.removal !== null,
  build: (ctx) => {
    const count = ctx.entries.length
    if (ctx.capabilities.removal === 'remove-ref') {
      return [
        [
          isSelection(ctx)
            ? action(
                'remove-from-collection',
                label('removeFromCollectionN', 'Remove {{count}} from collection', { count }),
                { icon: 'remove-ref', danger: true },
              )
            : action(
                'remove-from-collection',
                label('removeFromCollection', 'Remove from collection'),
                { icon: 'remove-ref', danger: true },
              ),
        ],
      ]
    }
    return [
      [
        isSelection(ctx)
          ? action('delete', label('deleteN', 'Delete {{count}} items', { count }), {
              icon: 'trash',
              danger: true,
            })
          : action('delete', label('delete', 'Delete'), { icon: 'trash', danger: true }),
      ],
    ]
  },
}

/** Blank-area: create — only where content can actually be stored. */
const viewCreateProvider: FileMenuProvider = {
  id: 'default.view-create',
  order: 100,
  when: (ctx) => isView(ctx) && ctx.capabilities.acceptsContent,
  build: () => [
    [
      action('new-folder', label('newFolder', 'New folder'), { icon: 'new-folder' }),
      action('upload', label('upload', 'Upload…'), { icon: 'upload' }),
      action('paste', label('paste', 'Paste'), { icon: 'copy', shortcut: 'Ctrl/⌘V' }),
    ],
  ],
}

/** Blank-area inside a collection: structure management. */
const collectionViewProvider: FileMenuProvider = {
  id: 'default.collection-view',
  order: 100,
  when: (ctx) => isView(ctx) && ctx.capabilities.kind === 'collection',
  build: () => [
    [action('add-existing', label('addExisting', 'Add existing files'), { icon: 'collection' }), action('paste', label('pasteReferences', 'Paste references'), { icon: 'copy' }), action('new-group', label('newGroup', 'New group…'), { icon: 'new-folder' })],
  ],
}

/** Blank-area: display mode + refresh. */
const viewDisplayProvider: FileMenuProvider = {
  id: 'default.view-display',
  order: 200,
  when: isView,
  build: (ctx) => [
    [
      action('view-list', label('viewList', 'View as list'), {
        icon: 'view-list',
        disabled: ctx.viewMode === 'list',
      }),
      action('view-icon', label('viewIcon', 'View as icons'), {
        icon: 'view-icon',
        disabled: ctx.viewMode === 'icon',
      }),
      action('refresh', label('refresh', 'Refresh'), { icon: 'refresh' }),
    ],
  ],
}

/** Blank-area: selection helpers + current path. */
const viewSelectProvider: FileMenuProvider = {
  id: 'default.view-select',
  order: 300,
  when: isView,
  build: (ctx) => [
    [
      action('select-loaded', label('selectLoaded', 'Select loaded {{count}} items', { count: ctx.loadedCount ?? 0 }), { icon: 'select-all' }),
      ...(!ctx.searching ? [action('select-all', label('selectEntire', 'Select all items in this folder'), { icon: 'select-all' })] : []),
      action('copy-path', label('copyCurrentPath', 'Copy current path'), { icon: 'copy' }),
    ],
  ],
}

export function createDefaultFileMenuRegistry(): FileMenuRegistry {
  const registry = new FileMenuRegistry()
  registry.register(openProvider)
  registry.register(transferProvider)
  registry.register(clipboardProvider)
  registry.register(organizeProvider)
  registry.register(addToCollectionProvider)
  registry.register(collectionMemberProvider)
  registry.register(jumpToOriginalProvider)
  registry.register(dangerProvider)
  registry.register(viewCreateProvider)
  registry.register(collectionViewProvider)
  registry.register(viewDisplayProvider)
  registry.register(viewSelectProvider)
  return registry
}

/** Shared registry instance — the file manager's extension point. */
export const fileBrowserMenuRegistry = createDefaultFileMenuRegistry()
