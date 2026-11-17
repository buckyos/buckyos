import clsx from 'clsx'
import { AlertTriangle, ChevronDown, ChevronRight, FolderClosed, Lock, RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useState, type ReactNode } from 'react'
import { useI18n } from '../../i18n/provider'
import type { FolderChildrenPage } from './data/sidebarSources'
import { onFolderChildrenChanged } from './data/sidebarSources'
import { toUiError } from './data/state'
import type { TreeFolder } from './types'

export function SectionError({ message, onRetry, indent = 0 }: { message: string; onRetry: () => void; indent?: number }) {
  const { t } = useI18n()
  return (
    <div
      className="mx-2 flex items-center gap-2 rounded-[12px] bg-[color:color-mix(in_srgb,var(--cp-warning)_12%,var(--cp-surface))] px-2.5 py-2 text-[11px] text-[color:var(--cp-warning)]"
      style={indent ? { marginLeft: indent } : undefined}
      data-testid="sidebar-section-error"
    >
      <AlertTriangle size={12} className="shrink-0" />
      <span className="min-w-0 flex-1 leading-4">{message}</span>
      <button
        type="button"
        onClick={onRetry}
        aria-label={t('filebrowser.retry', 'Retry')}
        className="shrink-0 rounded-full p-1 hover:bg-[color:color-mix(in_srgb,var(--cp-warning)_18%,transparent)]"
      >
        <RefreshCw size={12} />
      </button>
    </div>
  )
}

export interface FolderTreeRoot extends TreeFolder {
  icon?: ReactNode
  /** Not a folder that can be expanded (e.g. a collection root). */
  leaf?: boolean
  /** Open on first show (the computer's root right after unlocking). */
  defaultExpanded?: boolean
}

export interface FolderTreeProps {
  roots: FolderTreeRoot[]
  /** Canonical url of the active pane location. */
  activeUrl: string
  onNavigate: (url: string) => void
  loadChildren: (url: string, cursor?: string) => Promise<FolderChildrenPage>
  /** Where the window opened: the tree reveals no ancestors until the location moves on. */
  openedAt?: string
}

const storageKey = (url: string) => `files.tree:${url}`
const isUnder = (url: string | null, ancestor: string) => !!url && url.startsWith(ancestor.endsWith('/') ? ancestor : `${ancestor}/`)

/**
 * Lazily loaded folder tree. Everything starts collapsed; only folders the
 * user expanded by hand stay open across sessions (`files.tree:<url>` =
 * 'true'). Moving into a folder from the main area later reveals its ancestors.
 */
export function FolderTree({ roots, activeUrl, onNavigate, loadChildren, openedAt }: FolderTreeProps) {
  // The location the tree must reveal — none on first open, set on each later navigation.
  const [mountedUrl] = useState(openedAt ?? activeUrl)
  const reveal = activeUrl === mountedUrl ? null : activeUrl
  return (
    <div className="space-y-0.5" role="tree">
      {roots.map((root) => (
        <FolderTreeNode
          key={root.url}
          folder={root}
          icon={root.icon}
          leaf={root.leaf}
          defaultExpanded={root.defaultExpanded}
          depth={0}
          activeUrl={activeUrl}
          reveal={reveal}
          onNavigate={onNavigate}
          loadChildren={loadChildren}
        />
      ))}
    </div>
  )
}

interface ChildState {
  status: 'idle' | 'loading' | 'ready' | 'error'
  folders: TreeFolder[]
  nextCursor?: string
  error?: string
  loadingMore?: boolean
}

function FolderTreeNode({
  folder,
  icon,
  leaf,
  defaultExpanded,
  depth,
  activeUrl,
  reveal,
  onNavigate,
  loadChildren,
}: {
  folder: TreeFolder
  icon?: ReactNode
  leaf?: boolean
  defaultExpanded?: boolean
  depth: number
  activeUrl: string
  reveal: string | null
  onNavigate: (url: string) => void
  loadChildren: (url: string, cursor?: string) => Promise<FolderChildrenPage>
}) {
  const { t } = useI18n()
  const url = folder.url
  const [expanded, setExpanded] = useState(() => {
    try {
      const stored = localStorage.getItem(storageKey(url))
      if (stored === 'true' || isUnder(reveal, url)) return true
      if (stored === 'false') return false
    } catch { /* no storage */ }
    return !!defaultExpanded || isUnder(reveal, url)
  })
  const [seenReveal, setSeenReveal] = useState(reveal)
  if (seenReveal !== reveal) {
    setSeenReveal(reveal)
    if (isUnder(reveal, url) && !expanded) setExpanded(true)
  }
  const [children, setChildren] = useState<ChildState>({ status: 'idle', folders: [] })
  const [generation, setGeneration] = useState(0)

  const load = useCallback(() => {
    setChildren((prev) => ({ ...prev, status: 'loading', error: undefined }))
    loadChildren(url).then(
      (page) => setChildren({ status: 'ready', folders: page.folders, nextCursor: page.nextCursor }),
      (err: unknown) => {
        const ui = toUiError(err)
        setChildren({ status: 'error', folders: [], error: t(ui.messageKey, ui.fallback) })
      },
    )
  }, [loadChildren, url, t])

  useEffect(() => onFolderChildrenChanged(url, () => setGeneration((value) => value + 1)), [url])

  useEffect(() => {
    if (!expanded || leaf || folder.locked) return
    // eslint-disable-next-line react-hooks/set-state-in-effect -- kicks off the lazy load of this node's children
    load()
  }, [expanded, leaf, folder.locked, load, generation])

  const loadMore = () => {
    const cursor = children.nextCursor
    if (!cursor) return
    setChildren((prev) => ({ ...prev, loadingMore: true }))
    loadChildren(url, cursor).then(
      (page) => setChildren((prev) => ({ status: 'ready', folders: [...prev.folders, ...page.folders], nextCursor: page.nextCursor })),
      (err: unknown) => {
        const ui = toUiError(err)
        setChildren((prev) => ({ ...prev, loadingMore: false, error: t(ui.messageKey, ui.fallback) }))
      },
    )
  }

  const toggle = () => {
    const next = !expanded
    setExpanded(next)
    try { localStorage.setItem(storageKey(url), String(next)) } catch { /* no storage */ }
  }

  const active = activeUrl === url
  const empty = children.status === 'ready' && children.folders.length === 0 && !children.nextCursor
  const expandable = !leaf && !folder.locked && !empty
  const indent = depth * 12 + 8

  return (
    <div role="treeitem" aria-expanded={expandable ? expanded : undefined} aria-selected={active}>
      <div
        className={clsx(
          'group flex w-full items-center gap-1 rounded-[14px] text-sm transition',
          active
            ? 'bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_28%,var(--cp-surface))] text-[color:var(--cp-text)]'
            : 'text-[color:var(--cp-muted)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)] hover:text-[color:var(--cp-text)]',
        )}
        style={{ paddingLeft: indent }}
      >
        {expandable ? (
          <button
            type="button"
            className="flex h-7 w-5 shrink-0 items-center justify-center"
            aria-label={`${expanded ? t('filebrowser.tree.collapse', 'Collapse') : t('filebrowser.tree.expand', 'Expand')} ${folder.name}`}
            onClick={toggle}
          >
            {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          </button>
        ) : (
          <span className="w-5 shrink-0" />
        )}
        <button
          type="button"
          aria-label={folder.name}
          disabled={folder.locked}
          title={folder.locked ? t('filebrowser.tree.locked', 'You cannot open this folder') : undefined}
          className="flex min-h-8 min-w-0 flex-1 items-center gap-2 pr-2 text-left disabled:cursor-not-allowed disabled:opacity-60"
          onClick={() => onNavigate(url)}
        >
          {folder.locked ? <Lock size={14} className="shrink-0 text-[color:var(--cp-muted)]" /> : icon ?? <FolderClosed size={14} className="shrink-0 text-[color:var(--cp-muted)]" />}
          <span className="min-w-0 truncate font-medium">{folder.name}</span>
        </button>
      </div>
      {expanded && expandable ? (
        <div className="space-y-0.5" role="group">
          {children.status === 'loading' && !children.folders.length ? (
            <div className="flex items-center gap-2 py-1" style={{ paddingLeft: indent + 32 }} data-testid="tree-node-loading">
              <span className="inline-block h-3 w-24 animate-pulse rounded-full bg-[color:color-mix(in_srgb,var(--cp-border)_50%,transparent)]" />
            </div>
          ) : null}
          {children.status === 'error' ? <SectionError message={children.error ?? ''} onRetry={load} indent={indent + 20} /> : null}
          {children.folders.map((child) => (
            <FolderTreeNode
              key={child.url}
              folder={child}
              depth={depth + 1}
              activeUrl={activeUrl}
              reveal={reveal}
              onNavigate={onNavigate}
              loadChildren={loadChildren}
            />
          ))}
          {children.status === 'ready' && children.error ? <SectionError message={children.error} onRetry={loadMore} indent={indent + 20} /> : null}
          {children.nextCursor ? (
            <button
              type="button"
              className="min-h-8 w-full rounded-[12px] text-left text-xs text-[color:var(--cp-accent)] hover:underline disabled:opacity-60"
              style={{ paddingLeft: indent + 32 }}
              disabled={children.loadingMore}
              onClick={loadMore}
            >
              {t('filebrowser.tree.more', 'More…')}
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}
