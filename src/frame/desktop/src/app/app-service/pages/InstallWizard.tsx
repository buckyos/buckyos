/* ── App Service Install Wizard ── */

import { useState, useEffect } from 'react'
import { useMediaQuery } from '@mui/material'
import {
  ArrowLeft,
  Globe,
  Hash,
  Upload,
  Shield,
  FolderOpen,
  Wifi,
  Database,
  CheckCircle2,
  Loader2,
} from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useAppServiceStore } from '../hooks/use-app-service-store'
import { AppIcon } from '../../../components/DesktopVisuals'
import type { InstallSource, InstallAppInfo } from '../mock/types'
import type { AppServiceNav } from '../components/layout/navigation'

/* ── Back button – hidden on mobile where title bar provides back ── */

function BackButton({ onClick, label }: { onClick: () => void; label: string }) {
  const isMobile = useMediaQuery('(max-width: 767px)')
  if (isMobile) return null
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex items-center gap-1.5 text-sm font-medium"
      style={{ color: 'var(--cp-muted)' }}
    >
      <ArrowLeft size={16} />
      {label}
    </button>
  )
}

/* ── Step 1: Choose source ── */

function StepSource({
  onNext,
  onBack,
}: {
  onNext: (source: InstallSource, value: string) => void
  onBack: () => void
}) {
  const { t } = useI18n()
  const [source, setSource] = useState<InstallSource>('url')
  const [value, setValue] = useState('')

  const sources: { key: InstallSource; label: string; icon: typeof Globe; placeholder: string }[] = [
    { key: 'url', label: 'URL', icon: Globe, placeholder: 'https://example.com/app/meta.json' },
    { key: 'object-id', label: t('appService.install.objectId', 'Object ID'), icon: Hash, placeholder: 'obj://abc123...' },
    { key: 'file', label: t('appService.install.uploadFile', 'Upload File'), icon: Upload, placeholder: '' },
  ]

  return (
    <div className="space-y-5">
      <BackButton onClick={onBack} label={t('appService.install.back', 'Back')} />

      <div>
        <h1 className="font-display text-xl font-semibold" style={{ color: 'var(--cp-text)' }}>
          {t('appService.install.title', 'Install App')}
        </h1>
        <p className="text-sm mt-1" style={{ color: 'var(--cp-muted)' }}>
          {t('appService.install.chooseSource', 'Choose an installation source')}
        </p>
      </div>

      {/* Source selector */}
      <div className="space-y-2">
        {sources.map((s) => (
          <button
            key={s.key}
            type="button"
            onClick={() => setSource(s.key)}
            className="flex w-full items-center gap-3 rounded-xl px-4 py-3 text-left transition-colors"
            style={{
              background: source === s.key
                ? 'color-mix(in srgb, var(--cp-accent) 8%, var(--cp-surface))'
                : 'var(--cp-surface)',
              border: source === s.key
                ? '1px solid color-mix(in srgb, var(--cp-accent) 30%, var(--cp-border))'
                : '1px solid var(--cp-border)',
            }}
          >
            <div
              className="flex h-9 w-9 items-center justify-center rounded-lg"
              style={{
                background: source === s.key
                  ? 'color-mix(in srgb, var(--cp-accent) 16%, transparent)'
                  : 'var(--cp-surface-2)',
                color: source === s.key ? 'var(--cp-accent)' : 'var(--cp-muted)',
              }}
            >
              <s.icon size={16} />
            </div>
            <span
              className="text-sm font-medium"
              style={{ color: source === s.key ? 'var(--cp-text)' : 'var(--cp-muted)' }}
            >
              {s.label}
            </span>
          </button>
        ))}
      </div>

      {/* Input area */}
      {source === 'file' ? (
        <div
          className="rounded-2xl border-2 border-dashed p-8 text-center"
          style={{
            borderColor: 'var(--cp-border)',
            color: 'var(--cp-muted)',
          }}
        >
          <Upload size={24} className="mx-auto mb-2" />
          <p className="text-sm">{t('appService.install.dropFile', 'Drop .pkg or meta.json here')}</p>
          <button
            type="button"
            onClick={() => setValue('mock-file.pkg')}
            className="mt-3 rounded-lg px-4 py-2 text-xs font-medium"
            style={{
              background: 'var(--cp-surface-2)',
              color: 'var(--cp-text)',
              border: '1px solid var(--cp-border)',
            }}
          >
            {t('appService.install.browse', 'Browse Files')}
          </button>
        </div>
      ) : (
        <input
          type="text"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder={sources.find((s) => s.key === source)?.placeholder}
          className="w-full rounded-xl px-4 py-3 text-sm outline-none"
          style={{
            background: 'var(--cp-surface)',
            color: 'var(--cp-text)',
            border: '1px solid var(--cp-border)',
          }}
        />
      )}

      <button
        type="button"
        disabled={!value && source !== 'file'}
        onClick={() => onNext(source, value || 'mock-file.pkg')}
        className="w-full rounded-xl px-4 py-3 text-sm font-medium transition-colors disabled:opacity-40"
        style={{ background: 'var(--cp-accent)', color: 'white' }}
      >
        {t('appService.install.next', 'Next')}
      </button>
    </div>
  )
}

/* ── Step 2: App Info & Permissions ── */

function StepPermissions({
  appInfo,
  onNext,
  onBack,
}: {
  appInfo: InstallAppInfo
  onNext: () => void
  onBack: () => void
}) {
  const { t } = useI18n()

  const permIcons: Record<string, typeof Shield> = {
    'File Access': FolderOpen,
    'Network Access': Wifi,
    Database: Database,
  }

  return (
    <div className="space-y-5">
      <BackButton onClick={onBack} label={t('appService.install.back', 'Back')} />

      <div>
        <h1 className="font-display text-xl font-semibold" style={{ color: 'var(--cp-text)' }}>
          {t('appService.install.title', 'Install App')}
        </h1>
      </div>

      {/* App info card */}
      <div
        className="rounded-2xl p-5"
        style={{
          background: 'var(--cp-surface)',
          border: '1px solid var(--cp-border)',
        }}
      >
        <div className="flex items-center gap-3">
          <div
            className="flex h-12 w-12 items-center justify-center rounded-xl"
            style={{
              background: 'color-mix(in srgb, var(--cp-accent) 10%, var(--cp-surface-2))',
              color: 'var(--cp-text)',
            }}
          >
            <AppIcon iconKey={appInfo.iconKey} className="!size-6" />
          </div>
          <div>
            <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
              {appInfo.name}
            </h2>
            <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              v{appInfo.version}
            </p>
          </div>
        </div>
        <p className="text-sm mt-3" style={{ color: 'var(--cp-muted)' }}>
          {appInfo.description}
        </p>
      </div>

      {/* Permissions */}
      <section>
        <h2
          className="text-xs font-semibold uppercase tracking-wide mb-2"
          style={{ color: 'var(--cp-muted)' }}
        >
          {t('appService.install.permissions', 'Permissions')}
        </h2>
        <div
          className="rounded-2xl divide-y"
          style={{
            background: 'var(--cp-surface)',
            border: '1px solid var(--cp-border)',
          }}
        >
          {appInfo.permissions.map((perm, index) => {
            const Icon = permIcons[perm.label] ?? Shield
            return (
              <div
                key={perm.label}
                className="flex items-center gap-3 px-4 py-3"
                style={{ borderTop: index === 0 ? 'none' : '1px solid var(--cp-border)' }}
              >
                <div
                  className="flex h-8 w-8 items-center justify-center rounded-lg"
                  style={{
                    background: 'color-mix(in srgb, var(--cp-warning) 12%, transparent)',
                    color: 'var(--cp-warning)',
                  }}
                >
                  <Icon size={14} />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                    {perm.label}
                  </div>
                  <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                    {perm.description}
                  </div>
                </div>
              </div>
            )
          })}
        </div>
      </section>

      <button
        type="button"
        onClick={onNext}
        className="w-full rounded-xl px-4 py-3 text-sm font-medium"
        style={{ background: 'var(--cp-accent)', color: 'white' }}
      >
        {t('appService.install.next', 'Next')}
      </button>
    </div>
  )
}

/* ── Step 3: Admin Password ── */

function StepAdminConfirm({
  onInstall,
  onBack,
}: {
  onInstall: () => void
  onBack: () => void
}) {
  const { t } = useI18n()
  const [password, setPassword] = useState('')

  return (
    <div className="space-y-5">
      <BackButton onClick={onBack} label={t('appService.install.back', 'Back')} />

      <div>
        <h1 className="font-display text-xl font-semibold" style={{ color: 'var(--cp-text)' }}>
          {t('appService.install.title', 'Install App')}
        </h1>
        <p className="text-sm mt-1" style={{ color: 'var(--cp-muted)' }}>
          {t('appService.install.adminConfirm', 'Enter admin password to proceed with installation')}
        </p>
      </div>

      <div
        className="rounded-2xl p-5"
        style={{
          background: 'var(--cp-surface)',
          border: '1px solid var(--cp-border)',
        }}
      >
        <div className="flex items-center gap-3 mb-4">
          <div
            className="flex h-10 w-10 items-center justify-center rounded-xl"
            style={{
              background: 'color-mix(in srgb, var(--cp-accent) 12%, transparent)',
              color: 'var(--cp-accent)',
            }}
          >
            <Shield size={18} />
          </div>
          <div>
            <div className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.adminPassword', 'Admin Password')}
            </div>
            <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {t('appService.install.adminPasswordHint', 'Required to install new applications')}
            </div>
          </div>
        </div>
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          placeholder="Enter admin password"
          className="w-full rounded-xl px-4 py-3 text-sm outline-none"
          style={{
            background: 'var(--cp-bg)',
            color: 'var(--cp-text)',
            border: '1px solid var(--cp-border)',
          }}
        />
      </div>

      <div className="flex gap-3">
        <button
          type="button"
          onClick={onBack}
          className="flex-1 rounded-xl px-4 py-3 text-sm font-medium"
          style={{
            background: 'var(--cp-surface-2)',
            color: 'var(--cp-text)',
            border: '1px solid var(--cp-border)',
          }}
        >
          {t('appService.install.cancel', 'Cancel')}
        </button>
        <button
          type="button"
          disabled={!password}
          onClick={onInstall}
          className="flex-1 rounded-xl px-4 py-3 text-sm font-medium transition-colors disabled:opacity-40"
          style={{ background: 'var(--cp-accent)', color: 'white' }}
        >
          {t('appService.install.startInstall', 'Start Install')}
        </button>
      </div>
    </div>
  )
}

/* ── Step 4: Installing ── */

function StepInstalling({
  onDone,
}: {
  onDone: () => void
}) {
  const { t } = useI18n()
  const [done, setDone] = useState(false)

  // Simulate install completion
  useEffect(() => {
    const timer = setTimeout(() => setDone(true), 3000)
    return () => clearTimeout(timer)
  }, [])

  return (
    <div className="space-y-5 text-center py-8">
      {done ? (
        <>
          <CheckCircle2 size={48} className="mx-auto" style={{ color: 'var(--cp-success)' }} />
          <div>
            <h2 className="font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.complete', 'Installation Complete')}
            </h2>
            <p className="text-sm mt-1" style={{ color: 'var(--cp-muted)' }}>
              {t('appService.install.completeDesc', 'The application has been installed and is starting up.')}
            </p>
          </div>
          <button
            type="button"
            onClick={onDone}
            className="rounded-xl px-6 py-3 text-sm font-medium"
            style={{ background: 'var(--cp-accent)', color: 'white' }}
          >
            {t('appService.install.done', 'Done')}
          </button>
        </>
      ) : (
        <>
          <Loader2
            size={48}
            className="mx-auto animate-spin"
            style={{ color: 'var(--cp-accent)' }}
          />
          <div>
            <h2 className="font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('appService.install.installing', 'Installing...')}
            </h2>
            <p className="text-sm mt-1" style={{ color: 'var(--cp-muted)' }}>
              {t('appService.install.installingDesc', 'Downloading and configuring the application. This may take a moment.')}
            </p>
          </div>
        </>
      )}
    </div>
  )
}

/* ── Install Wizard ── */

interface InstallWizardProps {
  onNavigate: (nav: AppServiceNav) => void
}

export function InstallWizard({ onNavigate }: InstallWizardProps) {
  const store = useAppServiceStore()
  const [step, setStep] = useState(1)
  const [appInfo, setAppInfo] = useState<InstallAppInfo | null>(null)

  const handleBack = () => onNavigate({ page: 'home' })

  const handleSourceNext = (_source: InstallSource, value: string) => {
    const info = store.parseInstallSource(value)
    if (info) {
      setAppInfo(info)
      setStep(2)
    }
  }

  const handlePermissionsNext = () => setStep(3)

  const handleInstall = () => {
    if (appInfo) {
      store.installApp(appInfo)
      setStep(4)
    }
  }

  const handleDone = () => onNavigate({ page: 'home' })

  switch (step) {
    case 1:
      return <StepSource onNext={handleSourceNext} onBack={handleBack} />
    case 2:
      return appInfo ? (
        <StepPermissions appInfo={appInfo} onNext={handlePermissionsNext} onBack={() => setStep(1)} />
      ) : null
    case 3:
      return <StepAdminConfirm onInstall={handleInstall} onBack={() => setStep(2)} />
    case 4:
      return <StepInstalling onDone={handleDone} />
    default:
      return null
  }
}
