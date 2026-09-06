import { useEffect, useRef, useState, type ChangeEvent } from 'react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { ArrowLeft, FileArchive, FileUp, Loader2, Server } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import type { AppServiceNav } from '../components/layout/navigation'
import { FilePickerDialog } from '../components/installer/FilePickerDialog'
import { useAppServiceStore } from '../hooks/use-app-service-store'
import {
  manualInstallSourceSchema,
  type ManualInstallSourceInput,
} from '../schemas'
import type { PickedPikgFile, SourceParseResult } from '../types'
import type { AppInstallerDialogParams } from '../../../sysdlg'

export function InstallWizard({
  onNavigate,
  onOpenInstaller,
}: {
  onNavigate: (nav: AppServiceNav) => void
  onOpenInstaller: (params: AppInstallerDialogParams) => Promise<void>
}) {
  const store = useAppServiceStore()
  const { t } = useI18n()
  const form = useForm<ManualInstallSourceInput>({
    resolver: zodResolver(manualInstallSourceSchema),
    defaultValues: { sourceText: '' },
  })
  const [pickerOpen, setPickerOpen] = useState(false)
  const [advanced, setAdvanced] = useState(false)
  const [analysis, setAnalysis] = useState<SourceParseResult | null>(null)
  const [progress, setProgress] = useState<{
    state: 'preparing' | 'importing'
    percent: number
  } | null>(null)
  const [opening, setOpening] = useState(false)
  const abort = useRef<AbortController | null>(null)
  const sourceRegistration = form.register('sourceText')
  const changeSource = (
    event: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>,
  ) => {
    void sourceRegistration.onChange(event)
    abort.current?.abort()
    setAnalysis(null)
    setProgress(null)
  }
  useEffect(() => () => abort.current?.abort(), [])
  const analyze = async (input: string | PickedPikgFile) => {
    abort.current?.abort()
    const controller = new AbortController()
    abort.current = controller
    setAnalysis(null)
    const result = await store.analyzeInstallSource(
      input,
      (state, percent) => setProgress({ state, percent }),
      controller.signal,
    )
    if (controller.signal.aborted) return
    setProgress(null)
    setAnalysis(result)
  }
  const cancel = () => {
    abort.current?.abort()
    setProgress(null)
    setAnalysis({ ok: false, code: 'IMPORT_CANCELED' })
  }
  const open = async () => {
    if (!analysis?.ok || opening) return
    setOpening(true)
    const draft_id = store.createDraft(analysis.source)
    await onOpenInstaller({ draft_id })
    setOpening(false)
    setAnalysis(null)
  }
  const chooseFile = (file?: File) => {
    if (file)
      void analyze({
        location: 'device',
        name: file.name,
        sizeBytes: file.size,
        file,
      })
  }
  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <button
        type="button"
        className="hidden min-h-11 items-center gap-2 md:inline-flex"
        onClick={() => onNavigate({ page: 'home' })}
      >
        <ArrowLeft size={16} />
        {t('appService.detail.back')}
      </button>
      <header>
        <h1 className="font-display text-2xl font-semibold">
          {t('appService.source.title')}
        </h1>
        <p className="mt-2 text-sm text-[var(--cp-muted)]">
          {t('app22.source.intro')}
        </p>
      </header>
      <section
        className="space-y-4 rounded-[22px] border border-[var(--cp-border)] bg-[var(--cp-surface)] p-5"
        onDragOver={(e) => e.preventDefault()}
        onDrop={(e) => {
          e.preventDefault()
          chooseFile(e.dataTransfer.files[0])
        }}
      >
        <h2 className="flex items-center gap-2 font-semibold">
          <FileArchive size={19} />
          {t('app22.source.choose')}
        </h2>
        <div className="grid gap-3 sm:grid-cols-2">
          <label
            className="flex min-h-11 cursor-pointer items-center justify-center gap-2 rounded-xl bg-[var(--cp-accent)] px-4 py-3 text-sm font-semibold text-[var(--cp-surface)]"
            htmlFor="app-service-pikg-upload"
          >
            <FileUp size={17} />
            {t('app22.source.device')}
          </label>
          <input
            id="app-service-pikg-upload"
            data-testid="app-service-pikg-upload"
            type="file"
            accept=".pikg,application/octet-stream"
            className="sr-only"
            onChange={(e) => {
              chooseFile(e.target.files?.[0])
              e.target.value = ''
            }}
          />
          <button
            type="button"
            className="flex min-h-11 items-center justify-center gap-2 rounded-xl border border-[var(--cp-border)] px-4 text-sm font-semibold"
            onClick={() => setPickerOpen(true)}
          >
            <Server size={17} />
            {t('app22.source.server')}
          </button>
        </div>
        <p className="text-xs leading-5 text-[var(--cp-muted)]">
          {t('app22.source.mockFiles')}
        </p>
      </section>
      <form
        className="space-y-3 rounded-[22px] border border-[var(--cp-border)] bg-[var(--cp-surface)] p-5"
        onSubmit={(event) => {
          void form.handleSubmit(({ sourceText }) => analyze(sourceText))(event)
        }}
        noValidate
      >
        <label
          className="block text-sm font-semibold"
          htmlFor="app-service-source"
        >
          {t('app22.source.identifier')}
        </label>
        {!advanced && (
          <input
            id="app-service-source"
            {...sourceRegistration}
            onChange={changeSource}
            className="min-h-11 w-full rounded-xl border border-[var(--cp-border)] bg-[var(--cp-bg)] px-3"
            placeholder="did:bns:nextcloud.buckyos"
          />
        )}
        {form.formState.errors.sourceText && (
          <p role="alert">{t('app22.error.INVALID_SOURCE')}</p>
        )}
        <button
          type="submit"
          disabled={Boolean(progress)}
          className="min-h-11 rounded-xl border border-[var(--cp-border)] px-4 disabled:opacity-40"
        >
          {t('app22.source.check')}
        </button>
        <details
          className="text-sm"
          onToggle={(e) => setAdvanced(e.currentTarget.open)}
        >
          <summary className="min-h-11 cursor-pointer py-3">
            {t('app22.source.advanced')}
          </summary>
          <p className="text-xs leading-5 text-[var(--cp-muted)]">
            {t('app22.source.advancedHint')}
          </p>
          {advanced && (
            <textarea
              id="app-service-source"
              aria-label={t('app22.source.advancedInput')}
              className="mt-3 min-h-28 w-full rounded-xl border border-[var(--cp-border)] bg-[var(--cp-bg)] p-3 text-xs"
              {...sourceRegistration}
              onChange={changeSource}
            />
          )}
        </details>
      </form>
      {progress && (
        <section
          role="status"
          className="rounded-xl border border-[var(--cp-border)] p-4"
        >
          <div className="flex items-center gap-2">
            <Loader2 className="animate-spin" size={17} />
            {t(`app22.source.${progress.state}`)}
          </div>
          {progress.state === 'importing' && (
            <progress
              className="mt-3 w-full"
              value={progress.percent}
              max={100}
              aria-label={t('app22.source.importing')}
            />
          )}
          <button type="button" className="mt-2 min-h-11 px-3" onClick={cancel}>
            {t('common.cancel')}
          </button>
        </section>
      )}
      {analysis && (
        <section
          className="break-words rounded-xl border border-[var(--cp-border)] p-4 text-sm"
          role={analysis.ok ? 'status' : 'alert'}
          data-testid={
            analysis.ok
              ? 'app-service-source-result'
              : 'app-service-source-error'
          }
        >
          {analysis.ok ? (
            <>
              <p>{analysis.source.display_name}</p>
              <p className="mt-2 text-xs text-[var(--cp-muted)]">
                {t(`app22.source.kind.${analysis.source.kind}`)}
                {analysis.source.size_bytes !== null &&
                  ` · ${analysis.source.size_bytes.toLocaleString()} B`}
              </p>
            </>
          ) : (
            t(`app22.error.${analysis.code}`)
          )}
        </section>
      )}
      <footer className="flex justify-end">
        <button
          type="button"
          data-testid="app-service-source-next"
          disabled={!analysis?.ok || Boolean(progress) || opening}
          onClick={() => void open()}
          className="min-h-11 rounded-xl bg-[var(--cp-accent)] px-5 font-semibold text-[var(--cp-surface)] disabled:opacity-40"
        >
          {t('app22.source.continue')}
        </button>
      </footer>
      {pickerOpen && (
        <FilePickerDialog
          onCancel={() => {
            setPickerOpen(false)
            cancel()
          }}
          onSelect={(file) => {
            setPickerOpen(false)
            void analyze(file)
          }}
        />
      )}
    </div>
  )
}
