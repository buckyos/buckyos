import { GitCompare, Moon, Play, Save, Smartphone, Sun } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import type { ServiceRole } from '../datamodel/types'
import { useI18n } from '../i18n/provider'
import { useProviderMetadataStore } from '../state/useProviderMetadataStore'
import { useThemeMode } from '../theme/provider'
import { StatusBadge } from '../components/status/StatusBadge'

export function TopBar() {
  const { t, locale, setLocale } = useI18n()
  const { themeMode, setThemeMode } = useThemeMode()
  const {
    workspace,
    serviceRole,
    setServiceRole,
    viewMode,
    enterEdit,
    runPublishPreview,
  } = useProviderMetadataStore()
  const navigate = useNavigate()
  const data = workspace.data

  async function handlePreview() {
    await runPublishPreview()
    navigate('/publish')
  }

  function handleRoleChange(role: ServiceRole) {
    setServiceRole(role)
  }

  return (
    <header className="flex min-h-16 shrink-0 items-center justify-between gap-3 border-b border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-surface)_92%,transparent)] px-4 md:px-5">
      <div className="min-w-0">
        <div className="truncate text-sm font-bold text-[color:var(--cp-text)]">{t('app.title', 'Provider Metadata Cloud')}</div>
        <div className="hidden truncate text-xs text-[color:var(--cp-muted)] md:block">
          {t('top.revision', 'Revision')}: {data?.published_revision ?? '-'} · {t('top.pending', 'Pending changes')}: {data?.pending_changes.length ?? 0}
        </div>
      </div>
      <div className="flex min-w-0 items-center gap-2">
        <div className="hidden rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] p-1 md:flex">
          {(['tech', 'ops'] as ServiceRole[]).map((role) => (
            <button
              className={`rounded px-3 py-1.5 text-xs font-semibold ${serviceRole === role ? 'bg-[color:var(--cp-accent)] text-white' : 'text-[color:var(--cp-muted)]'}`}
              key={role}
              onClick={() => handleRoleChange(role)}
            >
              {t(`role.${role}`, role)}
            </button>
          ))}
        </div>
        <StatusBadge tone={viewMode === 'edit' ? 'warning' : 'success'}>
          {t(viewMode === 'edit' ? 'mode.edit' : 'mode.browse', viewMode)}
        </StatusBadge>
        {viewMode === 'browse' ? (
          <button className="hidden items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 py-2 text-xs font-bold text-white md:inline-flex" onClick={enterEdit}>
            <Play size={14} />
            {t('action.enterEdit', 'Enter edit')}
          </button>
        ) : (
          <button className="hidden items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 py-2 text-xs font-bold text-[color:var(--cp-text)] md:inline-flex">
            <Save size={14} />
            {t('action.saveDraft', 'Save draft')}
          </button>
        )}
        <div className="inline-flex items-center gap-1 rounded-md border border-[color:var(--cp-border)] px-2 py-1.5 text-[11px] font-semibold text-[color:var(--cp-muted)] md:hidden">
          <Smartphone size={14} />
          {t('mobile.browseOnly', 'Mobile browse only')}
        </div>
        <button className="hidden items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 py-2 text-xs font-bold text-white md:inline-flex" onClick={handlePreview}>
          <GitCompare size={14} />
          {t('action.previewPublish', 'Preview publish')}
        </button>
        <button
          aria-label={t('top.theme', 'Theme')}
          className="hidden h-9 w-9 items-center justify-center rounded-md border border-[color:var(--cp-border)] md:inline-flex"
          onClick={() => setThemeMode(themeMode === 'light' ? 'dark' : 'light')}
        >
          {themeMode === 'light' ? <Moon size={16} /> : <Sun size={16} />}
        </button>
        <select
          aria-label={t('top.language', 'Language')}
          className="h-9 rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-2 text-xs"
          value={locale}
          onChange={(event) => setLocale(event.target.value as 'en-US' | 'zh-CN')}
        >
          <option value="en-US">EN</option>
          <option value="zh-CN">中文</option>
        </select>
      </div>
    </header>
  )
}
