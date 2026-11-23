import { Alert, Button, IconButton, Switch, Tooltip } from '@mui/material'
import { Pencil, Plus, Trash2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { fetchSsoConfig, saveOAuthSources, type OAuthSource } from '../../../../api/sso'
import { useI18n } from '../../../../i18n/provider'
import { ListCard, Section, SectionNote, StatusBadge } from '../shared/Section'
import { DeleteOAuthSourceDialog } from './DeleteOAuthSourceDialog'
import { KIND_LABELS, saveErrorText } from './model'
import { OAuthSourceDialog } from './OAuthSourceDialog'

/**
 * Settings › Sign-in & Sharing › External login providers. Every change (add, edit, switch, delete)
 * is saved at once; the page's own Save button covers only the other settings.
 */
export function OAuthSourcesSection({
  sources,
  onSaved,
}: {
  sources: OAuthSource[]
  onSaved: (sources: OAuthSource[]) => void
}) {
  const { t } = useI18n()
  const [origin, setOrigin] = useState('')
  const [dialog, setDialog] = useState<{ editing: OAuthSource | null } | null>(null)
  const [deleting, setDeleting] = useState<OAuthSource | null>(null)
  // Each opening remounts its dialog, so the dialog starts from fresh state.
  const [opening, setOpening] = useState(0)
  const openDialog = (editing: OAuthSource | null) => {
    setOpening((value) => value + 1)
    setDialog({ editing })
  }
  const openDelete = (source: OAuthSource) => {
    setOpening((value) => value + 1)
    setDeleting(source)
  }
  // The switch shows the new value while it is being saved.
  const [switching, setSwitching] = useState<{ id: string; enabled: boolean } | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let live = true
    void fetchSsoConfig().then(({ data }) => {
      if (live && data?.zone_origin) setOrigin(data.zone_origin)
    })
    return () => { live = false }
  }, [])

  const store = async (next: OAuthSource[]): Promise<string | null> => {
    const { data, error: failure } = await saveOAuthSources(next)
    if (!data) return saveErrorText(failure, t)
    onSaved(data.oauth_sources ?? [])
    return null
  }

  const toggle = async (source: OAuthSource, enabled: boolean) => {
    setSwitching({ id: source.id, enabled })
    setError(null)
    const failure = await store(sources.map((item) => (item.id === source.id ? { ...item, enabled } : item)))
    setSwitching(null)
    setError(failure)
  }

  const googleTaken = sources.some((source) => source.kind === 'google')

  return (
    <Section
      title={t('sso.provider.title', 'External login providers')}
      description={t('sso.provider.description', 'Let people sign in to this Zone with a Google, OpenID Connect or OAuth 2.0 account.')}
      testId="settings-sso-providers"
      actions={(
        <Button size="small" variant="outlined" startIcon={<Plus size={14} />} onClick={() => openDialog(null)} data-testid="sso-provider-add">
          {t('sso.provider.addShort', 'Add')}
        </Button>
      )}
    >
      <div className="space-y-2">
        {error ? <Alert severity="error" onClose={() => setError(null)}>{error}</Alert> : null}
        {sources.length === 0 ? (
          <SectionNote>{t('sso.provider.empty', 'No external login provider yet.')}</SectionNote>
        ) : sources.map((source) => (
          <ListCard key={source.id} testId={`sso-provider-row-${source.id}`}>
            <div className="flex items-center gap-2">
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                  <span className="truncate font-medium" style={{ color: 'var(--cp-text)' }}>{source.display_name}</span>
                  {!source.enabled && <StatusBadge status="neutral" label={t('sso.provider.off', 'Off')} />}
                </div>
                <p className="break-all text-sm" style={{ color: 'var(--cp-muted)' }}>
                  {source.webhost ? `${KIND_LABELS[source.kind]} · ${source.webhost}` : KIND_LABELS[source.kind]}
                </p>
              </div>
              <Switch
                checked={switching?.id === source.id ? switching.enabled : source.enabled}
                disabled={switching !== null}
                onChange={(event) => void toggle(source, event.target.checked)}
                slotProps={{ input: { 'aria-label': t('sso.provider.enableNamed', 'Allow sign-in with {{name}}', { name: source.display_name }) } }}
              />
              <Tooltip title={t('sso.provider.edit', 'Edit')}>
                <IconButton
                  size="small"
                  aria-label={t('sso.provider.editNamed', 'Edit {{name}}', { name: source.display_name })}
                  onClick={() => openDialog(source)}
                >
                  <Pencil size={16} />
                </IconButton>
              </Tooltip>
              <Tooltip title={t('sso.provider.delete', 'Delete')}>
                <IconButton
                  size="small"
                  aria-label={t('sso.provider.deleteNamed', 'Delete {{name}}', { name: source.display_name })}
                  onClick={() => openDelete(source)}
                >
                  <Trash2 size={16} />
                </IconButton>
              </Tooltip>
            </div>
          </ListCard>
        ))}
      </div>

      <OAuthSourceDialog
        key={`edit-${opening}`}
        open={!!dialog}
        editing={dialog?.editing ?? null}
        googleTaken={googleTaken}
        origin={origin}
        onClose={() => setDialog(null)}
        onSave={async (source) => {
          const editing = dialog?.editing
          const next = editing
            ? sources.map((item) => (item.id === editing.id ? source : item))
            : [...sources, source]
          const failure = await store(next)
          if (!failure) setDialog(null)
          return failure
        }}
      />
      <DeleteOAuthSourceDialog
        key={`delete-${opening}`}
        source={deleting}
        onClose={() => setDeleting(null)}
        onDelete={async (source) => {
          const failure = await store(sources.filter((item) => item.id !== source.id))
          if (!failure) setDeleting(null)
          return failure
        }}
      />
    </Section>
  )
}
