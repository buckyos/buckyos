/**
 * Archives in Preview (notepads/archive-preview-files-todo.md T6; 7z / tar in
 * archive-preview-more-formats-todo.md): a
 * folder page built on Files' list (`MainContent`) and the viewer for members,
 * with Preview's own back stack — member → its folder → parent folder → the
 * archive root → leave Preview. Folders and members are read through NFSP
 * (`<file>/@/archive/...` paths or `ObjectRef { inner_path }` of objects).
 */

import { useMediaQuery } from '@mui/material'
import clsx from 'clsx'
import { ArrowLeft, ChevronRight, Copy, FileArchive, FolderDown, KeyRound, Lock, MoreHorizontal, SquareArrowOutUpRight, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState, type ComponentType } from 'react'
import { useI18n } from '../../i18n/provider'
import './files.css'
import { useMobileBackHandler } from '../../desktop/windows/MobileNavContext'
import { MainContent } from '../../app/filebrowser/MainContent'
import type { FileItem } from '../../app/filebrowser/data/FolderReader'
import { folderOps } from '../../app/filebrowser/data/folderOps'
import { installFileBrowserData } from '../../app/filebrowser/data/install'
import { ensureSession, nfspClient } from '../../app/filebrowser/data/nfsp/client'
import { copyPathText, crumbsForUrl, normalizeUrl } from '../../app/filebrowser/data/urls'
import { useFolderList } from '../../app/filebrowser/data/useFolderList'
import type { FileEntry, ViewMode } from '../../app/filebrowser/types'
import { useSelectContent } from '../../sysdlg'
import type { ArchiveSummary } from '../../api/nfsp_client'
import { ArchivePasswordDialog as PasswordDialog } from './ArchivePasswordDialog'
import type { ContentPreviewProps } from '../ContentPreview'
import {
  archiveParentUrl,
  archiveReferenceText,
  archiveRootOfUrl,
  archiveSourceOf,
  forgetArchivePassword,
  isArchiveRootUrl,
  locatorOfArchiveUrl,
  type ArchiveTarget,
} from './archive'
import type { ContentRef, PreviewErrorState, PreviewHostAction } from './types'

interface FolderPage {
  url: string
  selected?: string
  scroll?: number
}

const PASSWORD_CODES = new Set(['PASSWORD_REQUIRED', 'PASSWORD_INVALID'])

function memberPathOf(source: ContentRef): string | null {
  if (source.kind === 'cyfs-path') return (source as { path: string }).path.replace(/^cyfs:\/\//, '')
  if (source.kind === 'object-id') {
    const { objectId, innerPath } = source as { objectId: string; innerPath?: string }
    return innerPath ? `archive://${objectId}${innerPath.split('/').slice(1).map((s) => `/${encodeURIComponent(s)}`).join('')}` : null
  }
  return null
}

export interface ArchivePreviewProps {
  target: ArchiveTarget
  viewerProps: ContentPreviewProps
  Viewer: ComponentType<ContentPreviewProps>
  /** The archive cannot be listed: show the original as an ordinary item (download, Open with). */
  onFallback?: () => void
}

export function ArchivePreview({ target, viewerProps, Viewer, onFallback }: ArchivePreviewProps) {
  const { t } = useI18n()
  const [pages, setPages] = useState<FolderPage[]>(() => [{ url: target.dirUrl }])
  const [file, setFile] = useState<ContentRef | null>(target.file ?? null)
  const [viewerKey, setViewerKey] = useState(0)
  const [folderKey, setFolderKey] = useState(0)
  const [password, setPassword] = useState<{ name: string; url: string } | null>(null)
  const [stale, setStale] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)
  const page = pages[pages.length - 1]
  const exit = viewerProps.onRequestExit
  const select = useSelectContent()

  useEffect(() => { installFileBrowserData() }, [])
  useEffect(() => {
    if (!notice) return
    const timer = window.setTimeout(() => setNotice(null), 3200)
    return () => window.clearTimeout(timer)
  }, [notice])

  const back = useCallback(() => {
    if (file) {
      setFile(null)
      return
    }
    if (pages.length > 1) {
      setPages((all) => all.slice(0, -1))
      return
    }
    const parent = archiveParentUrl(page.url)
    if (parent) setPages([{ url: parent }])
    else exit?.()
  }, [file, pages.length, page.url, exit])
  const canBack = !!file || pages.length > 1 || !!archiveParentUrl(page.url) || !!exit
  useMobileBackHandler(canBack ? back : null)

  const remember = useCallback((patch: Partial<FolderPage>) => {
    setPages((all) => [...all.slice(0, -1), { ...all[all.length - 1], ...patch }])
  }, [])

  const openFolder = useCallback((url: string) => setPages((all) => [...all, { url: normalizeUrl(url) }]), [])
  const openFile = useCallback((item: FileItem) => {
    remember({ selected: item.key })
    setFile(archiveSourceOf(item.entry.path))
  }, [remember])

  const reopen = useCallback(() => {
    setStale(false)
    setFile(null)
    setPages([{ url: target.dirUrl }])
    void nfspClient().invalidateContainer(locatorOfArchiveUrl(target.dirUrl)).catch(() => undefined)
  }, [target.dirUrl])

  const saveToZone = useCallback(async (source: ContentRef, name: string) => {
    const result = await select({ mode: 'folder', writable: true, title: t('archive.saveToZone', 'Save to Files…') }, { id: 'archive-save' })
    if (result.action !== 'selected') return
    const folder = result.items[0]
    if (!folder || folder.origin !== 'zone' || folder.source.kind !== 'cyfs-path') return
    const parent = folder.source.path.slice('cyfs://'.length) || '/'
    try {
      await ensureSession()
      const at = source.kind === 'object-id'
        ? { type: 'object' as const, obj_id: (source as { objectId: string }).objectId, inner_path: (source as { innerPath?: string }).innerPath }
        : (source as { path: string }).path.replace(/^cyfs:\/\//, '')
      const info = await nfspClient().resolve(at, ['base', 'ident'], { cache: 'no-cache' })
      if (!info?.copy_ref) throw new Error(t('archive.error', 'This archive cannot be opened.'))
      const entry: FileEntry = { id: info.node_id ?? name, copyRef: JSON.stringify(info.copy_ref), name, kind: 'other', path: memberPathOf(source) ?? name, modifiedAt: '' }
      setNotice(t('archive.saveStarted', 'Saving to {{name}}', { name: parent }))
      const results = await folderOps().copyEntries([entry], parent)
      const failed = results.find((r) => r.status === 'failed')
      if (failed) setNotice(t('archive.saveFailed', 'Could not save: {{message}}', { message: failed.error?.fallback ?? '' }))
    } catch (error) {
      setNotice(t('archive.saveFailed', 'Could not save: {{message}}', { message: error instanceof Error ? error.message : String(error) }))
    }
  }, [select, t])

  const memberActions = useMemo<PreviewHostAction[]>(() => [
    {
      id: 'archive-save',
      label: t('archive.saveToZone', 'Save to Files…'),
      icon: <FolderDown size={15} />,
      placement: 'overflow',
      onInvoke: ({ item }) => { void saveToZone(item.source, item.title) },
    },
    {
      id: 'archive-copy-reference',
      label: t('archive.copyReference', 'Copy reference'),
      icon: <Copy size={15} />,
      placement: 'overflow',
      onInvoke: ({ item }) => {
        const path = memberPathOf(item.source)
        if (!path) return
        const text = path.startsWith('/') ? copyPathText(path) : archiveReferenceText(path)
        void navigator.clipboard?.writeText(text).then(() => setNotice(t('archive.copied', 'Copied')))
      },
    },
    {
      id: 'archive-open-window',
      label: t('archive.openInWindow', 'Preview in a new window'),
      icon: <SquareArrowOutUpRight size={15} />,
      placement: 'overflow',
      onInvoke: ({ item }) => {
        void import('../../app/preview/launch').then((m) => m.openPreviewInNewWindow({ source: item.source, origin: { app: 'preview' } }))
      },
    },
  ], [saveToZone, t])

  const onViewerError = useCallback((error: PreviewErrorState, item: Parameters<NonNullable<ContentPreviewProps['onError']>>[1]) => {
    if (PASSWORD_CODES.has(error.code) && item) setPassword({ name: item.title, url: memberPathOf(item.source) ?? page.url })
    if (error.code === 'STALE') setStale(true)
    viewerProps.onError?.(error, item)
  }, [page.url, viewerProps])

  const unlocked = useCallback(() => {
    setPassword(null)
    setViewerKey((k) => k + 1)
    setFolderKey((k) => k + 1)
  }, [])
  // An archive whose names are encrypted (7z `-mhe`): only its root takes the password.
  const askFolderPassword = useCallback(() => setPassword({ name: target.name, url: archiveRootOfUrl(page.url) }), [target.name, page.url])

  return (
    <div className="files-ui relative flex h-full w-full flex-col bg-[color:var(--cp-surface-opaque)]" data-testid="archive-preview">
      {file ? (
        <Viewer
          key={viewerKey}
          {...viewerProps}
          source={file}
          session={{ kind: 'container', container: archiveSourceOf(page.url), current: file }}
          hostActions={[...(viewerProps.hostActions ?? []), ...memberActions]}
          onRequestExit={back}
          onError={onViewerError}
          onItemChanged={(item) => {
            setFile(item.source)
            viewerProps.onItemChanged?.(item)
          }}
        />
      ) : (
        <ArchiveFolder
          key={`${page.url}#${folderKey}`}
          page={page}
          name={target.name}
          canBack={canBack}
          showClose={!!exit && !viewerProps.titleInHost}
          onBack={back}
          onClose={exit}
          onOpenFolder={openFolder}
          onOpenFile={openFile}
          onRemember={remember}
          onStale={() => setStale(true)}
          onPassword={askFolderPassword}
          onOpenAsFile={pages.length === 1 && page.url === target.dirUrl ? onFallback : undefined}
          onNotice={setNotice}
        />
      )}
      {stale ? (
        <div className="absolute inset-x-0 top-0 z-20 flex items-center justify-center gap-3 border-b border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] px-4 py-2 text-[13px]" role="alert" data-testid="archive-stale">
          <span>{t('archive.stale', 'The archive has changed since it was opened.')}</span>
          <button type="button" className="files-btn" onClick={reopen}>
            {t('archive.reopen', 'Open again')}
          </button>
        </div>
      ) : null}
      {password ? <PasswordDialog name={password.name} url={password.url} onUnlocked={unlocked} onCancel={() => setPassword(null)} /> : null}
      {notice ? (
        <div className="pointer-events-none absolute inset-x-0 bottom-6 z-30 flex justify-center" role="status">
          <span className="files-toast">{notice}</span>
        </div>
      ) : null}
    </div>
  )
}

function readViewMode(): ViewMode {
  try {
    return localStorage.getItem('files.view') === 'icon' ? 'icon' : 'list'
  } catch {
    return 'list'
  }
}

function ArchiveFolder({
  page,
  name,
  canBack,
  showClose,
  onBack,
  onClose,
  onOpenFolder,
  onOpenFile,
  onRemember,
  onStale,
  onPassword,
  onOpenAsFile,
  onNotice,
}: {
  page: FolderPage
  name: string
  canBack: boolean
  showClose: boolean
  onBack: () => void
  onClose?: () => void
  onOpenFolder: (url: string) => void
  onOpenFile: (item: FileItem) => void
  onRemember: (patch: Partial<FolderPage>) => void
  onStale: () => void
  onPassword: () => void
  onOpenAsFile?: () => void
  onNotice: (text: string) => void
}) {
  const { t } = useI18n()
  const isMobile = useMediaQuery('(max-width: 900px)')
  const list = useFolderList(page.url, 'name', 'asc')
  const [viewMode] = useState(readViewMode)
  const [selected, setSelected] = useState<Set<string>>(() => new Set(page.selected ? [page.selected] : []))
  const [summary, setSummary] = useState<ArchiveSummary | null>(null)
  const [menu, setMenu] = useState(false)
  const root = isArchiveRootUrl(page.url)

  useEffect(() => {
    if (!root) return
    let cancelled = false
    void ensureSession()
      .then(() => nfspClient().resolve(locatorOfArchiveUrl(page.url), ['base']))
      .then((info) => { if (!cancelled) setSummary(info?.attrs?.archive ?? null) })
      .catch(() => undefined)
    return () => { cancelled = true }
  }, [page.url, root])

  const listError = list.status === 'error' ? (list.error as (Error & { code?: unknown }) | null) : null
  const errorCode = typeof listError?.code === 'string' ? listError.code : listError?.name ?? ''
  useEffect(() => { if (errorCode === 'STALE') onStale() }, [errorCode, onStale])
  useEffect(() => { if (errorCode === 'PASSWORD_REQUIRED') onPassword() }, [errorCode, onPassword])

  const crumbs = useMemo(() => {
    const all = crumbsForUrl(page.url, name)
    const start = all.findIndex((c) => isArchiveRootUrl(c.url))
    return start >= 0 ? all.slice(start) : all
  }, [page.url, name])

  const open = useCallback((item: FileItem) => {
    if (item.entry.kind === 'folder') onOpenFolder(item.entry.path)
    else onOpenFile(item)
  }, [onOpenFolder, onOpenFile])

  const forget = useCallback(() => {
    setMenu(false)
    void forgetArchivePassword(page.url).then(() => onNotice(t('archive.forgotten', 'The password was forgotten')), () => undefined)
  }, [page.url, onNotice, t])

  const errorMessage = errorCode === 'ARCHIVE_CORRUPT'
    ? t('archive.corrupt', 'The archive is damaged.')
    : errorCode === 'PASSWORD_REQUIRED'
      ? t('archive.locked.title', 'This archive is encrypted')
      : errorCode === 'QUOTA_EXCEEDED'
        ? t('archive.tooLarge', 'This archive is too large to open here.')
        : errorCode === 'UNSUPPORTED'
          ? t('archive.unsupported', 'This kind of archive is not supported.')
          : list.error?.message ?? t('archive.error', 'This archive cannot be opened.')

  return (
    <div className="flex h-full w-full flex-col" data-testid="archive-folder" data-archive-url={page.url}>
      <header className="flex h-12 shrink-0 items-center gap-1 border-b border-[color:var(--files-hairline)] px-2">
        {canBack ? (
          <button type="button" aria-label={t('archive.back', 'Back')} title={t('archive.back', 'Back')} onClick={onBack} className="files-icon-btn" data-testid="archive-back">
            <ArrowLeft size={16} />
          </button>
        ) : null}
        <FileArchive size={16} className="ml-1 shrink-0" style={{ color: 'var(--files-type-amber)' }} />
        <nav className="flex min-w-0 flex-1 items-center gap-0.5 overflow-hidden pl-1 text-[13px]" aria-label={name}>
          {crumbs.map((crumb, i) => (
            <span key={crumb.url} className="flex min-w-0 items-center gap-0.5">
              {i > 0 ? <ChevronRight size={14} className="shrink-0 text-[color:var(--cp-muted)]" /> : null}
              {i === crumbs.length - 1 ? (
                <span className="truncate px-1 font-medium text-[color:var(--cp-text)]">{i === 0 ? name : crumb.label}</span>
              ) : (
                <button type="button" className="h-6 truncate rounded-[6px] px-1 text-[color:var(--cp-muted)] transition-colors hover:bg-[color:var(--files-hover)] hover:text-[color:var(--cp-text)]" onClick={() => onOpenFolder(crumb.url)}>
                  {i === 0 ? name : crumb.label}
                </button>
              )}
            </span>
          ))}
        </nav>
        {summary && summary.encrypted_entries > 0 ? <Lock size={14} className="shrink-0 text-[color:var(--cp-muted)]" aria-label={t('archive.encrypted', 'Encrypted')} /> : null}
        <div className="relative">
          <button type="button" aria-label={t('archive.more', 'More')} onClick={() => setMenu((v) => !v)} className={clsx('files-icon-btn', menu && 'is-active')} data-testid="archive-menu">
            <MoreHorizontal size={16} />
          </button>
          {menu ? (
            <div className="absolute right-0 top-10 z-30 min-w-56 rounded-[12px] border border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] p-1 text-[14px] shadow-[var(--cp-panel-shadow)]" role="menu">
              <button type="button" role="menuitem" className="flex h-9 w-full items-center gap-3 rounded-[8px] px-2 text-left hover:bg-[color:var(--files-hover)]" onClick={forget}>
                <KeyRound size={16} className="text-[color:var(--cp-muted)]" /> {t('archive.forget', 'Forget this archive\'s password')}
              </button>
            </div>
          ) : null}
        </div>
        {showClose && onClose ? (
          <button type="button" aria-label={t('common.close', 'Close')} onClick={onClose} className="files-icon-btn">
            <X size={16} />
          </button>
        ) : null}
      </header>
      {root && summary && summary.hidden_entries > 0 ? (
        <p className="border-b border-[color:var(--files-hairline)] px-4 py-2 text-[12px] text-[color:var(--cp-muted)]" data-testid="archive-hidden">
          {t('archive.hidden', '{{count}} item(s) in this archive cannot be shown (unsafe or duplicate names, or links).', { count: summary.hidden_entries })}
        </p>
      ) : null}
      <div className="min-h-0 flex-1">
        {list.status === 'error' && errorCode !== 'STALE' ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 p-10 text-center text-[14px] text-[color:var(--cp-muted)]" data-testid="archive-error" data-code={errorCode}>
            <FileArchive size={32} strokeWidth={1.5} />
            <p className="mt-1 text-[16px] font-medium text-[color:var(--cp-text)]">{errorMessage}</p>
            <div className="mt-1 flex gap-2">
              {errorCode === 'PASSWORD_REQUIRED' ? (
                <button type="button" onClick={onPassword} className="files-btn files-btn-primary" data-testid="archive-enter-password">
                  {t('archive.password.enter', 'Enter password')}
                </button>
              ) : (
                <button type="button" onClick={() => list.reload()} className="files-btn">
                  {t('filebrowser.retry', 'Retry')}
                </button>
              )}
              {onOpenAsFile ? (
                <button type="button" onClick={onOpenAsFile} className="files-btn files-btn-primary" data-testid="archive-open-as-file">
                  {t('archive.openAsFile', 'Open as a file')}
                </button>
              ) : null}
            </div>
          </div>
        ) : (
          <MainContent
            list={list}
            viewMode={viewMode}
            selectedKeys={selected}
            onSelect={(item, modifiers) => {
              // Phones: a plain tap opens (Files' rule); a desktop click selects.
              if (isMobile && !modifiers?.shift && !modifiers?.toggle) {
                open(item)
                return
              }
              setSelected(new Set([item.key]))
              onRemember({ selected: item.key })
            }}
            onOpenFolder={onOpenFolder}
            onOpenFile={onOpenFile}
            onClearSelection={() => setSelected(new Set())}
            currentUrl={page.url}
            isMobile={isMobile}
            scroll={page.scroll}
            onScroll={(scroll) => onRemember({ scroll })}
            thumbnailSource={() => null}
            readOnlyNote={t('archive.readOnly', 'Files in an archive are read-only')}
            emptyState={{
              title: crumbs.length <= 1 ? t('archive.empty.title', 'This archive is empty') : t('archive.emptyFolder.title', 'This folder is empty'),
              body: crumbs.length <= 1 ? t('archive.empty.body', 'There are no files in it.') : '',
              icon: <FileArchive size={32} strokeWidth={1.5} />,
            }}
          />
        )}
      </div>
    </div>
  )
}
