import { IconButton, Tab, Tabs, Tooltip } from '@mui/material'
import {
  Clock,
  Copy,
  FolderOpen,
  Info,
  Link2,
  MessageSquare,
  Share2,
  Unlink,
  X,
} from 'lucide-react'
import { useState } from 'react'
import { useI18n } from '../../i18n/provider'
import { fileIcon, formatBytes, formatDate } from './fileDisplay'
import type { FileItem } from './data/FolderReader'
import { usePreview } from './data/usePreview'
import { copyPathText, displayPath } from './data/urls'
import type { Topic } from './types'
import { levelLabel } from './dialogs/permissionLabels'

interface PreviewPanelProps {
  item: FileItem | null
  items?: FileItem[]
  onOpenFile?: (item: FileItem) => void
  topics: Topic[]
  onClose?: () => void
  onJumpToTopic: (topicId: string) => void
  onJumpToPath: (path: string) => void
  embedded?: boolean
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[88px_1fr] items-start gap-2 text-[13px] leading-5">
      <span className="text-[color:var(--cp-muted)]">{label}</span>
      <span className="min-w-0 break-words tabular-nums text-[color:var(--cp-text)]">{children}</span>
    </div>
  )
}

/** A path that wraps after its slashes instead of mid-name. */
function WrappingPath({ path }: { path: string }) {
  const parts = path.split('/')
  return <>{parts.map((part, index) => <span key={index}>{part}{index < parts.length - 1 ? <>/<wbr /></> : null}</span>)}</>
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return <p className="files-label mb-2">{children}</p>
}

/** Section skeleton — enrichment namespaces resolving (§4.5 Loading/Progress). */
function SectionSkeleton({ rows = 4 }: { rows?: number }) {
  return (
    <div className="space-y-2" data-testid="preview-skeleton">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="grid grid-cols-[88px_1fr] items-center gap-2">
          <span className="h-3 w-14 animate-pulse rounded-full bg-[color:color-mix(in_srgb,var(--cp-border)_50%,transparent)]" />
          <span
            className="h-3 animate-pulse rounded-full bg-[color:color-mix(in_srgb,var(--cp-border)_45%,transparent)]"
            style={{ width: `${72 - (i % 3) * 16}%` }}
          />
        </div>
      ))}
    </div>
  )
}

export function PreviewPanel({
  item, items = [], onOpenFile,
  topics,
  onClose,
  onJumpToTopic,
  onJumpToPath,
  embedded = false,
}: PreviewPanelProps) {
  const { t, locale } = useI18n()
  const [tab, setTab] = useState(0)
  // Enrichment resolves asynchronously — identity/name render immediately
  // from the item; meta/story sections skeleton until the state settles.
  const preview = usePreview(item, topics)
  const enriching = preview.status === 'loading'

  if (items.length > 1) {
    const known = items.filter((item) => item.entry.sizeBytes !== undefined)
    const kinds = new Map<string, number>()
    items.forEach((item) => kinds.set(item.entry.kind, (kinds.get(item.entry.kind) ?? 0) + 1))
    return (
      <div className="space-y-4 p-4">
        <p className="text-[15px] font-medium text-[color:var(--cp-text)]">{t('filebrowser.status.selectedCount', '{{count}} selected', { count: items.length })}</p>
        <div className="space-y-2">
          {[...kinds].map(([kind, count]) => <Row key={kind} label={t(`filebrowser.kind.${kind}`, kind)}>{count}</Row>)}
        </div>
        <p className="text-[13px] tabular-nums text-[color:var(--cp-muted)]">{t('filebrowser.meta.knownSize', 'Known size ({{count}}/{{total}} items)', { count: known.length, total: items.length })}: <span className="text-[color:var(--cp-text)]">{formatBytes(known.reduce((sum, item) => sum + item.entry.sizeBytes!, 0), locale)}</span></p>
      </div>
    )
  }
  if (!item) {
    return (
      <div className="flex h-full w-full flex-col items-center justify-center gap-3 p-6 text-center text-[color:var(--cp-muted)]">
        <Info size={20} strokeWidth={1.5} />
        <p className="text-[13px]">
          {t('filebrowser.preview.empty', 'Select a file to see its details')}
        </p>
      </div>
    )
  }

  const entry = item.entry
  const topicChips = preview.data?.topics ?? []

  // Reference items carry two paths: the collection-side path and the original.
  const refPath = item.ref && item.ref.refPath !== entry.path ? item.ref.refPath : null
  const linkTarget = entry.link ? displayPath(entry.link.targetUrl) : null
  const originalPath = linkTarget ?? entry.path
  const originalParent = originalPath.split('/').slice(0, -1).join('/') || '/'

  return (
    <div className="flex h-full w-full flex-col">
      <div className="flex items-start gap-3 px-4 pb-4 pt-4">
        <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-[10px] bg-[color:var(--cp-surface-2-opaque)]">
          {fileIcon(entry, 24)}
        </div>
        <div className="min-w-0 flex-1">
          <div className="line-clamp-2 break-words text-[15px] font-medium leading-5 text-[color:var(--cp-text)]">
            {entry.name}
          </div>
          {refPath ? (
            <div className="mt-1 truncate font-[family-name:var(--files-mono)] text-[12px] text-[color:var(--cp-muted)]">
              {refPath}
            </div>
          ) : null}
          <div className="mt-1 flex items-start gap-1">
            <button
              type="button"
              onClick={() => onJumpToPath(originalParent)}
              title={t('filebrowser.preview.jumpOriginal', 'Jump to original location')}
              className="min-w-0 text-left font-[family-name:var(--files-mono)] text-[12px] leading-[18px] text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)] hover:underline"
            >
              <WrappingPath path={displayPath(originalPath)} />
            </button>
            <Tooltip title={t('filebrowser.topbar.copyPath', 'Copy path')}>
              <button type="button" className="files-icon-btn -mt-[3px] !h-6 !min-w-6 !p-0" aria-label={t('filebrowser.topbar.copyPath', 'Copy path')} onClick={() => void navigator.clipboard.writeText(copyPathText(originalPath))}>
                <Copy size={14} />
              </button>
            </Tooltip>
          </div>
          {entry.link?.broken || item.ref?.broken ? (
            <span className="mt-2 inline-flex items-center gap-1.5 text-[12px] text-[color:var(--files-warning-ink)]">
              <Unlink size={14} /> {t('filebrowser.preview.brokenRef', 'Dangling reference')}
            </span>
          ) : null}
          {entry.kind !== 'folder' && onOpenFile && !entry.link?.broken && !item.ref?.broken ? (
            <button type="button" className="files-btn files-btn-primary mt-3 w-full" onClick={() => onOpenFile(item)}>{t('filebrowser.operation.openFile', 'Open file')}</button>
          ) : null}
        </div>
        {!embedded && onClose ? (
          <IconButton size="small" onClick={onClose} aria-label={t('common.close', 'Close')}>
            <X size={16} />
          </IconButton>
        ) : null}
      </div>

      <Tabs
        value={tab}
        onChange={(_, value) => setTab(value)}
        variant="fullWidth"
        className="border-y border-[color:var(--files-hairline)]"
        sx={{ minHeight: 40, '& .MuiTab-root': { minHeight: 40, fontSize: 13, textTransform: 'none', color: 'var(--cp-muted)' }, '& .Mui-selected': { color: 'var(--cp-text) !important', fontWeight: 500 }, '& .MuiTabs-indicator': { height: 2, backgroundColor: 'var(--cp-text)' } }}
      >
        <Tab label={t('filebrowser.preview.tab.meta', 'Meta')} />
        <Tab label={t('filebrowser.preview.tab.story', 'Story')} />
        <Tab label={t('filebrowser.preview.tab.triggers', 'AI')} />
      </Tabs>

      <div className="flex-1 space-y-5 overflow-y-auto px-4 py-4">
        {tab === 0 && enriching ? <SectionSkeleton rows={5} /> : null}
        {tab === 0 && !enriching ? (
          <>
            <div className="space-y-2">
              <Row label={t('filebrowser.preview.size', 'Size')}>
                {entry.kind === 'folder' ? t('filebrowser.meta.notCalculated', 'Not calculated') : formatBytes(entry.sizeBytes, locale)}
              </Row>
              <Row label={t('filebrowser.preview.kind', 'Kind')}>
                <span className="capitalize">{t(`filebrowser.kind.${entry.kind}`, entry.kind)}</span>
              </Row>
              <Row label={t('filebrowser.preview.modified', 'Modified')}>
                {formatDate(entry.modifiedAt, locale)}
              </Row>
              <Row label={t('filebrowser.preview.source', 'Source')}>
                {entry.source?.label ?? '—'}
              </Row>
              {entry.accessLevel ? (
                <Row label={t('filebrowser.permissions.yourAccess', 'Your access')}>
                  <span data-testid="preview-access-level">{levelLabel(t, entry.accessLevel)}</span>
                </Row>
              ) : null}
              {entry.publicUrl ? (
                <Row label={t('filebrowser.preview.publicUrl', 'Public URL')}>
                  <a
                    href={entry.publicUrl}
                    target="_blank"
                    rel="noreferrer noopener"
                    className="inline-flex items-start gap-1 break-all font-[family-name:var(--files-mono)] text-[12px] text-[color:var(--files-link)] hover:underline"
                  >
                    <Link2 size={14} className="mt-0.5 shrink-0" />
                    {entry.publicUrl}
                  </a>
                </Row>
              ) : null}
            </div>

            {entry.exif ? (
              <div>
                <SectionLabel>{t('filebrowser.preview.exif', 'EXIF')}</SectionLabel>
                <div className="space-y-2">
                  {entry.exif.camera ? (
                    <Row label={t('filebrowser.preview.camera', 'Camera')}>{entry.exif.camera}</Row>
                  ) : null}
                  {entry.exif.lens ? (
                    <Row label={t('filebrowser.preview.lens', 'Lens')}>{entry.exif.lens}</Row>
                  ) : null}
                  {entry.exif.takenAt ? (
                    <Row label={t('filebrowser.preview.takenAt', 'Taken')}>
                      {entry.exif.takenAt}
                    </Row>
                  ) : null}
                  {entry.exif.location ? (
                    <Row label={t('filebrowser.preview.location', 'Location')}>
                      {entry.exif.location}
                    </Row>
                  ) : null}
                </div>
              </div>
            ) : null}

            {entry.summary ? (
              <div>
                <SectionLabel>{t('filebrowser.preview.aiSummary', 'AI summary')}</SectionLabel>
                <p className="text-[13px] leading-[22px] text-[color:var(--cp-text)]">
                  {entry.summary}
                </p>
              </div>
            ) : null}

            {entry.tags?.length ? (
              <div>
                <SectionLabel>{t('filebrowser.preview.tags', 'Tags')}</SectionLabel>
                <div className="flex flex-wrap gap-1">
                  {entry.tags.map((tag) => (
                    <span key={tag} className="files-chip">
                      #{tag}
                    </span>
                  ))}
                </div>
              </div>
            ) : null}

            {topicChips.length ? (
              <div>
                <SectionLabel>{t('filebrowser.preview.topics', 'Topics')}</SectionLabel>
                <div className="flex flex-wrap gap-1">
                  {topicChips.map((topic) => (
                    <button
                      key={topic.id}
                      type="button"
                      onClick={() => onJumpToTopic(topic.id)}
                      className="files-chip !text-[color:var(--cp-text)] transition-colors hover:!bg-[color:var(--files-pressed)]"
                    >
                      {topic.title}
                    </button>
                  ))}
                </div>
              </div>
            ) : null}

            <button
              type="button"
              onClick={() => onJumpToPath(originalParent)}
              className="files-btn"
            >
              <FolderOpen size={16} /> {t('filebrowser.preview.reveal', 'Reveal in folder')}
            </button>
          </>
        ) : null}

        {tab === 1 && enriching ? <SectionSkeleton rows={3} /> : null}
        {tab === 1 && !enriching ? (
          entry.story?.length ? (
            <div className="space-y-2">
              {entry.story.map((story) => (
                <div
                  key={story.id}
                  className="rounded-[10px] border border-[color:var(--files-hairline)] p-3"
                >
                  <div className="flex items-center gap-2 text-[13px] font-medium text-[color:var(--cp-text)]">
                    <span className="shrink-0 text-[color:var(--cp-muted)]">
                      {story.kind === 'chat' ? (
                        <MessageSquare size={16} />
                      ) : story.kind === 'share' ? (
                        <Share2 size={16} />
                      ) : (
                        <Clock size={16} />
                      )}
                    </span>
                    <span className="min-w-0 truncate">{story.title}</span>
                  </div>
                  <p className="mt-1.5 text-[13px] leading-5 text-[color:var(--cp-text)]">
                    {story.excerpt}
                  </p>
                  <p className="mt-1.5 text-[12px] tabular-nums text-[color:var(--cp-muted)]">
                    {formatDate(story.occurredAt, locale)}
                    {story.source ? ` · ${story.source}` : ''}
                  </p>
                </div>
              ))}
            </div>
          ) : (
            <div className="px-2 py-6 text-center text-[13px] leading-5 text-[color:var(--cp-muted)]">
              {t(
                'filebrowser.preview.storyEmpty',
                'No story attached yet. Stories appear when the file has chat, share, or session context.',
              )}
            </div>
          )
        ) : null}

        {tab === 2 ? (
          <div className="space-y-3 text-[13px] text-[color:var(--cp-muted)]">
            <div>
              <p className="files-label mb-2 flex items-center gap-2">
                {entry.triggersActive ? <span className="files-dot" aria-hidden /> : null}
                {t('filebrowser.preview.aiStatus', 'AI pipeline status')}
              </p>
              <p className="leading-5 text-[color:var(--cp-text)]">
                {entry.triggersActive === undefined
                  ? t('filebrowser.preview.aiUnknown', 'AI processing status is unknown.')
                  : entry.triggersActive
                    ? t('filebrowser.preview.aiEnabledState', 'Applicable AI processing policy: enabled.')
                    : t('filebrowser.preview.aiDisabledState', 'Applicable AI processing policy: disabled.')}
                {' '}{t('filebrowser.preview.aiPolicyScope', 'This is the reported policy summary; file overrides and inherited folder rules may differ.')}

              </p>
            </div>
            <p className="text-[12px] leading-5">
              {t(
                'filebrowser.preview.aiHint',
                'Trigger policies ride alongside access permissions and can be managed per folder.',
              )}
            </p>
          </div>
        ) : null}
      </div>
    </div>
  )
}
