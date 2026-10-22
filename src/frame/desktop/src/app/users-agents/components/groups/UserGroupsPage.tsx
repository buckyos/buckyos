/* ── System user groups and address book projection rules (admin only) ── */

import { useMemo, useState, type FormEvent, type ReactNode } from 'react'
import { Alert, Button, Chip, CircularProgress, IconButton, TextField } from '@mui/material'
import { ArrowRight, Pencil, Plus, RotateCcw, Trash2, UsersRound, X } from 'lucide-react'
import {
  ASSIGNABLE_USER_TYPES,
  USER_GROUP_ID_PATTERN,
  systemGroupRef,
  userTypeGroupRef,
  type ContactProjectionRule,
} from '../../../../api/user_type'
import { errorMessage } from '../../../agent-setup/model'
import { useI18n } from '../../../../i18n/provider'
import { useLocalUsers, useSelf, useUserGroups, useUsersAgentsStore } from '../../hooks/use-users-agents-store'
import type { SystemUserGroup } from '../../datamodel/types'
import { accountName, accountOptions, groupRefLabel, type AccountOption } from '../shared/userGroupLabels'

const panelStyle = {
  background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
  border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
}

const itemStyle = {
  background: 'color-mix(in srgb, var(--cp-surface) 80%, transparent)',
  border: '1px solid color-mix(in srgb, var(--cp-border) 40%, transparent)',
}

function Panel({ title, badge, children, testId }: { title: string; badge?: ReactNode; children: ReactNode; testId: string }) {
  return (
    <section className="rounded-[22px] px-5 py-4" style={panelStyle} data-testid={testId}>
      <div className="mb-3 flex items-center justify-between gap-2">
        <h3 className="font-display text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
          {title}
        </h3>
        {badge}
      </div>
      {children}
    </section>
  )
}

/** Runs a store change and turns its failure into a message. */
function useChange() {
  const { t } = useI18n()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const run = async (change: () => Promise<unknown>): Promise<boolean> => {
    setBusy(true)
    setError(null)
    const failure = await change()
    setBusy(false)
    if (failure) setError(t('usersAgents.error.change', undefined, { reason: errorMessage(failure) }))
    return !failure
  }
  return { busy, error, run }
}

function CreateGroupForm({ groups }: { groups: readonly SystemUserGroup[] }) {
  const { t } = useI18n()
  const store = useUsersAgentsStore()
  const { busy, error, run } = useChange()
  const [groupId, setGroupId] = useState('')
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const id = groupId.trim()
  const idInvalid = id.length > 0 && !USER_GROUP_ID_PATTERN.test(id)
  const idTaken = groups.some((group) => group.id === id)

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!id || idInvalid || idTaken || !name.trim()) return
    if (await run(() => store.createUserGroup(id, name.trim(), description.trim() || undefined))) {
      setGroupId('')
      setName('')
      setDescription('')
    }
  }

  return (
    <form className="space-y-2 rounded-[16px] px-4 py-3" style={itemStyle} onSubmit={(event) => void submit(event)} data-testid="user-group-create">
      <div className="grid gap-2 sm:grid-cols-2">
        <TextField
          size="small"
          label={t('usersAgents.userGroups.groupId')}
          value={groupId}
          onChange={(event) => setGroupId(event.target.value)}
          error={idInvalid || idTaken}
          helperText={idTaken
            ? t('usersAgents.userGroups.groupIdTaken')
            : idInvalid ? t('usersAgents.userGroups.groupIdInvalid') : t('usersAgents.userGroups.groupIdHint')}
          inputProps={{ autoComplete: 'off', spellCheck: false }}
        />
        <TextField
          size="small"
          label={t('usersAgents.userGroups.name')}
          value={name}
          onChange={(event) => setName(event.target.value)}
          inputProps={{ maxLength: 64 }}
        />
      </div>
      <TextField
        size="small"
        fullWidth
        label={t('usersAgents.userGroups.description')}
        value={description}
        onChange={(event) => setDescription(event.target.value)}
      />
      {error ? <Alert severity="error">{error}</Alert> : null}
      <div className="flex justify-end">
        <Button
          type="submit"
          size="small"
          variant="contained"
          disabled={busy || !id || idInvalid || idTaken || !name.trim()}
          startIcon={busy ? <CircularProgress color="inherit" size={14} /> : <Plus size={14} />}
        >
          {t('usersAgents.userGroups.create')}
        </Button>
      </div>
    </form>
  )
}

function GroupCard({ group, accounts }: { group: SystemUserGroup; accounts: readonly AccountOption[] }) {
  const { t } = useI18n()
  const store = useUsersAgentsStore()
  const { busy, error, run } = useChange()
  const candidates = accounts.filter((account) => !group.members.includes(account.id))

  const rename = () => {
    const next = window.prompt(t('usersAgents.userGroups.renamePrompt'), group.name)?.trim()
    if (next && next !== group.name) void run(() => store.updateUserGroup(group.id, { name: next }))
  }

  const remove = () => {
    if (window.confirm(t('usersAgents.userGroups.deleteConfirm', undefined, { name: group.name }))) {
      void run(() => store.deleteUserGroup(group.id))
    }
  }

  return (
    <div className="rounded-[16px] px-4 py-3" style={itemStyle} data-testid="user-group" data-group-id={group.id}>
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{group.name}</div>
          <div className="truncate font-mono text-[11px]" style={{ color: 'var(--cp-muted)' }}>{systemGroupRef(group.id)}</div>
          {group.description ? (
            <div className="mt-1 text-[12px]" style={{ color: 'var(--cp-muted)' }}>{group.description}</div>
          ) : null}
        </div>
        {busy ? <CircularProgress size={14} /> : null}
        <IconButton
          size="small"
          title={t('usersAgents.userGroups.rename', undefined, { name: group.name })}
          aria-label={t('usersAgents.userGroups.rename', undefined, { name: group.name })}
          onClick={rename}
          disabled={busy}
        >
          <Pencil size={13} />
        </IconButton>
        <IconButton
          size="small"
          title={t('usersAgents.userGroups.delete', undefined, { name: group.name })}
          aria-label={t('usersAgents.userGroups.delete', undefined, { name: group.name })}
          onClick={remove}
          disabled={busy}
        >
          <Trash2 size={13} />
        </IconButton>
      </div>

      <div className="mt-3 text-[12px] font-medium" style={{ color: 'var(--cp-muted)' }}>
        {t('usersAgents.userGroups.members', undefined, { count: group.members.length })}
      </div>
      <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
        {group.members.length === 0 ? (
          <span className="text-[12px]" style={{ color: 'var(--cp-muted)' }}>{t('usersAgents.userGroups.noMembers')}</span>
        ) : null}
        {group.members.map((member) => {
          const name = accountName(member, accounts)
          return (
            <Chip
              key={member}
              size="small"
              variant="outlined"
              label={name}
              disabled={busy}
              onDelete={() => void run(() => store.setUserGroupMembers(group.id, { remove: [member] }))}
              deleteIcon={<X size={12} role="button" aria-label={t('usersAgents.userGroups.removeMember', undefined, { member: name })} />}
            />
          )
        })}
        {candidates.length > 0 ? (
          <TextField
            select
            size="small"
            value=""
            disabled={busy}
            onChange={(event) => {
              const userId = event.target.value
              if (userId) void run(() => store.setUserGroupMembers(group.id, { add: [userId] }))
            }}
            SelectProps={{ native: true }}
            inputProps={{ 'aria-label': t('usersAgents.userGroups.addMember', undefined, { name: group.name }) }}
            sx={{ width: 200 }}
          >
            <option value="">{t('usersAgents.userGroups.addMemberPlaceholder')}</option>
            {candidates.map((account) => (
              <option key={account.id} value={account.id}>{account.name}</option>
            ))}
          </TextField>
        ) : null}
      </div>
      {error ? <Alert className="mt-2" severity="error">{error}</Alert> : null}
    </div>
  )
}

function ProjectionRules({ groups, rules, isDefault }: { groups: readonly SystemUserGroup[]; rules: readonly ContactProjectionRule[]; isDefault: boolean }) {
  const { t } = useI18n()
  const store = useUsersAgentsStore()
  const { busy, error, run } = useChange()
  const refs = useMemo(
    () => [
      ...ASSIGNABLE_USER_TYPES.map(userTypeGroupRef),
      ...groups.map((group) => systemGroupRef(group.id)),
    ],
    [groups],
  )
  const [source, setSource] = useState('')
  const [target, setTarget] = useState('')
  const duplicate = rules.some((rule) => rule.source === source && rule.target === target)
  const label = (ref: string) => groupRefLabel(ref, groups, t)

  const add = async () => {
    if (!source || !target || duplicate) return
    if (await run(() => store.setProjectionRules([...rules, { source, target }]))) {
      setSource('')
      setTarget('')
    }
  }

  const removeRule = (index: number) => {
    void run(() => store.setProjectionRules(rules.filter((_, at) => at !== index)))
  }

  const reset = () => {
    if (window.confirm(t('usersAgents.projection.resetConfirm'))) void run(() => store.resetProjectionRules())
  }

  const select = (value: string, onChange: (value: string) => void, ariaLabel: string) => (
    <TextField
      select
      size="small"
      value={value}
      label={ariaLabel}
      onChange={(event) => onChange(event.target.value)}
      SelectProps={{ native: true }}
      InputLabelProps={{ shrink: true }}
      sx={{ minWidth: 170, flex: 1 }}
      disabled={busy}
    >
      <option value="" />
      {refs.map((ref) => (
        <option key={ref} value={ref}>{label(ref)}</option>
      ))}
    </TextField>
  )

  return (
    <Panel
      title={t('usersAgents.projection.title')}
      testId="contact-projections"
      badge={(
        <Chip
          size="small"
          variant="outlined"
          label={isDefault ? t('usersAgents.projection.default') : t('usersAgents.projection.custom')}
          data-testid="projection-mode"
        />
      )}
    >
      <p className="mb-3 text-[12px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('usersAgents.projection.intro')}</p>
      <div className="space-y-1.5">
        {rules.length === 0 ? (
          <div className="text-sm" style={{ color: 'var(--cp-muted)' }}>{t('usersAgents.projection.none')}</div>
        ) : null}
        {rules.map((rule, index) => (
          <div
            key={`${rule.source}->${rule.target}`}
            className="flex items-center gap-2 rounded-[14px] px-3 py-2"
            style={itemStyle}
            data-testid="projection-rule"
          >
            <Chip size="small" label={label(rule.source)} />
            <ArrowRight size={14} style={{ color: 'var(--cp-muted)' }} aria-hidden />
            <Chip size="small" variant="outlined" label={label(rule.target)} />
            <span className="min-w-0 flex-1 truncate text-[12px]" style={{ color: 'var(--cp-muted)' }}>
              {t('usersAgents.projection.rule', undefined, { source: label(rule.source), target: label(rule.target) })}
            </span>
            <IconButton
              size="small"
              aria-label={t('usersAgents.projection.remove', undefined, { source: label(rule.source), target: label(rule.target) })}
              onClick={() => removeRule(index)}
              disabled={busy}
            >
              <Trash2 size={13} />
            </IconButton>
          </div>
        ))}
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        {select(source, setSource, t('usersAgents.projection.source'))}
        <ArrowRight size={14} style={{ color: 'var(--cp-muted)' }} aria-hidden />
        {select(target, setTarget, t('usersAgents.projection.target'))}
        <Button size="small" variant="contained" onClick={() => void add()} disabled={busy || !source || !target || duplicate} startIcon={<Plus size={14} />}>
          {t('usersAgents.projection.add')}
        </Button>
      </div>
      {duplicate && source && target ? <div className="mt-1 text-[12px]" style={{ color: 'var(--cp-warning)' }}>{t('usersAgents.projection.duplicate')}</div> : null}
      {error ? <Alert className="mt-2" severity="error">{error}</Alert> : null}
      <div className="mt-3 flex justify-end">
        <Button size="small" variant="outlined" onClick={reset} disabled={busy || isDefault} startIcon={<RotateCcw size={14} />}>
          {t('usersAgents.projection.reset')}
        </Button>
      </div>
    </Panel>
  )
}

export function UserGroupsPage() {
  const { t } = useI18n()
  const userGroups = useUserGroups()
  const self = useSelf()
  const localUsers = useLocalUsers()
  const accounts = useMemo(() => accountOptions(self, localUsers), [self, localUsers])

  return (
    <div className="space-y-4" data-testid="user-groups-page">
      <div className="flex items-center gap-3">
        <div
          className="flex h-10 w-10 items-center justify-center rounded-[14px]"
          style={{ background: 'color-mix(in srgb, var(--cp-accent) 14%, var(--cp-surface))', color: 'var(--cp-accent)' }}
        >
          <UsersRound size={20} />
        </div>
        <div className="min-w-0">
          <h2 className="font-display text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>{t('usersAgents.userGroups.title')}</h2>
          <p className="text-[12px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('usersAgents.userGroups.intro')}</p>
        </div>
      </div>

      {!userGroups ? (
        <Alert severity="warning">{t('usersAgents.userGroups.unavailable')}</Alert>
      ) : (
        <>
          <Panel title={t('usersAgents.userGroups.listTitle', undefined, { count: userGroups.groups.length })} testId="user-group-list">
            <div className="space-y-2">
              {userGroups.groups.length === 0 ? (
                <div className="text-sm" style={{ color: 'var(--cp-muted)' }}>{t('usersAgents.userGroups.empty')}</div>
              ) : null}
              {userGroups.groups.map((group) => (
                <GroupCard key={group.id} group={group} accounts={accounts} />
              ))}
              <CreateGroupForm groups={userGroups.groups} />
            </div>
          </Panel>
          <ProjectionRules
            groups={userGroups.groups}
            rules={userGroups.projectionRules}
            isDefault={userGroups.projectionsAreDefault}
          />
        </>
      )}
    </div>
  )
}
