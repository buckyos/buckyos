import { IconButton, Tab, Tabs } from '@mui/material'
import {
  Camera,
  Clock,
  FolderClosed,
  Hash,
  Info,
  Link2,
  MapPin,
  MessageSquare,
  Share2,
  Sparkles,
  Wand2,
  X,
} from 'lucide-react'
import { useState } from 'react'
import { useI18n } from '../../i18n/provider'
import { formatBytes, formatDate, kindIcon } from './MainContent'
import type { FileEntry, Topic } from './types'

interface PreviewPanelProps {
  entry: FileEntry | null
  topics: Topic[]
  onClose?: () => void
  onJumpToTopic: (topicId: string) => void
  onJumpToPath: (path: string) => void
  embedded?: boolean
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[84px_1fr] items-start gap-2 text-[12px] leading-5">
      <span className="text-[color:var(--cp-muted)]">{label}</span>
      <span className="min-w-0 break-words text-[color:var(--cp-text)]">{children}</span>
    </div>
  )
}

export function PreviewPanel({
  entry,
  topics,
  onClose,
  onJumpToTopic,
  onJumpToPath,
  embedded = false,
}: PreviewPanelProps) {
  const { t } = useI18n()
  const [tab, setTab] = useState(0)

  if (!entry) {
    return (
      <div className="flex h-full w-full flex-col items-center justify-center gap-2 p-6 text-center text-[color:var(--cp-muted)]">
        <Info size={24} className="opacity-60" />
        <p className="text-sm">
          {t('filebrowser.preview.empty', 'Select a file to inspect its meta and story.')}
        </p>
      </div>
    )
  }

  const topicChips = (entry.topicIds ?? [])
    .map((id) => topics.find((topic) => topic.id === id))
    .filter((topic): topic is Topic => !!topic)

  return (
    <div className="flex h-full w-full flex-col">
      <div className="flex items-start gap-3 border-b border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] px-4 py-3">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-[16px] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_86%,transparent)]">
          {kindIcon(entry.kind, 22)}
        </div>
        <div className="min-w-0 flex-1">
          <div className="line-clamp-2 font-display text-[15px] font-semibold text-[color:var(--cp-text)]">
            {entry.name}
          </div>
          <div className="mt-0.5 truncate font-mono text-[10px] text-[color:var(--cp-muted)]">
            {entry.path}
          </div>
        </div>
        {!embedded && onClose ? (
          <IconButton size="small" onClick={onClose} aria-label={t('common.close', 'Close')}>
            <X size={14} />
          </IconButton>
        ) : null}
      </div>

      <Tabs
        value={tab}
        onChange={(_, value) => setTab(value)}
        variant="fullWidth"
        className="border-b border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)]"
      >
        <Tab label={t('filebrowser.preview.tab.meta', 'Meta')} />
        <Tab label={t('filebrowser.preview.tab.story', 'Story')} />
        <Tab label={t('filebrowser.preview.tab.triggers', 'AI')} />
      </Tabs>

      <div className="flex-1 space-y-3 overflow-y-auto px-4 py-3">
        {tab === 0 ? (
          <>
            <div className="space-y-1.5">
              <Row label={t('filebrowser.preview.size', 'Size')}>
                {entry.kind === 'folder' ? '—' : formatBytes(entry.sizeBytes)}
              </Row>
              <Row label={t('filebrowser.preview.kind', 'Kind')}>
                <span className="capitalize">{entry.kind}</span>
              </Row>
              <Row label={t('filebrowser.preview.modified', 'Modified')}>
                {formatDate(entry.modifiedAt)}
              </Row>
              <Row label={t('filebrowser.preview.source', 'Source')}>
                {entry.source?.label ?? '—'}
              </Row>
              {entry.publicUrl ? (
                <Row label={t('filebrowser.preview.publicUrl', 'Public URL')}>
                  <a
                    href={entry.publicUrl}
                    target="_blank"
                    rel="noreferrer noopener"
                    className="inline-flex items-center gap-1 font-mono text-[11px] text-[color:var(--cp-accent)] hover:underline"
                  >
                    <Link2 size={11} />
                    {entry.publicUrl}
                  </a>
                </Row>
              ) : null}
            </div>

            {entry.exif ? (
              <div className="rounded-[16px] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_86%,transparent)] p-3">
                <p className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-[color:var(--cp-muted)]">
                  <Camera size={12} /> {t('filebrowser.preview.exif', 'EXIF')}
                </p>
                <div className="space-y-1 text-[11px]">
                  {entry.exif.camera ? (
                    <Row label={t('filebrowser.preview.camera', 'Camera')}>{entry.exif.camera}</Row>
                  ) : null}
                  {entry.exif.lens ? (
                    <Row label={t('filebrowser.preview.lens', 'Lens')}>{entry.exif.lens}</Row>
                  ) : null}
                  {entry.exif.takenAt ? (
                    <Row label={t('filebrowser.preview.takenAt', 'Taken')}>
                      <span className="inline-flex items-center gap-1">
                        <Clock size={11} /> {entry.exif.takenAt}
                      </span>
                    </Row>
                  ) : null}
                  {entry.exif.location ? (
                    <Row label={t('filebrowser.preview.location', 'Location')}>
                      <span className="inline-flex items-center gap-1">
                        <MapPin size={11} /> {entry.exif.location}
                      </span>
                    </Row>
                  ) : null}
                </div>
              </div>
            ) : null}

            {entry.summary ? (
              <div className="rounded-[16px] border border-[color:color-mix(in_srgb,var(--cp-accent-soft)_50%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,var(--cp-surface))] p-3">
                <p className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-[color:var(--cp-accent)]">
                  <Sparkles size={12} /> {t('filebrowser.preview.aiSummary', 'AI summary')}
                </p>
                <p className="text-[12px] leading-5 text-[color:var(--cp-text)]">
                  {entry.summary}
                </p>
              </div>
            ) : null}

            {entry.tags?.length ? (
              <div>
                <p className="mb-1.5 text-[11px] font-semibold uppercase tracking-wider text-[color:var(--cp-muted)]">
                  {t('filebrowser.preview.tags', 'Tags')}
                </p>
                <div className="flex flex-wrap gap-1">
                  {entry.tags.map((tag) => (
                    <span
                      key={tag}
                      className="inline-flex items-center gap-1 rounded-full bg-[color:color-mix(in_srgb,var(--cp-surface-2)_90%,transparent)] px-2 py-0.5 text-[11px] text-[color:var(--cp-muted)]"
                    >
                      <Hash size={10} /> {tag}
                    </span>
                  ))}
                </div>
              </div>
            ) : null}

            {topicChips.length ? (
              <div>
                <p className="mb-1.5 text-[11px] font-semibold uppercase tracking-wider text-[color:var(--cp-muted)]">
                  {t('filebrowser.preview.topics', 'Topics')}
                </p>
                <div className="flex flex-wrap gap-1.5">
                  {topicChips.map((topic) => (
                    <button
                      key={topic.id}
                      type="button"
                      onClick={() => onJumpToTopic(topic.id)}
                      className="inline-flex items-center gap-1 rounded-full border border-[color:var(--cp-accent)] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_22%,var(--cp-surface))] px-2.5 py-0.5 text-[11px] font-semibold text-[color:var(--cp-accent)]"
                    >
                      <Sparkles size={10} /> {topic.title}
                    </button>
                  ))}
                </div>
              </div>
            ) : null}

            <button
              type="button"
              onClick={() => onJumpToPath(entry.path.split('/').slice(0, -1).join('/') || '/')}
              className="inline-flex items-center gap-1.5 rounded-full border border-[color:var(--cp-border)] px-3 py-1 text-[11px] text-[color:var(--cp-muted)] hover:border-[color:var(--cp-accent)] hover:text-[color:var(--cp-accent)]"
            >
              <FolderClosed size={12} /> {t('filebrowser.preview.reveal', 'Reveal in folder')}
            </button>
          </>
        ) : null}

        {tab === 1 ? (
          entry.story?.length ? (
            <div className="space-y-2">
              {entry.story.map((story) => (
                <div
                  key={story.id}
                  className="rounded-[16px] border border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_88%,transparent)] p-3"
                >
                  <div className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-[color:var(--cp-muted)]">
                    {story.kind === 'chat' ? (
                      <MessageSquare size={11} />
                    ) : story.kind === 'share' ? (
                      <Share2 size={11} />
                    ) : (
                      <Clock size={11} />
                    )}
                    {story.title}
                  </div>
                  <p className="mt-1.5 text-[12px] leading-5 text-[color:var(--cp-text)]">
                    {story.excerpt}
                  </p>
                  <p className="mt-1 text-[10px] text-[color:var(--cp-muted)]">
                    {formatDate(story.occurredAt)}
                    {story.source ? ` · ${story.source}` : ''}
                  </p>
                </div>
              ))}
            </div>
          ) : (
            <div className="rounded-[16px] border border-dashed border-[color:var(--cp-border)] p-4 text-center text-[12px] text-[color:var(--cp-muted)]">
              {t(
                'filebrowser.preview.storyEmpty',
                'No story attached yet. Stories appear when the file has chat, share, or session context.',
              )}
            </div>
          )
        ) : null}

        {tab === 2 ? (
          <div className="space-y-2 text-[12px] text-[color:var(--cp-muted)]">
            <div className="rounded-[16px] border border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_88%,transparent)] p-3">
              <p className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-[color:var(--cp-muted)]">
                <Wand2 size={12} className="text-[color:var(--cp-accent)]" />{' '}
                {t('filebrowser.preview.aiStatus', 'AI pipeline status')}
              </p>
              <p className="mt-2 leading-5 text-[color:var(--cp-text)]">
                {entry.triggersActive
                  ? t(
                      'filebrowser.preview.aiActive',
                      'This folder is wired to the knowledge base pipeline. New files here are semantically indexed.',
                    )
                  : t(
                      'filebrowser.preview.aiInactive',
                      'This folder is excluded from AI post-processing. Upload here stays strictly filesystem-level.',
                    )}
              </p>
            </div>
            <p className="text-[11px] leading-5">
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
