import { Alert, Button, Dialog, DialogActions, DialogContent, DialogTitle, MenuItem, TextField } from '@mui/material'
import { useCallback, useEffect, useState } from 'react'
import { callSettingsRpc } from '../../../api/settings'
import { fetchAgentList, type AgentEntry } from '../../../api/user_mgr'
import { useI18n } from '../../../i18n/provider'
import { CopyButton } from '../components/shared/CopyButton'
import { ListCard, SectionNote, StatusBadge } from '../components/shared/Section'

type Device = { executor_id: string; name: string; enabled: boolean; expires_at_ms: number | null }
type DeviceRow = { device: Device; online: { generation: number } | null }
type Issued = { device: Device; access_url: string; start_command: string }

export function AgentDevices({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t } = useI18n()
  const [devices, setDevices] = useState<DeviceRow[]>([])
  const [agents, setAgents] = useState<AgentEntry[]>([])
  const [name, setName] = useState('')
  const [agent, setAgent] = useState('')
  const [roots, setRoots] = useState('')
  const [profile, setProfile] = useState('default')
  const [hours, setHours] = useState(24)
  const [mode, setMode] = useState('read_write')
  const [issued, setIssued] = useState<Issued | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const refresh = useCallback(async () => {
    const result = await callSettingsRpc<{ devices: DeviceRow[] }>('executor.device.list')
    if (result.data) setDevices(result.data.devices)
    else if (result.error) setError(String(result.error))
  }, [])
  useEffect(() => {
    void refresh()
    const timer = window.setInterval(() => void refresh(), 5000)
    return () => window.clearInterval(timer)
  }, [refresh])
  useEffect(() => {
    if (open) void fetchAgentList().then(({ data }) => setAgents((data?.agents ?? []).filter((entry) => entry.settings.enabled && entry.install.state === 'ready')))
  }, [open])
  const mutate = async (method: string, params: Record<string, unknown>) => {
    setBusy(true)
    setError(null)
    try {
      const result = await callSettingsRpc<Issued>(method, params)
      if (!result.data) throw result.error ?? new Error('Request failed')
      if (result.data.access_url) setIssued(result.data)
      await refresh()
      return true
    } catch (cause) { setError(String(cause)); return false }
    finally { setBusy(false) }
  }
  const create = async () => {
    const ops = mode === 'read_only' ? ['open_stream', 'describe', 'read', 'query', 'cancel'] : ['open_stream', 'describe', 'exec', 'read', 'write', 'edit', 'query', 'cancel']
    if (await mutate('executor.device.create', {
      name,
      profiles: { [profile]: { workdir_roots: roots.split('\n').map((value) => value.trim()).filter(Boolean), env: {}, principals: [agent], ops } },
      expires_at_ms: hours ? Date.now() + hours * 3600_000 : null,
    })) { onClose(); setName('') }
  }
  return <div className="mt-3 space-y-2" data-testid="settings-agent-devices">
    {error ? <Alert severity="error" onClose={() => setError(null)}>{error}</Alert> : null}
    {devices.map(({ device, online }) => <ListCard key={device.executor_id}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <span className="mr-2 text-sm font-medium">{device.name}</span>
          <StatusBadge status={online ? 'pass' : 'neutral'} label={online ? t('settings.cluster.online', 'Online') : t('settings.cluster.offline', 'Offline')} />
          <p className="mt-1 text-xs" style={{ color: 'var(--cp-muted)' }}>{t('settings.executor.linuxCapabilities', 'Linux · Commands and files · UI unsupported')}{' · '}{!device.enabled ? t('settings.executor.revoked', 'Access revoked') : device.expires_at_ms ? new Date(device.expires_at_ms).toLocaleString() : t('settings.executor.noExpiry', 'No expiry')}</p>
        </div>
        <div className="flex gap-1">
          <Button size="small" disabled={busy} onClick={() => void mutate('executor.device.issue', { executor_id: device.executor_id, expires_at_ms: device.expires_at_ms === null || device.expires_at_ms > Date.now() ? device.expires_at_ms : Date.now() + 24 * 3600_000 })}>{t('settings.executor.resetUrl', 'Reset URL')}</Button>
          <Button size="small" disabled={busy || !device.enabled} onClick={() => void mutate('executor.device.revoke', { executor_id: device.executor_id })}>{t('settings.executor.revoke', 'Revoke')}</Button>
          <Button size="small" color="error" disabled={busy} onClick={() => void mutate('executor.device.delete', { executor_id: device.executor_id })}>{t('settings.common.delete', 'Delete')}</Button>
        </div>
      </div>
    </ListCard>)}
    <Dialog open={open} onClose={busy ? undefined : onClose} fullWidth maxWidth="sm">
      <DialogTitle>{t('settings.cluster.addDevice', 'Add device')}</DialogTitle>
      <DialogContent className="space-y-4">
        <SectionNote>{t('settings.executor.description', 'Run the Linux launcher with the generated URL. The device runs as the current OS user and is available only while the daemon is running. Expiry or revocation disconnects it.')}</SectionNote>
        <TextField autoFocus fullWidth label={t('settings.executor.deviceName', 'Device name')} value={name} onChange={(event) => setName(event.target.value)} />
        <TextField select fullWidth label={t('settings.executor.allowedAgent', 'Allowed Agent')} value={agent} onChange={(event) => setAgent(event.target.value)}>{agents.map((entry) => <MenuItem key={entry.agent_did} value={entry.agent_did}>{entry.display_name || entry.name}</MenuItem>)}</TextField>
        <TextField fullWidth label={t('settings.executor.profile', 'Profile')} value={profile} onChange={(event) => setProfile(event.target.value)} />
        <TextField fullWidth multiline minRows={2} label={t('settings.executor.roots', 'Allowed working directories (one absolute path per line)')} placeholder="/home/alice/work" value={roots} onChange={(event) => setRoots(event.target.value)} helperText={t('settings.executor.shellScope', 'These paths limit file operations and working directories. Shell commands retain the current OS user’s permissions.')} />
        <TextField select fullWidth label={t('settings.executor.permissions', 'Permissions')} value={mode} onChange={(event) => setMode(event.target.value)}><MenuItem value="read_write">{t('settings.executor.readWrite', 'Commands and file changes')}</MenuItem><MenuItem value="read_only">{t('settings.executor.readOnly', 'Read files only')}</MenuItem></TextField>
        <TextField select fullWidth label={t('settings.executor.validFor', 'URL valid for')} value={hours} onChange={(event) => setHours(Number(event.target.value))}><MenuItem value={1}>1 h</MenuItem><MenuItem value={24}>24 h</MenuItem><MenuItem value={168}>7 d</MenuItem><MenuItem value={0}>{t('settings.executor.noExpiry', 'No expiry')}</MenuItem></TextField>
      </DialogContent>
      <DialogActions><Button disabled={busy} onClick={onClose}>{t('settings.common.cancel', 'Cancel')}</Button><Button variant="contained" disabled={busy || !name.trim() || !agent || !roots.trim() || !/^[a-z0-9_-]{1,64}$/.test(profile)} onClick={() => void create()}>{t('settings.executor.generateUrl', 'Generate URL')}</Button></DialogActions>
    </Dialog>
    <Dialog open={issued !== null} onClose={() => setIssued(null)} fullWidth maxWidth="sm">
      <DialogTitle>{t('settings.executor.startDevice', 'Start your device')}</DialogTitle>
      <DialogContent className="space-y-3">
        <SectionNote>{t('settings.executor.keepUrlPrivate', 'Keep this URL private. Use the Linux launcher from the daemon release package. Closing the daemon takes this device offline. Resetting this URL disconnects existing access.')}</SectionNote>
        <TextField fullWidth multiline label={t('settings.executor.startCommand', 'Startup command')} value={issued?.start_command ?? ''} slotProps={{ input: { readOnly: true } }} />
        <CopyButton label={t('settings.executor.copyCommand', 'Copy command')} text={() => issued?.start_command ?? ''} />
        <CopyButton label={t('settings.executor.copyUrl', 'Copy URL')} text={() => issued?.access_url ?? ''} />
      </DialogContent>
      <DialogActions><Button onClick={() => setIssued(null)}>{t('settings.common.close', 'Close')}</Button></DialogActions>
    </Dialog>
  </div>
}
