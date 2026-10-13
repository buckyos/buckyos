/* ── TaskCenter – new URL download ── */

import { useState, type FormEvent } from 'react'
import { Alert, Button, CircularProgress, Dialog, DialogActions, DialogContent, DialogTitle, TextField } from '@mui/material'
import { useI18n } from '../../../i18n/provider'
import { isDownloadableUrl } from '../../../api/downloads'
import { useTaskCenterStore } from '../hooks/use-task-center-store'

interface NewDownloadDialogProps {
  open: boolean
  onClose: () => void
  /** The task of the started download. */
  onCreated: (taskId: string) => void
}

function errorText(error: unknown): string {
  if (error instanceof Error && error.message) return error.message
  return String(error)
}

export function NewDownloadDialog({ open, onClose, onCreated }: NewDownloadDialogProps) {
  const store = useTaskCenterStore()
  const { t } = useI18n()
  const [url, setUrl] = useState('')
  const [filename, setFilename] = useState('')
  const [touched, setTouched] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const urlValid = isDownloadableUrl(url)
  const showUrlError = touched && url.trim() !== '' && !urlValid

  const close = () => {
    if (submitting) return
    setUrl('')
    setFilename('')
    setTouched(false)
    setError(null)
    onClose()
  }

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    setTouched(true)
    if (!urlValid || submitting) return
    setSubmitting(true)
    setError(null)
    try {
      const taskId = await store.createDownload({ url, filename })
      setSubmitting(false)
      setUrl('')
      setFilename('')
      setTouched(false)
      onCreated(taskId)
    } catch (reason) {
      setSubmitting(false)
      setError(t('taskCenter.download.failed', 'Could not start the download: {{detail}}', { detail: errorText(reason) }))
    }
  }

  return (
    <Dialog open={open} onClose={close} fullWidth maxWidth="xs" data-testid="task-download-dialog">
      <form onSubmit={submit} noValidate>
        <DialogTitle>{t('taskCenter.download.title', 'New Download')}</DialogTitle>
        <DialogContent>
          <div className="space-y-3 pt-1">
            <TextField
              autoFocus
              fullWidth
              size="small"
              type="url"
              label={t('taskCenter.download.url', 'Link')}
              placeholder="https://"
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              onBlur={() => setTouched(true)}
              error={showUrlError}
              helperText={showUrlError ? t('taskCenter.download.urlInvalid', 'Enter an http:// or https:// link.') : undefined}
              inputProps={{ 'data-testid': 'task-download-url' }}
            />
            <TextField
              fullWidth
              size="small"
              label={t('taskCenter.download.filename', 'Save as (optional)')}
              placeholder={t('taskCenter.download.filenamePlaceholder', 'Taken from the link')}
              value={filename}
              onChange={(event) => setFilename(event.target.value)}
              inputProps={{ 'data-testid': 'task-download-filename' }}
            />
            <p className="text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
              {t('taskCenter.download.hint', 'Saved to Downloads in your home folder. Progress shows in the system notifications.')}
            </p>
            {error ? <Alert severity="error">{error}</Alert> : null}
          </div>
        </DialogContent>
        <DialogActions>
          <Button onClick={close} disabled={submitting}>
            {t('common.cancel', 'Cancel')}
          </Button>
          <Button
            type="submit"
            variant="contained"
            disabled={!urlValid || submitting}
            startIcon={submitting ? <CircularProgress size={14} color="inherit" /> : undefined}
            data-testid="task-download-submit"
          >
            {t('taskCenter.download.start', 'Download')}
          </Button>
        </DialogActions>
      </form>
    </Dialog>
  )
}
