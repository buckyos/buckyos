/**
 * Landing view of the Preview App when launched without content (§13.2):
 * open a CYFS path / Object ID, drop files, reopen the last session, or —
 * in the mock runtime — browse the generated sample gallery.
 */

import { FolderOpen, History, ScanEye, Settings2, Upload } from 'lucide-react'
import { useEffect, useRef, useState, type DragEvent } from 'react'
import type { ContentRef, PreviewSessionContext, PreviewSessionItemInput } from '../../components/preview/types'
import { useI18n } from '../../i18n/provider'
import { isMockRuntime } from '../../runtime'
import { usePreviewSettings } from './settings'
import type { PreviewOpenRequest } from './types'
import { previewWindowManager } from './windowManager'

const ORIGIN = { app: 'preview' }

function parseReference(input: string): ContentRef {
  const value = input.trim()
  // Object ids carry no path separators; anything else is a path.
  if (!value.includes('/') && /^[A-Za-z0-9:_-]{16,}$/.test(value)) return { kind: 'object-id', objectId: value }
  return { kind: 'cyfs-path', path: value }
}

function filesToRequest(files: File[]): PreviewOpenRequest | null {
  if (!files.length) return null
  const refs: ContentRef[] = files.map((file) => ({ kind: 'blob', value: { blob: file, name: file.name } }))
  if (refs.length === 1) return { source: refs[0], origin: ORIGIN }
  const session: PreviewSessionContext = {
    kind: 'list',
    items: refs.map((source, i) => ({ source, title: files[i].name })),
    currentIndex: 0,
    navigation: 'wrap',
  }
  return { source: refs[0], session, origin: ORIGIN }
}

export function PreviewLanding({ onOpen, onOpenSettings }: { onOpen: (request: PreviewOpenRequest) => void; onOpenSettings: () => void }) {
  const { t } = useI18n()
  const settings = usePreviewSettings()
  const [value, setValue] = useState('')
  const [dragOver, setDragOver] = useState(false)
  const [lastSession] = useState(() => (settings.restoreLastSession ? previewWindowManager.lastSession() : null))
  const fileInput = useRef<HTMLInputElement | null>(null)
  // The sample gallery only exists in the mock runtime; load it lazily so the
  // mock provider stays out of the production chunk.
  const [samples, setSamples] = useState<{ container: ContentRef; items: PreviewSessionItemInput[] } | null>(null)
  useEffect(() => {
    if (!isMockRuntime()) return
    let cancelled = false
    void import('../../components/preview/mockProvider').then((mod) => {
      if (!cancelled) setSamples({ container: mod.MOCK_SAMPLES_CONTAINER, items: mod.mockSampleItems() })
    })
    return () => {
      cancelled = true
    }
  }, [])

  const submit = () => {
    if (!value.trim()) return
    onOpen({ source: parseReference(value), origin: ORIGIN })
  }
  const handleDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    setDragOver(false)
    const request = filesToRequest([...event.dataTransfer.files])
    if (request) onOpen(request)
  }

  return (
    <div
      className="flex h-full w-full flex-col items-center overflow-y-auto bg-[color:var(--cp-bg)] px-6 py-10"
      data-testid="preview-landing"
      onDragOver={(event) => {
        event.preventDefault()
        setDragOver(true)
      }}
      onDragLeave={() => setDragOver(false)}
      onDrop={handleDrop}
    >
      <div className="flex w-full max-w-xl flex-col items-center gap-5 text-center">
        <div className="flex h-16 w-16 items-center justify-center rounded-[22px] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_28%,var(--cp-surface))] text-[color:var(--cp-accent)]">
          <ScanEye size={30} />
        </div>
        <div>
          <h1 className="font-display text-[20px] font-semibold text-[color:var(--cp-text)]">{t('apps.preview', 'Preview')}</h1>
          <p className="mt-1 text-[13px] leading-6 text-[color:var(--cp-muted)]">
            {t('previewApp.landing.body', 'Quick look at any file, CYFS path or Object ID. Content first — the toolbar appears when you move.')}
          </p>
        </div>

        <form
          className="flex w-full items-center gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            submit()
          }}
        >
          <input
            value={value}
            onChange={(event) => setValue(event.target.value)}
            placeholder={t('previewApp.landing.placeholder', 'cyfs:///home/… or an Object ID')}
            aria-label={t('previewApp.landing.placeholder', 'cyfs:///home/… or an Object ID')}
            data-testid="preview-landing-input"
            className="h-10 flex-1 rounded-full border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] px-4 font-mono text-[12px] text-[color:var(--cp-text)] outline-none focus:border-[color:var(--cp-accent)]"
          />
          <button
            type="submit"
            data-testid="preview-landing-open"
            className="h-10 rounded-full bg-[color:var(--cp-accent)] px-4 text-[13px] font-semibold text-white disabled:opacity-40"
            disabled={!value.trim()}
          >
            {t('common.open', 'Open')}
          </button>
        </form>

        <div
          className={`flex w-full flex-col items-center gap-2 rounded-[20px] border border-dashed px-6 py-6 text-[12px] text-[color:var(--cp-muted)] transition-colors ${
            dragOver ? 'border-[color:var(--cp-accent)] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_18%,var(--cp-surface))]' : 'border-[color:var(--cp-border)]'
          }`}
          data-testid="preview-landing-dropzone"
        >
          <Upload size={18} />
          <span>{t('previewApp.landing.drop', 'Drop files here to preview them (several files become one session)')}</span>
          <button
            type="button"
            onClick={() => fileInput.current?.click()}
            className="mt-1 inline-flex items-center gap-1.5 rounded-full border border-[color:var(--cp-border)] px-3 py-1 text-[12px] text-[color:var(--cp-text)] hover:border-[color:var(--cp-accent)]"
          >
            <FolderOpen size={13} /> {t('previewApp.landing.chooseFiles', 'Choose files…')}
          </button>
          <input
            ref={fileInput}
            type="file"
            multiple
            className="hidden"
            data-testid="preview-landing-file-input"
            onChange={(event) => {
              const request = filesToRequest([...(event.target.files ?? [])])
              event.target.value = ''
              if (request) onOpen(request)
            }}
          />
        </div>

        <div className="flex flex-wrap items-center justify-center gap-2">
          {lastSession ? (
            <button
              type="button"
              onClick={() => onOpen(lastSession)}
              className="inline-flex items-center gap-1.5 rounded-full border border-[color:var(--cp-border)] px-3 py-1.5 text-[12px] text-[color:var(--cp-text)] hover:border-[color:var(--cp-accent)]"
            >
              <History size={13} /> {t('previewApp.landing.restore', 'Reopen last session')}
            </button>
          ) : null}
          <button
            type="button"
            onClick={onOpenSettings}
            className="inline-flex items-center gap-1.5 rounded-full border border-[color:var(--cp-border)] px-3 py-1.5 text-[12px] text-[color:var(--cp-text)] hover:border-[color:var(--cp-accent)]"
            data-testid="preview-landing-settings"
          >
            <Settings2 size={13} /> {t('previewApp.action.settings', 'Preview settings…')}
          </button>
        </div>

        {samples?.items.length ? (
          <div className="w-full text-left">
            <p className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-[color:var(--cp-muted)]">
              {t('previewApp.landing.samples', 'Sample gallery (mock runtime)')}
            </p>
            <div className="grid grid-cols-2 gap-1.5 sm:grid-cols-3">
              {samples.items.map((item) => (
                <button
                  key={item.title}
                  type="button"
                  data-testid={`preview-sample-${item.title}`}
                  onClick={() =>
                    onOpen({
                      source: item.source,
                      session: { kind: 'container', container: samples.container, current: item.source },
                      origin: ORIGIN,
                    })
                  }
                  className="truncate rounded-xl border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] px-3 py-2 text-left font-mono text-[11px] text-[color:var(--cp-text)] hover:border-[color:var(--cp-accent)]"
                  title={item.title}
                >
                  {item.title}
                </button>
              ))}
            </div>
          </div>
        ) : null}
      </div>
    </div>
  )
}
