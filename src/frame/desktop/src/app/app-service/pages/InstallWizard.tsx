import { useEffect, useState } from 'react'
import { useForm, useWatch } from 'react-hook-form'
import {
  AlertTriangle,
  ArrowLeft,
  CheckCircle2,
  FileArchive,
  FileJson2,
  FileUp,
  Link2,
  Loader2,
  Server,
  ShieldAlert,
} from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import type { AppServiceNav } from '../components/layout/navigation'
import { AppInstallerDialog } from '../components/installer/AppInstallerDialog'
import { FilePickerDialog } from '../components/installer/FilePickerDialog'
import { useAppServiceStore } from '../hooks/use-app-service-store'
import {
  manualInstallSourceSchema,
  type ManualInstallSourceInput,
} from '../schemas'
import type {
  InstallSourceKind,
  PickedPikgFile,
  SourceParseErrorCode,
  SourceParseResult,
} from '../mock/types'

function sourceKindLabel(kind: InstallSourceKind, t: ReturnType<typeof useI18n>['t']) {
  return t(`appService.source.kind.${kind}`, kind)
}

function SourceKindIcon({ kind }: { kind: InstallSourceKind }) {
  switch (kind) {
    case 'url-app-meta':
    case 'url-pikg':
      return <Link2 size={16} aria-hidden="true" />
    case 'app-did':
      return <ShieldAlert size={16} aria-hidden="true" />
    case 'signed-jwt':
    case 'unsigned-json':
      return <FileJson2 size={16} aria-hidden="true" />
    case 'local-pikg':
    case 'personal-server-pikg':
      return <FileArchive size={16} aria-hidden="true" />
  }
}

function parseErrorMessage(code: SourceParseErrorCode, t: ReturnType<typeof useI18n>['t']) {
  return t(`appService.source.error.${code}`, 'This input is not a supported installation source.')
}

function AnalysisResult({ result }: { result: SourceParseResult }) {
  const { t } = useI18n()
  if (!result.ok) {
    return (
      <div
        className="flex items-start gap-3 rounded-[16px] p-4"
        role="alert"
        data-testid="app-service-source-error"
        style={{ background: 'color-mix(in srgb, var(--cp-danger) 7%, var(--cp-surface))', border: '1px solid color-mix(in srgb, var(--cp-danger) 24%, var(--cp-border))' }}
      >
        <AlertTriangle size={17} className="mt-0.5 shrink-0" aria-hidden="true" style={{ color: 'var(--cp-danger)' }} />
        <div>
          <div className="text-xs font-semibold" style={{ color: 'var(--cp-danger)' }}>
            {t('appService.source.notRecognized', 'Source not recognized')}
          </div>
          <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-text)' }}>{parseErrorMessage(result.code, t)}</p>
        </div>
      </div>
    )
  }

  const warning = result.source.warningCode === 'UNSIGNED_CANDIDATE'
  const color = warning ? 'var(--cp-warning)' : 'var(--cp-success)'
  return (
    <div
      className="flex items-start gap-3 rounded-[16px] p-4"
      data-testid="app-service-source-result"
      style={{ background: `color-mix(in srgb, ${color} 7%, var(--cp-surface))`, border: `1px solid color-mix(in srgb, ${color} 24%, var(--cp-border))` }}
    >
      <span className="flex size-9 shrink-0 items-center justify-center rounded-full" style={{ background: `color-mix(in srgb, ${color} 13%, transparent)`, color }}>
        <SourceKindIcon kind={result.source.kind} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs font-semibold" style={{ color }}>{sourceKindLabel(result.source.kind, t)}</span>
          <span className="rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide" style={{ color: 'var(--cp-muted)', background: 'var(--cp-surface-2)' }}>
            {result.source.normalizedType}
          </span>
        </div>
        <p className="mt-1 truncate text-xs" style={{ color: 'var(--cp-muted)' }}>{result.source.displaySource}</p>
        {warning && (
          <p className="mt-2 text-xs leading-5" style={{ color: 'var(--cp-text)' }}>
            {t('appService.source.unsignedCandidate', 'This JSON is a candidate document, not trusted proof. Authority and owner evidence will be checked in the Installer.')}
          </p>
        )}
      </div>
    </div>
  )
}

function SourceEntry({ onBack, onResolved }: { onBack: () => void; onResolved: (taskId: string) => void }) {
  const store = useAppServiceStore()
  const { t } = useI18n()
  const form = useForm<ManualInstallSourceInput>({
    defaultValues: { sourceText: '' },
    mode: 'onBlur',
  })
  const [pickedFile, setPickedFile] = useState<PickedPikgFile | null>(null)
  const [pickerOpen, setPickerOpen] = useState(false)
  const [analyzing, setAnalyzing] = useState(false)
  const [analysis, setAnalysis] = useState<SourceParseResult | null>(null)
  const sourceText = useWatch({ control: form.control, name: 'sourceText', defaultValue: '' })
  const sourceRegistration = form.register('sourceText')
  const hasCandidate = Boolean(pickedFile || sourceText.trim())

  useEffect(() => {
    const candidate = pickedFile ?? sourceText.trim()
    if (!candidate) return

    let cancelled = false
    const timer = window.setTimeout(async () => {
      if (cancelled) return
      setAnalyzing(true)
      setAnalysis(null)
      const result = typeof candidate === 'string'
        ? manualInstallSourceSchema.safeParse({ sourceText: candidate })
        : null
      if (result && !result.success) {
        if (!cancelled) {
          setAnalysis({ ok: false, code: 'UNRECOGNIZED_INPUT' })
          setAnalyzing(false)
        }
        return
      }

      const parsed = await store.analyzeInstallSource(candidate)
      if (!cancelled) {
        setAnalysis(parsed)
        setAnalyzing(false)
      }
    }, pickedFile ? 0 : 320)

    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [pickedFile, sourceText, store])

  const selectFile = (file: PickedPikgFile) => {
    setPickedFile(file)
    setAnalysis(null)
    setAnalyzing(false)
    form.setValue('sourceText', '')
    form.clearErrors()
  }

  const chooseLocalFile = (file: File | undefined) => {
    if (!file) return
    selectFile({ location: 'device', name: file.name, sizeBytes: file.size })
  }

  const continueInstall = () => {
    if (!analysis?.ok) return
    onResolved(store.createInstallTask(analysis.source))
  }

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <button
        type="button"
        onClick={onBack}
        className="hidden min-h-11 items-center gap-2 rounded-lg pr-3 text-sm font-semibold md:inline-flex"
        style={{ color: 'var(--cp-muted)' }}
      >
        <ArrowLeft size={16} aria-hidden="true" />
        {t('appService.detail.back', 'Back to applications')}
      </button>

      <header>
        <div className="text-[10px] font-semibold uppercase tracking-[0.2em]" style={{ color: 'var(--cp-muted)' }}>
          {t('appService.source.stageOne', 'Stage 1 · Source')}
        </div>
        <h1 className="mt-2 font-display text-2xl font-semibold" style={{ color: 'var(--cp-text)' }}>
          {t('appService.source.title', 'Add an application')}
        </h1>
        <p className="mt-2 max-w-2xl text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
          {t('appService.source.body', 'Paste an App Meta URL, .pikg URL, App DID, signed JWT, or complete JSON. You can also drop or choose a .pikg package.')}
        </p>
      </header>

      <section
        className="rounded-[22px] p-4 sm:p-5"
        onDragOver={(event) => event.preventDefault()}
        onDrop={(event) => {
          event.preventDefault()
          chooseLocalFile(event.dataTransfer.files[0])
        }}
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', boxShadow: 'var(--cp-panel-shadow)' }}
      >
        <label htmlFor="app-service-source" className="text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>
          {t('appService.source.inputLabel', 'Installation source')}
        </label>
        <div className="relative mt-2">
          <textarea
            {...sourceRegistration}
            id="app-service-source"
            rows={7}
            onChange={(event) => {
              setPickedFile(null)
              setAnalysis(null)
              setAnalyzing(false)
              sourceRegistration.onChange(event)
            }}
            placeholder={t('appService.source.placeholder', 'https://apps.example/app-meta.jwt\n\ndid:cyfs:app-example\n\neyJhbGciOiJFZERTQSJ9...')}
            className="w-full resize-y rounded-[16px] px-4 py-3 text-sm leading-6 outline-none"
            style={{ color: 'var(--cp-text)', background: 'var(--cp-bg)', border: '1px solid var(--cp-border)', minHeight: '168px' }}
          />
          <div className="pointer-events-none absolute bottom-3 right-3 flex items-center gap-1.5 rounded-full px-2 py-1 text-[10px]" style={{ color: 'var(--cp-muted)', background: 'var(--cp-surface-2)' }}>
            <FileUp size={12} aria-hidden="true" />
            {t('appService.source.dropHint', 'Drop .pikg')}
          </div>
        </div>

        <div className="mt-3 grid gap-2 sm:grid-cols-2">
          <label
            htmlFor="app-service-pikg-upload"
            className="inline-flex min-h-11 cursor-pointer items-center justify-center gap-2 rounded-xl px-4 text-sm font-semibold"
            style={{ color: 'var(--cp-text)', background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
          >
            <FileUp size={16} aria-hidden="true" />
            {t('appService.source.uploadPikg', 'Upload .pikg from device')}
          </label>
          <input
            id="app-service-pikg-upload"
            data-testid="app-service-pikg-upload"
            type="file"
            accept=".pikg,application/octet-stream"
            className="sr-only"
            onChange={(event) => chooseLocalFile(event.target.files?.[0])}
          />
          <button
            type="button"
            onClick={() => setPickerOpen(true)}
            className="inline-flex min-h-11 items-center justify-center gap-2 rounded-xl px-4 text-sm font-semibold"
            style={{ color: 'var(--cp-text)', background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}
          >
            <Server size={16} aria-hidden="true" />
            {t('appService.source.choosePikg', 'Choose .pikg from Personal Server')}
          </button>
        </div>

        {pickedFile && (
          <div className="mt-3 flex items-center gap-2 rounded-xl px-3 py-2.5 text-xs" style={{ color: 'var(--cp-text)', background: 'var(--cp-surface-2)' }}>
            <FileArchive size={15} aria-hidden="true" style={{ color: 'var(--cp-accent)' }} />
            <span className="min-w-0 flex-1 truncate">{pickedFile.name}</span>
            <span className="shrink-0 tabular-nums" style={{ color: 'var(--cp-muted)' }}>{Math.max(1, Math.ceil(pickedFile.sizeBytes / 1_048_576))} MB</span>
          </div>
        )}
      </section>

      {hasCandidate && analyzing && (
        <div className="flex min-h-16 items-center gap-3 rounded-[16px] px-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
          <Loader2 size={17} className="animate-spin" aria-hidden="true" style={{ color: 'var(--cp-accent)' }} />
          <span className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
            {t('appService.source.analyzing', 'Identifying source and preparing a controlled Installer input…')}
          </span>
        </div>
      )}
      {hasCandidate && !analyzing && analysis && <AnalysisResult result={analysis} />}

      <div className="rounded-[16px] p-4" style={{ background: 'var(--cp-surface-2)', border: '1px solid var(--cp-border)' }}>
        <div className="flex items-start gap-3">
          <CheckCircle2 size={16} className="mt-0.5 shrink-0" aria-hidden="true" style={{ color: 'var(--cp-accent)' }} />
          <p className="text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
            {t('appService.source.boundaryHint', 'App Service only identifies and normalizes the source. Trust, compatibility, permissions, download, and installation are handled by the reusable System App Installer.')}
          </p>
        </div>
      </div>

      <footer className="flex justify-end border-t pt-5" style={{ borderColor: 'var(--cp-border)' }}>
        <button
          type="button"
          disabled={!hasCandidate || !analysis?.ok || analyzing}
          onClick={continueInstall}
          className="min-h-11 rounded-xl px-5 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-40"
          data-testid="app-service-source-next"
          style={{ color: 'var(--cp-surface)', background: 'var(--cp-accent)' }}
        >
          {t('appService.source.continue', 'Open System App Installer')}
        </button>
      </footer>

      {pickerOpen && (
        <FilePickerDialog
          onCancel={() => setPickerOpen(false)}
          onSelect={(file) => { selectFile(file); setPickerOpen(false) }}
        />
      )}
    </div>
  )
}

interface InstallWizardProps {
  taskId?: string
  onNavigate: (nav: AppServiceNav) => void
}

export function InstallWizard({ taskId, onNavigate }: InstallWizardProps) {
  const store = useAppServiceStore()

  if (taskId) {
    return (
      <AppInstallerDialog
        taskId={taskId}
        onBackground={() => onNavigate({ page: 'home' })}
        onChangeSource={() => { store.clearActiveTask(); onNavigate({ page: 'install' }) }}
        onClose={() => { store.clearActiveTask(); onNavigate({ page: 'home' }) }}
        onViewApp={(serviceId) => { store.clearActiveTask(); onNavigate({ page: 'detail', serviceId }) }}
      />
    )
  }

  return (
    <SourceEntry
      onBack={() => onNavigate({ page: 'home' })}
      onResolved={(resolvedTaskId) => onNavigate({ page: 'install', taskId: resolvedTaskId })}
    />
  )
}
