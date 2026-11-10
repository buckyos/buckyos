import { Alert, Button, Dialog, DialogActions, DialogContent, DialogTitle, FormControlLabel, MenuItem, Switch, TextField } from '@mui/material'
import { useEffect, useState } from 'react'
import {
  createExecutorDevice,
  removeExecutorDevice,
  setNodeOwnerAgents,
  updateAgent,
  type AgentEntry,
  type AgentRuntimeAssignment,
  type AgentRuntimeCatalog,
  type AgentRuntimeCatalogEntry,
  type ComputerAccess,
  type ExecutorDeviceIssued,
  type RuntimeBaseline,
} from '../../../api/user_mgr'
import { isMockRuntime } from '../../../runtime'
import { useI18n } from '../../../i18n/provider'
import { errorMessage } from '../../agent-setup/model'
import { agentServiceClient } from '../../messagehub/conversation/worklog/source'
import { CopyButton } from '../components/shared/CopyButton'
import { SectionNote } from '../components/shared/Section'

type Translate = ReturnType<typeof useI18n>['t']

function splitRoots(value: string) {
  return value.split('\n').map((line) => line.trim()).filter(Boolean)
}

function ComputerSelect({ value, onChange, readOnly, t }: { value: ComputerAccess; onChange: (value: ComputerAccess) => void; readOnly?: boolean; t: Translate }) {
  return (
    <TextField
      select
      fullWidth
      label={t('settings.runtimes.computerLabel', 'ComputerUse (desktop access)')}
      value={value}
      onChange={(event) => onChange(event.target.value as ComputerAccess)}
      helperText={t('settings.runtimes.computerHint', 'On macOS, screen recording and Accessibility permissions must also be granted on the machine. The owner can pause desktop input locally.')}
    >
      <MenuItem value="none">{t('settings.runtimes.computer.none', 'Disabled')}</MenuItem>
      <MenuItem value="observe">{t('settings.runtimes.computer.observe', 'Screenshots only')}</MenuItem>
      <MenuItem value="control" disabled={readOnly}>{t('settings.runtimes.computer.control', 'Screenshots and input')}</MenuItem>
    </TextField>
  )
}

function BaselineSelect({ value, onChange, t }: { value: RuntimeBaseline; onChange: (value: RuntimeBaseline) => void; t: Translate }) {
  return (
    <TextField
      select
      fullWidth
      label={t('settings.runtimes.baselineLabel', 'Declared environment')}
      value={value}
      onChange={(event) => onChange(event.target.value as RuntimeBaseline)}
      helperText={t('settings.runtimes.baselineHint', 'What you declare this runtime provides. It decides which skills its sessions take; “Extended” means the extension tools are installed there.')}
    >
      <MenuItem value="standard">{t('settings.runtimes.baseline.standard', 'Standard environment')}</MenuItem>
      <MenuItem value="extended">{t('settings.runtimes.baseline.extended', 'Extended environment')}</MenuItem>
    </TextField>
  )
}

export function AddDeviceDialog({ open, agents, onClose, onIssued }: {
  open: boolean
  agents: AgentEntry[]
  onClose: () => void
  /** The device exists; `assignError` when assigning it to the chosen Agent failed. */
  onIssued: (issued: ExecutorDeviceIssued, assignError: string | null) => void
}) {
  const { t } = useI18n()
  const [name, setName] = useState('')
  const [agent, setAgent] = useState('')
  const [roots, setRoots] = useState('')
  const [hours, setHours] = useState(24)
  const [mode, setMode] = useState<'read_write' | 'read_only'>('read_write')
  const [computer, setComputer] = useState<ComputerAccess>('none')
  const [baseline, setBaseline] = useState<RuntimeBaseline>('standard')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const create = async () => {
    const ops = mode === 'read_only' ? ['open_stream', 'describe', 'read', 'query', 'cancel'] : ['open_stream', 'describe', 'exec', 'read', 'write', 'edit', 'query', 'cancel']
    if (computer !== 'none') ops.push('computer_inspect', 'computer_observe')
    if (computer === 'control' && mode === 'read_write') ops.push('computer_act')
    setBusy(true)
    setError(null)
    const created = await createExecutorDevice({
      name: name.trim(),
      workdirRoots: splitRoots(roots),
      ops,
      baseline,
      expiresAtMs: hours ? Date.now() + hours * 3600_000 : null,
    })
    if (!created.data) {
      setBusy(false)
      setError(errorMessage(created.error))
      return
    }
    const assigned = agent
      ? await updateAgent({ agentId: agent, runtimes: { [`exec-${created.data.device.executor_id}`]: { enabled: true } } })
      : null
    setBusy(false)
    setName('')
    setAgent('')
    onIssued(
      created.data,
      assigned && !assigned.data
        ? t('settings.runtimes.assignAfterCreateFailed', 'The device was added but could not be assigned to the Agent; it is unassigned. {{detail}}', { detail: errorMessage(assigned.error) })
        : null,
    )
  }

  return (
    <Dialog open={open} onClose={busy ? undefined : onClose} fullWidth maxWidth="sm">
      <DialogTitle>{t('settings.cluster.addDevice', 'Add device')}</DialogTitle>
      <DialogContent className="space-y-4">
        <SectionNote>{t('settings.runtimes.addDescription', 'Run the Linux or macOS daemon with the generated URL. The device runs as the current OS user and is available only while the daemon is running. What it allows is fixed here; expiry or revocation disconnects it.')}</SectionNote>
        {error ? <Alert severity="error" onClose={() => setError(null)}>{error}</Alert> : null}
        <TextField autoFocus fullWidth label={t('settings.runtimes.deviceName', 'Device name')} value={name} onChange={(event) => setName(event.target.value)} />
        <TextField fullWidth multiline minRows={2} label={t('settings.runtimes.roots', 'Allowed working directories (one absolute path per line)')} placeholder="/home/alice/work" value={roots} onChange={(event) => setRoots(event.target.value)} helperText={t('settings.runtimes.shellScope', 'These paths limit file operations and working directories. Shell commands retain the current OS user’s permissions.')} />
        <TextField select fullWidth label={t('settings.runtimes.permissions', 'Permissions')} value={mode} onChange={(event) => { setMode(event.target.value as typeof mode); if (event.target.value === 'read_only' && computer === 'control') setComputer('observe') }}>
          <MenuItem value="read_write">{t('settings.runtimes.modeReadWrite', 'Commands and file changes')}</MenuItem>
          <MenuItem value="read_only">{t('settings.runtimes.modeReadOnly', 'Read files only')}</MenuItem>
        </TextField>
        <ComputerSelect value={computer} onChange={setComputer} readOnly={mode === 'read_only'} t={t} />
        <BaselineSelect value={baseline} onChange={setBaseline} t={t} />
        <TextField select fullWidth label={t('settings.runtimes.validFor', 'URL valid for')} value={hours} onChange={(event) => setHours(Number(event.target.value))}>
          <MenuItem value={1}>1 h</MenuItem>
          <MenuItem value={24}>24 h</MenuItem>
          <MenuItem value={168}>7 d</MenuItem>
          <MenuItem value={0}>{t('settings.runtimes.noExpiry', 'No expiry')}</MenuItem>
        </TextField>
        <TextField select fullWidth label={t('settings.runtimes.assignOptional', 'Assign to Agent (optional)')} value={agent} onChange={(event) => setAgent(event.target.value)} helperText={t('settings.runtimes.assignOptionalHint', 'Without an Agent the device is only issued; assign it later from this page.')}>
          <MenuItem value="">{t('settings.runtimes.assignNone', 'Do not assign now')}</MenuItem>
          {agents.map((entry) => <MenuItem key={entry.agent_id} value={entry.agent_id}>{entry.display_name || entry.name}</MenuItem>)}
        </TextField>
      </DialogContent>
      <DialogActions>
        <Button disabled={busy} onClick={onClose}>{t('settings.common.cancel', 'Cancel')}</Button>
        <Button variant="contained" disabled={busy || !name.trim() || splitRoots(roots).length === 0} onClick={() => void create()}>{t('settings.runtimes.generateUrl', 'Generate URL')}</Button>
      </DialogActions>
    </Dialog>
  )
}

export function IssuedDeviceDialog({ issued, onClose }: { issued: ExecutorDeviceIssued | null; onClose: () => void }) {
  const { t } = useI18n()
  return (
    <Dialog open={issued !== null} onClose={onClose} fullWidth maxWidth="sm">
      <DialogTitle>{t('settings.runtimes.startDevice', 'Start your device')}</DialogTitle>
      <DialogContent className="space-y-3">
        <SectionNote>{t('settings.runtimes.keepUrlPrivate', 'Keep this URL private. Use the daemon package for your Linux or macOS device. Closing the daemon takes this device offline. Resetting this URL disconnects existing access.')}</SectionNote>
        <TextField fullWidth multiline label={t('settings.runtimes.startCommand', 'Startup command')} value={issued?.start_command ?? ''} slotProps={{ input: { readOnly: true } }} />
        <CopyButton label={t('settings.runtimes.copyCommand', 'Copy command')} text={() => issued?.start_command ?? ''} />
        <CopyButton label={t('settings.runtimes.copyUrl', 'Copy URL')} text={() => issued?.access_url ?? ''} />
      </DialogContent>
      <DialogActions><Button onClick={onClose}>{t('settings.runtimes.close', 'Close')}</Button></DialogActions>
    </Dialog>
  )
}

export function EditNodeDialog({ runtime, onClose }: { runtime: AgentRuntimeCatalogEntry; onClose: (changed?: boolean) => void }) {
  const { t } = useI18n()
  const [enabled, setEnabled] = useState(!runtime.closed)
  const [roots, setRoots] = useState((runtime.workdir_roots ?? []).join('\n'))
  const [computer, setComputer] = useState<ComputerAccess>(runtime.computer)
  const [baseline, setBaseline] = useState<RuntimeBaseline>(runtime.baseline)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const save = async () => {
    setBusy(true)
    setError(null)
    const { data, error: failed } = await setNodeOwnerAgents({ nodeId: runtime.node_id ?? '', enabled, workdirRoots: splitRoots(roots), computer, baseline })
    setBusy(false)
    if (data) return onClose(true)
    const message = errorMessage(failed)
    setError(/no_policy:/.test(message)
      ? t('settings.runtimes.noPolicy', 'This node is not open to Agents: no account for Agents was chosen when the node was activated.')
      : message)
  }

  return (
    <Dialog open onClose={busy ? undefined : () => onClose()} fullWidth maxWidth="sm" data-testid="settings-runtime-edit">
      <DialogTitle>{t('settings.runtimes.editTitle', 'What “{{name}}” allows Agents', { name: runtime.name || runtime.node_id || runtime.runtime_id })}</DialogTitle>
      <DialogContent className="space-y-4">
        {error ? <Alert severity="error" onClose={() => setError(null)}>{error}</Alert> : null}
        <FormControlLabel
          control={<Switch checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />}
          label={enabled ? t('settings.runtimes.openToAgents', 'Open to my Agents') : t('settings.runtimes.closedToAgents', 'Closed to Agents')}
        />
        {runtime.run_as ? <SectionNote>{t('settings.runtimes.runAsFixed', 'Agents run here as the OS account chosen when the node was activated. It cannot be changed here.')}</SectionNote> : null}
        <TextField fullWidth multiline minRows={2} label={t('settings.runtimes.roots', 'Allowed working directories (one absolute path per line)')} value={roots} onChange={(event) => setRoots(event.target.value)} />
        <ComputerSelect value={computer} onChange={setComputer} t={t} />
        <BaselineSelect value={baseline} onChange={setBaseline} t={t} />
      </DialogContent>
      <DialogActions>
        <Button disabled={busy} onClick={() => onClose()}>{t('settings.common.cancel', 'Cancel')}</Button>
        <Button variant="contained" disabled={busy} onClick={() => void save()}>{t('settings.runtimes.save', 'Save')}</Button>
      </DialogActions>
    </Dialog>
  )
}

export function AssignRuntimeDialog({ runtime, agents, agentName, reload, onClose }: {
  runtime: AgentRuntimeCatalogEntry
  agents: AgentEntry[]
  agentName: (agentId: string) => string
  reload: () => Promise<AgentRuntimeCatalog | null>
  onClose: (changed?: boolean) => void
}) {
  const { t } = useI18n()
  const [agent, setAgent] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  /** The other Agent that has this device switched on, as the server refused the assignment. */
  const [inUse, setInUse] = useState<AgentRuntimeAssignment | null>(null)

  const assign = async (takeFrom?: AgentRuntimeAssignment) => {
    setBusy(true)
    setError(null)
    if (takeFrom) {
      const released = await updateAgent({ agentId: takeFrom.agent_id, runtimes: { [takeFrom.runtime_id]: { enabled: false } } })
      if (!released.data) {
        setBusy(false)
        setError(errorMessage(released.error))
        return
      }
    }
    const own = runtime.assigned_to.find((item) => item.agent_id === agent)?.runtime_id ?? runtime.runtime_id
    const result = await updateAgent({ agentId: agent, runtimes: { [own]: { enabled: true } } })
    if (result.data) {
      setBusy(false)
      return onClose(true)
    }
    const message = errorMessage(result.error)
    if (/runtime_in_use:/.test(message)) {
      const latest = (await reload())?.runtimes.find((item) => item.runtime_id === runtime.runtime_id)
      setInUse(latest?.assigned_to.find((item) => item.enabled && item.agent_id !== agent) ?? null)
    }
    setBusy(false)
    setError(message)
  }

  return (
    <Dialog open onClose={busy ? undefined : () => onClose(Boolean(inUse))} fullWidth maxWidth="xs" data-testid="settings-runtime-assign">
      <DialogTitle>{t('settings.runtimes.assignTitle', 'Assign “{{name}}” to an Agent', { name: runtime.name || runtime.runtime_id })}</DialogTitle>
      <DialogContent className="space-y-4">
        <SectionNote>{t('settings.runtimes.assignHint', 'Assigning only says which Agent uses this runtime. What the runtime allows stays as configured.')}</SectionNote>
        {inUse ? (
          <Alert severity="warning">{t('settings.runtimes.inUse', 'This device is in use by {{agent}}. A device is used by one Agent at a time.', { agent: agentName(inUse.agent_id) })}</Alert>
        ) : error ? (
          <Alert severity="error" onClose={() => setError(null)}>{error}</Alert>
        ) : null}
        {agents.length === 0 ? <SectionNote>{t('settings.runtimes.noAgents', 'You have no Agent that is ready.')}</SectionNote> : null}
        <TextField select fullWidth label={t('settings.runtimes.agent', 'Agent')} value={agent} onChange={(event) => { setAgent(event.target.value); setInUse(null); setError(null) }}>
          {agents.map((entry) => <MenuItem key={entry.agent_id} value={entry.agent_id}>{entry.display_name || entry.name}</MenuItem>)}
        </TextField>
      </DialogContent>
      <DialogActions>
        <Button disabled={busy} onClick={() => onClose(Boolean(inUse))}>{t('settings.common.cancel', 'Cancel')}</Button>
        {inUse ? (
          <Button variant="contained" color="warning" disabled={busy || !agent} onClick={() => void assign(inUse)} data-testid="settings-runtime-assign-switch">
            {t('settings.runtimes.switchAndAssign', 'Switch it off for {{agent}} and assign here', { agent: agentName(inUse.agent_id) })}
          </Button>
        ) : (
          <Button variant="contained" disabled={busy || !agent} onClick={() => void assign()} data-testid="settings-runtime-assign-ok">{t('settings.runtimes.assignAction', 'Assign')}</Button>
        )}
      </DialogActions>
    </Dialog>
  )
}

export function UnassignRuntimeDialog({ runtime, assignment, agentName, onClose }: {
  runtime: AgentRuntimeCatalogEntry
  assignment: AgentRuntimeAssignment
  agentName: (agentId: string) => string
  onClose: (changed?: boolean) => void
}) {
  const { t } = useI18n()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const unassign = async () => {
    setBusy(true)
    setError(null)
    const result = await updateAgent({ agentId: assignment.agent_id, runtimes: { [assignment.runtime_id]: null } })
    setBusy(false)
    if (result.data) onClose(true)
    else setError(errorMessage(result.error))
  }

  return (
    <Dialog open onClose={busy ? undefined : () => onClose()} fullWidth maxWidth="xs" data-testid="settings-runtime-unassign">
      <DialogTitle>{t('settings.runtimes.unassignTitle', 'Unassign “{{name}}” from {{agent}}?', { name: runtime.name || runtime.runtime_id, agent: agentName(assignment.agent_id) })}</DialogTitle>
      <DialogContent className="space-y-3">
        {error ? <Alert severity="error">{error}</Alert> : null}
        <p className="text-sm">{t('settings.runtimes.unassignHint', 'The Agent stops using this runtime. Its history of the runtime is kept by the Agent, and the runtime itself is not changed.')}</p>
      </DialogContent>
      <DialogActions>
        <Button disabled={busy} onClick={() => onClose()}>{t('settings.common.cancel', 'Cancel')}</Button>
        <Button variant="contained" color="error" disabled={busy} onClick={() => void unassign()} data-testid="settings-runtime-unassign-ok">{t('settings.runtimes.unassign', 'Unassign')}</Button>
      </DialogActions>
    </Dialog>
  )
}

/** `runtimes.impact` of an Agent's own service: what the Agent still has going on a runtime. */
interface RuntimeImpact {
  runtime_id: string
  active_sessions: Array<{ session_id: string; title?: string | null }>
  pending_todos: Array<{ session_id: string; todo_id: string; title?: string | null }>
}

export function RemoveDeviceDialog({ runtime, action, agents, agentName, onClose }: {
  runtime: AgentRuntimeCatalogEntry
  action: 'revoke' | 'delete'
  agents: AgentEntry[]
  agentName: (agentId: string) => string
  onClose: (changed?: boolean) => void
}) {
  const { t } = useI18n()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [impact, setImpact] = useState<RuntimeImpact | null>(null)
  const user = runtime.assigned_to.find((item) => item.enabled)
  const userDid = agents.find((entry) => entry.agent_id === user?.agent_id)?.agent_did
  const userRuntimeId = user?.runtime_id

  useEffect(() => {
    if (!userDid || !userRuntimeId || isMockRuntime()) return
    let live = true
    void agentServiceClient(userDid)
      .then((client) => client.call<RuntimeImpact, Record<string, unknown>>('runtimes.impact', { runtime_id: userRuntimeId }))
      .then((result) => { if (live && result) setImpact(result) })
      .catch(() => undefined)
    return () => { live = false }
  }, [userDid, userRuntimeId])

  const remove = async () => {
    if (!runtime.executor_id) return
    setBusy(true)
    setError(null)
    const result = await removeExecutorDevice(action, runtime.executor_id)
    setBusy(false)
    if (result.data) onClose(true)
    else setError(errorMessage(result.error))
  }

  const name = runtime.name || runtime.runtime_id
  const sessions = impact?.active_sessions ?? []
  const todos = impact?.pending_todos ?? []
  return (
    <Dialog open onClose={busy ? undefined : () => onClose()} fullWidth maxWidth="sm" data-testid="settings-runtime-remove">
      <DialogTitle>{action === 'delete' ? t('settings.runtimes.deleteTitle', 'Delete device “{{name}}”?', { name }) : t('settings.runtimes.revokeTitle', 'Revoke access of “{{name}}”?', { name })}</DialogTitle>
      <DialogContent className="space-y-3 text-sm">
        {error ? <Alert severity="error">{error}</Alert> : null}
        <p>{action === 'delete'
          ? t('settings.runtimes.deleteHint', 'The device is removed and its daemon is disconnected.')
          : t('settings.runtimes.revokeHint', 'The daemon is disconnected and its URL stops working. Reset the URL to use the device again.')}</p>
        {runtime.assigned_to.length > 0 ? (
          <>
            <p>{t('settings.runtimes.removeAssigned', 'It will be taken away from these Agents. Each Agent keeps its history of the device.')}</p>
            <ul className="list-disc pl-5">
              {runtime.assigned_to.map((item) => (
                <li key={`${item.agent_id}:${item.runtime_id}`}>
                  {agentName(item.agent_id)}
                  {item.enabled ? ` · ${t('settings.runtimes.inUseNow', 'using it now')}` : ''}
                  {item.default ? ` · ${t('settings.runtimes.defaultOf', 'its default runtime')}` : ''}
                </li>
              ))}
            </ul>
          </>
        ) : (
          <p>{t('settings.runtimes.removeUnassigned', 'No Agent is assigned to it.')}</p>
        )}
        {sessions.length > 0 ? (
          <>
            <p>{t('settings.runtimes.impactSessions', 'Sessions of {{agent}} still running on it:', { agent: agentName(user?.agent_id ?? '') })}</p>
            <ul className="list-disc pl-5">
              {sessions.map((session) => <li key={session.session_id} className="break-all">{session.title || session.session_id} <span style={{ color: 'var(--cp-muted)' }}>({session.session_id})</span></li>)}
            </ul>
          </>
        ) : null}
        {todos.length > 0 ? (
          <>
            <p>{t('settings.runtimes.impactTodos', 'Unfinished to-dos that need it:')}</p>
            <ul className="list-disc pl-5">
              {todos.map((todo) => <li key={`${todo.session_id}:${todo.todo_id}`} className="break-all">{todo.title || todo.todo_id}</li>)}
            </ul>
          </>
        ) : null}
      </DialogContent>
      <DialogActions>
        <Button disabled={busy} onClick={() => onClose()}>{t('settings.common.cancel', 'Cancel')}</Button>
        <Button variant="contained" color="error" disabled={busy} onClick={() => void remove()} data-testid="settings-runtime-remove-ok">
          {action === 'delete' ? t('settings.runtimes.delete', 'Delete') : t('settings.runtimes.revoke', 'Revoke')}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
