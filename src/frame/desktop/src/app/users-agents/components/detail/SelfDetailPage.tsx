/* ── Self (current user) detail page ── */

import { useState } from 'react'
import { Alert, Button, Chip, CircularProgress, Dialog, DialogActions, DialogContent, DialogTitle } from '@mui/material'
import { useSelf, useUsersAgentsStore } from '../../hooks/use-users-agents-store'
import { HeaderSection } from '../sections/HeaderSection'
import { SocialAccountsSection } from '../sections/SocialAccountsSection'
import { InfoFieldsSection } from '../sections/InfoFieldsSection'
import { DIDDocumentSection } from '../sections/DIDDocumentSection'
import { SecuritySection } from '../sections/SecuritySection'
import { SELF_EDITABLE_INFO_KEYS } from '../../datamodel/transforms'
import { errorMessage } from '../../../agent-setup/model'
import { ImagePicker } from '../../../../presentation/ImagePicker'
import { AVATAR_IMAGE } from '../../../../presentation/imageFile'
import { useI18n } from '../../../../i18n/provider'

type SaveState =
  | { kind: 'idle' }
  | { kind: 'saving' }
  | { kind: 'saved' }
  | { kind: 'failed'; detail: string; retry: () => void }

function AvatarDialog({
  name,
  current,
  shown,
  onClose,
  onSave,
}: {
  name: string
  /** The picture saved on this Zone (a data URL), or null. */
  current: string | null
  /** What is shown now, e.g. the DID document's picture when none is saved here. */
  shown?: string
  onClose: () => void
  onSave: (avatar: string | null) => Promise<unknown>
}) {
  const { t } = useI18n()
  const [draft, setDraft] = useState<string | null>(current)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const save = async () => {
    if (draft === current) {
      onClose()
      return
    }
    setSaving(true)
    setError(null)
    const failure = await onSave(draft)
    setSaving(false)
    if (failure) setError(t('usersAgents.self.avatarSaveFailed', 'The avatar was not saved: {{detail}}', { detail: errorMessage(failure) }))
    else onClose()
  }

  return (
    <Dialog open onClose={saving ? undefined : onClose} fullWidth maxWidth="xs" aria-labelledby="self-avatar-dialog-title">
      <DialogTitle id="self-avatar-dialog-title">{t('usersAgents.self.avatarTitle', 'Profile picture')}</DialogTitle>
      <DialogContent>
        <div className="space-y-3 pt-1">
          <ImagePicker
            title={name}
            value={draft}
            fallbackIcons={!draft && !current && shown ? [shown] : []}
            options={AVATAR_IMAGE}
            disabled={saving}
            previewSize={88}
            labels={{
              choose: t('usersAgents.self.avatarChoose', 'Choose image'),
              remove: t('usersAgents.self.avatarRemove', 'Remove'),
              usingDefault: t('usersAgents.self.avatarDefault', 'No picture of your own: your initial, or the one in your DID document, is shown.'),
              usingCustom: t('usersAgents.self.avatarCustom', 'Cropped to a square; everyone who can see your profile sees it.'),
            }}
            onChange={setDraft}
            testId="self-avatar-picker"
            inputTestId="self-avatar-input"
          />
          {error ? <Alert severity="error" data-testid="self-avatar-error">{error}</Alert> : null}
        </div>
      </DialogContent>
      <DialogActions>
        <Button variant="text" disabled={saving} onClick={onClose}>{t('common.cancel')}</Button>
        <Button
          disabled={saving}
          startIcon={saving ? <CircularProgress size={13} color="inherit" /> : undefined}
          onClick={() => void save()}
          data-testid="self-avatar-save"
        >
          {t('common.save')}
        </Button>
      </DialogActions>
    </Dialog>
  )
}

export function SelfDetailPage() {
  const self = useSelf()
  const store = useUsersAgentsStore()
  const { t } = useI18n()
  const [avatarOpen, setAvatarOpen] = useState(false)
  const [saveState, setSaveState] = useState<SaveState>({ kind: 'idle' })

  const runSave = async (save: () => Promise<unknown>) => {
    setSaveState({ kind: 'saving' })
    const failure = await save()
    setSaveState(failure
      ? { kind: 'failed', detail: errorMessage(failure), retry: () => void runSave(save) }
      : { kind: 'saved' })
  }

  const handleBioEdit = () => {
    const nextBio = window.prompt(t('usersAgents.self.bioPrompt'), self.bio ?? '')
    if (nextBio !== null) void runSave(() => store.updateSelfBio(nextBio))
  }

  return (
    <div className="space-y-4">
      <HeaderSection
        name={self.displayName}
        kind="self"
        avatarUrl={self.avatarUrl}
        did={self.did}
        subtitle={self.bio}
        isOnline
        previewUrl={`/profile/${encodeURIComponent(self.did ?? self.id)}`}
        onAvatarEdit={() => setAvatarOpen(true)}
        onSubtitleEdit={handleBioEdit}
        badges={
          <>
            <Chip label={t(`usersAgents.role.${self.role}`)} size="small" color="primary" variant="outlined" />
            {self.twoFactorEnabled && (
              <Chip label="2FA" size="small" color="success" variant="outlined" />
            )}
          </>
        }
      />

      {saveState.kind !== 'idle' ? (
        <Alert
          severity={saveState.kind === 'failed' ? 'error' : saveState.kind === 'saved' ? 'success' : 'info'}
          data-testid="self-profile-save-status"
          onClose={saveState.kind === 'saving' ? undefined : () => setSaveState({ kind: 'idle' })}
          action={saveState.kind === 'failed' ? (
            <Button size="small" color="inherit" onClick={saveState.retry}>{t('usersAgents.self.retry', 'Retry')}</Button>
          ) : undefined}
        >
          {saveState.kind === 'saving'
            ? t('usersAgents.self.saving', 'Saving your profile…')
            : saveState.kind === 'saved'
              ? t('usersAgents.self.saved', 'Profile saved.')
              : t('usersAgents.self.saveFailed', 'Your profile was not saved: {{detail}}', { detail: saveState.detail })}
        </Alert>
      ) : null}

      <InfoFieldsSection
        title={t('usersAgents.section.profile')}
        fields={self.info}
        editableKeys={SELF_EDITABLE_INFO_KEYS}
        onFieldChange={(key, value) => void runSave(() => store.updateSelfInfo(key, value))}
      />

      <SocialAccountsSection entityId={self.id} accounts={self.socialAccounts} ownProfile />

      <InfoFieldsSection title={t('usersAgents.section.settings')} fields={self.settings} />

      <SecuritySection
        twoFactorEnabled={self.twoFactorEnabled}
        lastLogin={self.lastLogin}
      />

      <DIDDocumentSection document={self.didDocument} />

      {avatarOpen ? (
        <AvatarDialog
          name={self.displayName}
          current={self.avatarUrl?.startsWith('data:image/') ? self.avatarUrl : null}
          shown={self.avatarUrl}
          onClose={() => setAvatarOpen(false)}
          onSave={(avatar) => store.updateSelfAvatar(avatar)}
        />
      ) : null}
    </div>
  )
}
