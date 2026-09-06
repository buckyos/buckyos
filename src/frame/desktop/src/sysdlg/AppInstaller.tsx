/* eslint-disable react-refresh/only-export-components */
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { useLocation, useNavigate } from 'react-router-dom'
import { useMediaQuery } from '@mui/material'
import { z } from 'zod'
import { AlertTriangle, Loader2, PackageCheck, X } from 'lucide-react'
import { useI18n } from '../i18n/provider'
import { AppIcon } from '../components/DesktopVisuals'
import { useSudoByPassword } from '../components/sudo'
import { WindowDialogProvider } from '../desktop/windows/dialogs'
import {
  AppServiceStoreProvider,
  useSharedAppServiceStore,
} from '../app/app-service/hooks/use-app-service-store'
import {
  createInstallInputSchema,
  taskIdSchema,
  type InstallInput,
} from '../app/app-service/schemas'
import { emptyRuntime, targets } from '../app/app-service/mock/fixtures'
import type {
  InspectionDraft,
  InstallTask,
  PlanReadiness,
} from '../app/app-service/types'
import { DetailPage } from '../app/app-service/pages/DetailPage'
import { RuntimeSummary } from '../app/app-service/components/RuntimeSummary'

const targetSchema = z
  .object({
    node_id: z.string().min(1).max(256).optional(),
    node_did: z.string().min(1).max(512).optional(),
  })
  .strict()
  .refine((v) => Boolean(v.node_id || v.node_did))
const launchOptionsSchema = z
  .object({
    target: targetSchema.optional(),
    install_params: z.record(z.string(), z.unknown()).optional(),
    offline: z.boolean().optional(),
  })
  .strict()
export type AppInstallerLaunchOptions = z.infer<typeof launchOptionsSchema>
export type AppInstallerLaunchParams =
  | { task_id: string }
  | { identifier: string; ref?: string; options?: AppInstallerLaunchOptions }
export type AppInstallerInternalParams =
  | AppInstallerLaunchParams
  | { draft_id: string }
export type AppInstallerLaunchErrorCode =
  | 'duplicate_parameter'
  | 'unknown_parameter'
  | 'conflicting_parameters'
  | 'invalid_task_id'
  | 'invalid_identifier'
  | 'invalid_options'
  | 'invalid_target'
export type AppInstallerLaunchQueryResult =
  | { ok: true; params: AppInstallerLaunchParams }
  | { ok: false; code: AppInstallerLaunchErrorCode }
export function parseAppInstallerLaunchQuery(
  search: string,
): AppInstallerLaunchQueryResult {
  const params = new URLSearchParams(search)
  for (const key of params.keys()) {
    if (!['task_id', 'identifier', 'ref', 'options'].includes(key))
      return { ok: false, code: 'unknown_parameter' }
    if (params.getAll(key).length !== 1)
      return { ok: false, code: 'duplicate_parameter' }
  }
  const task = params.get('task_id')
  if (task !== null) {
    if (params.size !== 1) return { ok: false, code: 'conflicting_parameters' }
    return taskIdSchema.safeParse(task).success
      ? { ok: true, params: { task_id: task } }
      : { ok: false, code: 'invalid_task_id' }
  }
  const identifier = params.get('identifier')?.trim()
  if (
    !identifier ||
    identifier.length > 32768 ||
    identifier.startsWith('{') ||
    identifier.startsWith('pikg-stage-') ||
    identifier.startsWith('/') ||
    (identifier.split('.').length === 3 && identifier.startsWith('eyJ'))
  )
    return { ok: false, code: 'invalid_identifier' }
  const ref = params.get('ref')
  if (ref !== null && (!ref.trim() || ref.length > 2048))
    return { ok: false, code: 'invalid_identifier' }
  let options: AppInstallerLaunchOptions | undefined
  if (params.has('options')) {
    const serialized = params.get('options')!
    if (serialized.length > 16384) return { ok: false, code: 'invalid_options' }
    try {
      const result = launchOptionsSchema.safeParse(JSON.parse(serialized))
      if (!result.success) return { ok: false, code: 'invalid_options' }
      options = result.data
    } catch {
      return { ok: false, code: 'invalid_options' }
    }
  }
  if (
    options?.target &&
    !targets.some(
      (n) =>
        (!options.target?.node_id || n.node_id === options.target.node_id) &&
        (!options.target?.node_did || n.node_did === options.target.node_did),
    )
  )
    return { ok: false, code: 'invalid_target' }
  return {
    ok: true,
    params: {
      identifier,
      ...(ref ? { ref } : {}),
      ...(options ? { options } : {}),
    },
  }
}
function Button({
  children,
  onClick,
  disabled,
  primary,
  testId,
  type = 'button',
}: {
  children: ReactNode
  onClick?: () => void
  disabled?: boolean
  primary?: boolean
  testId?: string
  type?: 'button' | 'submit'
}) {
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled}
      data-testid={testId}
      className={`min-h-11 rounded-xl border border-[var(--cp-border)] px-4 py-2 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-40 ${primary ? 'bg-[var(--cp-accent)] text-[var(--cp-surface)]' : 'bg-[var(--cp-surface)] text-[var(--cp-text)]'}`}
    >
      {children}
    </button>
  )
}
function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid gap-1 border-b border-[var(--cp-border)] py-3 text-sm sm:grid-cols-[160px_minmax(0,1fr)]">
      <dt className="text-[var(--cp-muted)]">{label}</dt>
      <dd className="min-w-0 break-all">{children}</dd>
    </div>
  )
}
function ErrorCard({ code }: { code: string }) {
  const { t } = useI18n()
  return (
    <div
      role="alert"
      className="flex items-start gap-2 rounded-xl border border-[var(--cp-danger)] p-4 text-sm"
    >
      <AlertTriangle className="shrink-0 text-[var(--cp-danger)]" size={18} />
      <div>
        <p>{t(`app22.error.${code}`, t('app22.error.UNKNOWN'))}</p>
        <code className="mt-1 block break-all text-xs text-[var(--cp-muted)]">
          {code}
        </code>
      </div>
    </div>
  )
}
function Readiness({ value }: { value: PlanReadiness }) {
  const { t } = useI18n()
  return (
    <section
      data-testid="app-installer-install-readiness"
      className="space-y-3"
    >
      <p className="text-sm font-semibold">
        {t(`app22.readiness.${value.install}`)}
      </p>
      <details
        data-testid="app-installer-trust-evidence"
        className="rounded-xl border border-[var(--cp-border)] p-4"
      >
        <summary className="min-h-11 cursor-pointer py-3 text-sm font-semibold">
          {t('app22.check.evidence')}
        </summary>
        <dl>
          {(
            [
              'document',
              'signature',
              'owner',
              'authority',
              'content',
              'target',
              'config',
            ] as const
          ).map((key) => (
            <Row key={key} label={t(`app22.check.${key}`)}>
              {t(`app22.evidence.${value[key]}`)}
            </Row>
          ))}
          <Row label={t('app22.check.publication')}>
            {t(`app22.document.${value.document_status}`)}
          </Row>
        </dl>
      </details>
      {value.local_developer_authority && (
        <p
          data-testid="app-installer-developer-authority"
          className="rounded-xl bg-[var(--cp-surface-2)] p-4 text-sm"
        >
          {t('app22.check.localAuthority')}
        </p>
      )}
      {value.issues.length > 0 && (
        <div data-testid="app-installer-blocking-reason" className="space-y-2">
          {value.issues.map((issue, i) => (
            <div key={`${issue.field}-${i}`}>
              <ErrorCard code={issue.code} />
              <p className="mt-1 break-all text-xs text-[var(--cp-muted)]">
                {issue.field} · {t(`app22.fix.${issue.action}`)}
              </p>
            </div>
          ))}
        </div>
      )}
    </section>
  )
}
function CheckStep({
  draft,
  onConfigure,
  onViewApp,
  onChangeSource,
}: {
  draft: InspectionDraft
  onConfigure: () => void
  onViewApp: (id: string) => void
  onChangeSource: () => void
}) {
  const store = useSharedAppServiceStore()
  const { t } = useI18n()
  const app = draft.app
  const local = ['local-pikg', 'personal-server-pikg'].includes(
    draft.source.kind,
  )
  const satisfied =
    draft.plan?.plan_use === 'SATISFIED' && draft.status === 'ready'
  return (
    <div className="space-y-5">
      <div className="flex items-start gap-3">
        <AppIcon iconKey={app.iconKey} className="!size-12 shrink-0" />
        <div className="min-w-0">
          <h2 className="text-xl font-semibold">{app.show_name}</h2>
          <p className="mt-1 text-sm">
            v{app.version} · {t(`app22.runtimeType.${app.runtime_type}`)}
          </p>
          <p className="mt-2 text-sm leading-6 text-[var(--cp-muted)]">
            {t(app.description_key)}
          </p>
        </div>
      </div>
      <dl>
        <Row label={t('app22.check.publisher')}>{app.publisher}</Row>
        <Row label={t('app22.owner')}>{store.scope.user_id}</Row>
        <Row label={t('app22.source.label')}>{draft.source.display_name}</Row>
      </dl>
      <details
        data-testid="app-installer-source-identity"
        className="rounded-xl border border-[var(--cp-border)] p-4"
      >
        <summary className="min-h-11 cursor-pointer py-3 text-sm font-semibold">
          {t('app22.check.identity')}
        </summary>
        <dl>
          <Row label="AppDID">{app.did}</Row>
          <Row label="AppDoc Object ID">{app.object_id}</Row>
          <Row label={t('app22.check.objectOwner')}>{app.owner}</Row>
          <Row label="controller">{app.controller}</Row>
          <Row label="author">{app.author}</Row>
          {draft.source.referrer && (
            <Row label={t('app22.referrer')}>{draft.source.referrer}</Row>
          )}
        </dl>
        <p className="mt-3 text-xs leading-5 text-[var(--cp-muted)]">
          {t('app22.check.envelope')}
        </p>
      </details>
      {draft.suggestions && (
        <p className="text-xs leading-5 text-[var(--cp-muted)]">
          {t('app22.suggestions')}
        </p>
      )}
      {draft.status === 'inspecting' && (
        <p role="status" className="flex items-center gap-2">
          <Loader2 className="animate-spin" size={17} />
          {t('app22.check.inspecting')}
        </p>
      )}
      {draft.readiness && <Readiness value={draft.readiness} />}
      {draft.error && <ErrorCard code={draft.error} />}
      {satisfied && (
        <section
          data-testid="app-installer-satisfied"
          className="rounded-xl bg-[var(--cp-surface-2)] p-4"
        >
          <h3 className="font-semibold">{t('app22.satisfied')}</h3>
          <p className="mt-2 text-sm">{t('app22.satisfiedBody')}</p>
        </section>
      )}
      {draft.plan?.plan_use === 'UPGRADE' && (
        <section
          data-testid="app-installer-upgrade"
          className="rounded-xl bg-[var(--cp-surface-2)] p-4"
        >
          <h3 className="font-semibold">{t('app22.upgrade')}</h3>
          <p className="mt-2">
            {draft.plan.previous_version} → {app.version}
          </p>
          <p className="mt-2 text-sm">{t('app22.upgradeImpact')}</p>
        </section>
      )}
      <footer className="flex flex-wrap justify-end gap-2">
        <Button onClick={onChangeSource}>{t('app22.changeSource')}</Button>
        {satisfied ? (
          <Button
            primary
            onClick={() => onViewApp(draft.plan!.app_instance_id)}
          >
            {t('app22.viewApp')}
          </Button>
        ) : (
          <Button
            primary
            disabled={
              draft.status === 'inspecting' ||
              (!local &&
                draft.status === 'blocked' &&
                !draft.readiness?.issues.every(
                  (issue) => issue.action === 'edit',
                ))
            }
            onClick={onConfigure}
          >
            {t('app22.configure')}
          </Button>
        )}
        {draft.status === 'blocked' &&
          draft.readiness?.issues.some((i) => i.action === 'recheck') && (
            <Button onClick={() => void store.inspectDraft(draft.draft_id)}>
              {t('app22.recheck')}
            </Button>
          )}
      </footer>
    </div>
  )
}
function PlanStep({
  draft,
  onBack,
  onSubmitted,
  onViewApp,
}: {
  draft: InspectionDraft
  onBack: () => void
  onSubmitted: (id: string) => void
  onViewApp: (id: string) => void
}) {
  const store = useSharedAppServiceStore()
  const { t } = useI18n()
  const requestSudo = useSudoByPassword()
  const local = ['local-pikg', 'personal-server-pikg'].includes(
    draft.source.kind,
  )
  const schema = useMemo(
    () => createInstallInputSchema(draft.app, store.targets, local),
    [draft.app, store.targets, local],
  )
  const form = useForm<InstallInput>({
    resolver: zodResolver(schema),
    defaultValues: draft.input,
  })
  const [busy, setBusy] = useState(false)
  const busyRef = useRef(false)
  const [error, setError] = useState<string | null>(null)
  const [validation, setValidation] = useState<
    Array<{ field: string; code: string }>
  >([])
  const alive = useRef(true)
  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
    }
  }, [])
  const update = () => {
    store.editDraft(draft.draft_id, form.getValues())
    setError(null)
    setValidation([])
  }
  const recheck = form.handleSubmit(
    async (input) => {
      store.editDraft(draft.draft_id, input)
      setValidation([])
      await store.inspectDraft(draft.draft_id)
    },
    () => {
      const parsed = schema.safeParse(form.getValues())
      if (!parsed.success)
        setValidation(
          parsed.error.issues.map((i) => ({
            field: i.path.join('.'),
            code: i.code === 'custom' ? i.message : 'INVALID_FIELD',
          })),
        )
    },
  )
  const approve = async () => {
    const plan = draft.plan
    if (!plan || draft.status !== 'ready' || busyRef.current) return
    busyRef.current = true
    setBusy(true)
    setError(null)
    try {
      const grant = await requestSudo({
        username: store.scope.user_id,
        appid: 'control-panel',
        appInstanceId: plan.app_instance_id,
        aud: 'apps.submit',
        title: t('app22.authorize'),
        reason: t('app22.authReason', undefined, { name: draft.app.show_name }),
        confirmLabel: t('app22.authConfirm'),
        requestPassword: (params) => store.authorize(params, draft.draft_id),
      })
      if (!grant || !alive.current) return
      const result = await store.submitDraft(
        draft.draft_id,
        plan.plan_fingerprint,
        `${draft.draft_id}:${plan.plan_fingerprint}`,
        grant,
      )
      if (result.action === 'submitted') onSubmitted(result.task_id)
      else onViewApp(result.app_instance_id)
    } catch (e) {
      if (alive.current) setError(e instanceof Error ? e.message : 'UNKNOWN')
    } finally {
      busyRef.current = false
      if (alive.current) setBusy(false)
    }
  }
  const p = draft.input.install_params
  return (
    <div className="space-y-5">
      <h2 className="text-lg font-semibold">
        {t(
          draft.plan?.plan_use === 'UPGRADE'
            ? 'app22.upgradePlan'
            : 'app22.installPlan',
        )}
      </h2>
      <dl className="rounded-xl bg-[var(--cp-surface-2)] p-4">
        <Row label={t('app22.owner')}>{store.scope.user_id}</Row>
        <Row label={t('app22.target')}>
          {store.targets.find((n) => n.node_id === draft.input.target_node_id)
            ?.label ?? t('app22.unknown')}
        </Row>
        <Row label={t('app22.platform')}>
          {
            store.targets.find((n) => n.node_id === draft.input.target_node_id)
              ?.os
          }{' '}
          /{' '}
          {
            store.targets.find((n) => n.node_id === draft.input.target_node_id)
              ?.arch
          }
        </Row>
        <Row label={t('app22.address')}>
          {store.getById(`${draft.app.app_id}@${store.scope.user_id}`)?.record
            ?.address ?? t('app22.assigned')}
        </Row>
      </dl>
      <form
        onSubmit={recheck}
        noValidate
        className="space-y-5"
        onChange={update}
      >
        <fieldset
          disabled={busy || draft.status === 'submitting'}
          className="space-y-5 disabled:opacity-60"
        >
          <details>
            <summary className="min-h-11 cursor-pointer py-3 text-sm font-semibold">
              {t('app22.advancedTarget')}
            </summary>
            <label className="block text-sm">
              {t('app22.target')}
              <select
                {...form.register('target_node_id')}
                className="mt-2 min-h-11 w-full rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-3"
              >
                {store.targets.map((node) => (
                  <option key={node.node_id} value={node.node_id}>
                    {node.label} · {node.os}/{node.arch}
                  </option>
                ))}
              </select>
            </label>
          </details>
          <label className="flex min-h-11 items-center gap-3 text-sm">
            <input type="checkbox" {...form.register('offline')} />
            {t('app22.offline')}
          </label>
          {local && (
            <label className="block text-sm">
              {t('app22.policy')}
              <select
                aria-label={t('app22.policy')}
                {...form.register('policy')}
                className="mt-2 min-h-11 w-full rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-3"
              >
                <option value="NORMAL">{t('app22.policy.NORMAL')}</option>
                <option value="LOCAL_DEVELOPER">
                  {t('app22.policy.LOCAL_DEVELOPER')}
                </option>
              </select>
            </label>
          )}
          {draft.app.components.some((c) => !c.required) && (
            <section>
              <h3 className="text-sm font-semibold">{t('app22.components')}</h3>
              {draft.app.components.map((component) => (
                <label
                  key={component.name}
                  className="flex min-h-11 items-center gap-3 text-sm"
                >
                  <input
                    type="checkbox"
                    checked={p.selected_components.includes(component.name)}
                    disabled={component.required}
                    onChange={(e) => {
                      const values = form.getValues(
                        'install_params.selected_components',
                      )
                      form.setValue(
                        'install_params.selected_components',
                        e.target.checked
                          ? [...values, component.name]
                          : values.filter((v) => v !== component.name),
                      )
                      update()
                    }}
                  />
                  {component.name}
                  {component.required && ` · ${t('app22.required')}`}
                </label>
              ))}
            </section>
          )}
          {draft.app.endpoints.length > 0 && (
            <section
              data-testid="app-installer-access-settings"
              className="space-y-3"
            >
              <h3 className="text-sm font-semibold">{t('app22.services')}</h3>
              {draft.app.endpoints.map((endpoint) => {
                const service = p.service_settings.services[endpoint.name]
                return (
                  <div
                    key={endpoint.name}
                    className="space-y-3 rounded-xl border border-[var(--cp-border)] p-4"
                  >
                    <label className="flex min-h-11 items-center gap-3 text-sm">
                      <input
                        type="checkbox"
                        checked={service.enabled}
                        disabled={endpoint.required}
                        onChange={(e) => {
                          form.setValue(
                            `install_params.service_settings.services.${endpoint.name}.enabled`,
                            e.target.checked,
                          )
                          update()
                        }}
                      />
                      {endpoint.name} · {endpoint.protocol}:
                      {endpoint.inner_port}
                      {endpoint.required && ` · ${t('app22.required')}`}
                    </label>
                    <label className="block text-sm">
                      {t('app22.exposure')}
                      <select
                        {...form.register(
                          `install_params.service_settings.services.${endpoint.name}.expose.scope`,
                        )}
                        className="mt-2 min-h-11 w-full rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-3"
                      >
                        <option value="zone">{t('app22.zoneOnly')}</option>
                        <option value="">{t('app22.public')}</option>
                      </select>
                    </label>
                    {endpoint.route === 'port' && (
                      <label className="block text-sm">
                        {t('app22.port')}
                        <input
                          type="number"
                          {...form.register(
                            `install_params.service_settings.services.${endpoint.name}.expose.route.expose_port`,
                            { valueAsNumber: true },
                          )}
                          className="mt-2 min-h-11 w-full rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-3"
                        />
                      </label>
                    )}
                    <label className="flex min-h-11 items-center gap-3 text-sm">
                      <input
                        type="checkbox"
                        {...form.register(
                          `install_params.service_settings.services.${endpoint.name}.expose.allow_guest`,
                        )}
                      />
                      {t('app22.guest')}
                    </label>
                  </div>
                )
              })}
              <p className="text-xs leading-5 text-[var(--cp-muted)]">
                {t('app22.shortcutLater')}
              </p>
            </section>
          )}
          <section data-testid="app-installer-permissions">
            <h3 className="text-sm font-semibold">{t('app22.permissions')}</h3>
            {draft.app.permissions.map((permission) => (
              <label
                key={permission.scope_path}
                className="flex min-h-14 items-center gap-3 text-sm"
              >
                <input
                  type="checkbox"
                  disabled={permission.required}
                  checked={p.permissions.some(
                    (v) => v.scope_path === permission.scope_path,
                  )}
                  onChange={(e) => {
                    const values = form.getValues('install_params.permissions')
                    form.setValue(
                      'install_params.permissions',
                      e.target.checked
                        ? [...values, permission]
                        : values.filter(
                            (v) => v.scope_path !== permission.scope_path,
                          ),
                    )
                    update()
                  }}
                />
                <span className="min-w-0 break-all">
                  {permission.scope_path} · {permission.actions.join(', ')} ·{' '}
                  {t(permission.required ? 'app22.required' : 'app22.optional')}
                </span>
              </label>
            ))}
          </section>
          <section
            data-testid="app-installer-mount-settings"
            className="space-y-3"
          >
            <h3 className="text-sm font-semibold">{t('app22.storage')}</h3>
            {(['data', 'local_cache', 'external'] as const).map((kind) => (
              <div
                key={kind}
                className="rounded-xl border border-[var(--cp-border)] p-4"
              >
                <h4 className="text-sm font-semibold">
                  {t(`app22.mount.${kind}`)}
                </h4>
                <p className="mt-2 text-xs leading-5 text-[var(--cp-muted)]">
                  {t(`app22.mount.${kind}Hint`)}
                </p>
                {draft.app.mounts
                  .filter((m) => m.kind === kind)
                  .map((mount) => (
                    <label
                      key={mount.path}
                      className="mt-3 flex min-h-11 items-center gap-3 text-xs"
                    >
                      <input
                        type="checkbox"
                        disabled={mount.required}
                        checked={Boolean(p[`${kind}_mount_points`][mount.path])}
                        onChange={(e) => {
                          const map = {
                            ...form.getValues(
                              `install_params.${kind}_mount_points`,
                            ),
                          }
                          if (e.target.checked)
                            map[mount.path] = {
                              target_path: mount.target_path,
                              access: mount.access,
                            }
                          else delete map[mount.path]
                          form.setValue(
                            `install_params.${kind}_mount_points`,
                            map,
                          )
                          update()
                        }}
                      />
                      <span className="break-all">
                        {mount.path} → {mount.target_path} ·{' '}
                        {t(`app22.access.${mount.access}`)}
                      </span>
                    </label>
                  ))}
              </div>
            ))}
          </section>
          <section
            data-testid="app-installer-environment-settings"
            className="space-y-3"
          >
            <h3 className="text-sm font-semibold">{t('app22.environment')}</h3>
            {draft.app.environment.map((env) =>
              env.system ? (
                <p
                  key={env.name}
                  className="break-all text-xs leading-5 text-[var(--cp-muted)]"
                >
                  {env.name} · {t('app22.systemInjected')}
                </p>
              ) : (
                <label key={env.name} className="block text-sm">
                  {env.name}
                  {env.required && ` · ${t('app22.required')}`}
                  <input
                    type={env.sensitive ? 'password' : 'text'}
                    autoComplete="off"
                    {...form.register(`install_params.bash_envs.${env.name}`)}
                    className="mt-2 min-h-11 w-full rounded-xl border border-[var(--cp-border)] bg-[var(--cp-surface)] px-3"
                  />
                </label>
              ),
            )}
          </section>
          {(draft.app.start_param || draft.app.container_param) && (
            <details data-testid="app-installer-risky-params">
              <summary className="min-h-11 cursor-pointer py-3 text-sm font-semibold">
                {t('app22.risky')}
              </summary>
              <p className="text-xs leading-5 text-[var(--cp-muted)]">
                {t('app22.riskyHint')}
              </p>
              <pre className="mt-2 whitespace-pre-wrap break-all text-xs">
                {draft.app.start_param}
                {'\n'}
                {draft.app.container_param}
              </pre>
            </details>
          )}
          <label className="flex min-h-11 items-center gap-3 text-sm">
            <input
              type="checkbox"
              {...form.register('install_params.auto_start')}
            />
            {t('app22.autoStart')}
          </label>
        </fieldset>
        {validation.map((issue, i) => (
          <div key={i}>
            <ErrorCard code={issue.code} />
            <p className="mt-1 break-all text-xs">{issue.field}</p>
          </div>
        ))}
        {(draft.status === 'dirty' || draft.status === 'stale') && (
          <p role="status" className="text-sm">
            {t('app22.dirty')}
          </p>
        )}
        <Button
          type="submit"
          disabled={
            busy ||
            draft.status === 'inspecting' ||
            draft.status === 'submitting'
          }
          testId="app-installer-recheck"
        >
          {t(
            draft.status === 'inspecting'
              ? 'app22.rechecking'
              : 'app22.recheck',
          )}
        </Button>
      </form>
      {draft.readiness && <Readiness value={draft.readiness} />}
      {draft.plan && draft.status === 'ready' && (
        <section
          data-testid="app-installer-approved-summary"
          className="rounded-xl bg-[var(--cp-surface-2)] p-4"
        >
          <h3 className="text-sm font-semibold">{t('app22.finalSummary')}</h3>
          <dl>
            <Row label={t('app22.target')}>
              {draft.plan.target.label} · {draft.plan.target.os}/
              {draft.plan.target.arch}
            </Row>
            <Row label={t('app22.version')}>
              {draft.plan.previous_version
                ? `${draft.plan.previous_version} → `
                : ''}
              {draft.app.version}
            </Row>
            <Row label={t('app22.download')}>
              {draft.plan.download_bytes === null
                ? t('app22.unknown')
                : `${Math.ceil(draft.plan.download_bytes / 1048576)} MB`}
            </Row>
            <Row label={t('app22.components')}>
              {p.selected_components.join(', ')}
            </Row>
            <Row label={t('app22.policy')}>
              {t(`app22.policy.${draft.input.policy}`)} · {t('app22.offline')}:{' '}
              {t(draft.input.offline ? 'app22.yes' : 'app22.no')}
            </Row>
            <Row label={t('app22.services')}>
              {Object.entries(p.service_settings.services)
                .map(
                  ([name, service]) =>
                    `${name}: ${t(service.enabled ? 'app22.yes' : 'app22.no')} · ${t(service.expose.scope === 'zone' ? 'app22.zoneOnly' : 'app22.public')} · ${t('app22.guest')}: ${t(service.expose.allow_guest ? 'app22.yes' : 'app22.no')}${service.expose.route.type === 'port' ? ` · ${service.expose.route.expose_port}` : ''}`,
                )
                .join('; ')}
            </Row>
            <Row label={t('app22.storage')}>
              {(['data', 'local_cache', 'external'] as const)
                .map(
                  (kind) =>
                    `${t(`app22.mount.${kind}`)}: ${
                      Object.entries(p[`${kind}_mount_points`])
                        .map(
                          ([path, mount]) =>
                            `${path} → ${mount.target_path} (${t(`app22.access.${mount.access}`)})`,
                        )
                        .join(', ') || '—'
                    }`,
                )
                .join('; ')}
            </Row>
            <Row label={t('app22.permissions')}>
              {p.permissions
                .map((v) => `${v.scope_path} (${v.actions.join(', ')})`)
                .join('; ')}
            </Row>
            <Row label={t('app22.autoStart')}>
              {t(p.auto_start ? 'app22.yes' : 'app22.no')}
            </Row>
            <Row label={t('app22.environment')}>
              {Object.keys(p.bash_envs)
                .map(
                  (name) =>
                    `${name}=${draft.app.environment.find((e) => e.name === name)?.sensitive ? '••••••' : p.bash_envs[name]}`,
                )
                .join('; ')}
            </Row>
          </dl>
          <p className="mt-3 text-sm">
            {t(
              draft.plan.plan_use === 'UPGRADE'
                ? 'app22.upgradeImpact'
                : 'app22.installImpact',
            )}
          </p>
          <details className="mt-3">
            <summary className="min-h-11 cursor-pointer py-3 text-xs">
              {t('app22.diagnostics')}
            </summary>
            <code className="block break-all text-xs">
              {draft.plan.plan_fingerprint}
            </code>
          </details>
        </section>
      )}
      {error && <ErrorCard code={error} />}
      <footer className="flex flex-wrap justify-between gap-3 border-t border-[var(--cp-border)] pt-4">
        <Button disabled={busy} onClick={onBack}>
          {t('common.back')}
        </Button>
        <Button
          primary
          testId="app-installer-submit"
          disabled={
            busy || draft.status !== 'ready' || store.scope.role !== 'admin'
          }
          onClick={() => void approve()}
        >
          {t(
            busy
              ? 'app22.submitting'
              : draft.plan?.plan_use === 'UPGRADE'
                ? 'app22.confirmUpgrade'
                : 'app22.confirmInstall',
          )}
        </Button>
      </footer>
      {store.scope.role !== 'admin' && <ErrorCard code="ADMIN_REQUIRED" />}
    </div>
  )
}
function TaskStep({
  task,
  onFollowTask,
  onViewApp,
  onBackground,
  onChangeSource,
  onInspect,
}: {
  task: InstallTask
  onInspect: (id: string) => void
  onFollowTask: (id: string) => void
  onViewApp: (id: string) => void
  onBackground: () => void
  onChangeSource: () => void
}) {
  const store = useSharedAppServiceStore()
  const { t } = useI18n()
  const [copied, setCopied] = useState(false)
  const currentService = store.getById(task.app_instance_id)
  const service =
    currentService &&
    task.result?.deployment &&
    currentService.runtime.expected_deployment?.task_id !== task.task_id
      ? {
          ...currentService,
          runtime: {
            ...emptyRuntime(task.app.runtime_type),
            expected_deployment: task.result.deployment,
            reason: 'STALE_EVIDENCE',
          },
        }
      : currentService
  return (
    <div className="space-y-5">
      <div className="flex items-start gap-3">
        {task.phase === 'Running' ? (
          <Loader2 className="animate-spin" size={24} />
        ) : (
          <PackageCheck size={24} />
        )}
        <div>
          <h2 className="text-lg font-semibold">
            {t(
              task.outcome === 'Succeeded'
                ? 'app22.installComplete'
                : task.outcome === 'Failed'
                  ? 'app22.installFailed'
                  : task.outcome === 'Canceled'
                    ? 'app22.installCanceled'
                    : task.phase === 'Waiting'
                      ? 'app22.installWaiting'
                      : task.phase === 'Unknown'
                        ? 'app22.unknown'
                        : 'app22.executing',
            )}
          </h2>
          <p className="mt-1 text-sm">
            {task.app.show_name} · v{task.app.version}
          </p>
        </div>
      </div>
      <p
        data-testid="app-installer-task-id"
        className="break-all text-xs text-[var(--cp-muted)]"
      >
        {task.task_id}
      </p>
      {task.retry_of && (
        <div className="text-sm">
          {t('app22.previousAttempt')}
          <Button onClick={() => onFollowTask(task.retry_of!)}>
            {task.retry_of}
          </Button>
        </div>
      )}
      {task.phase !== 'Terminal' && (
        <section
          role="status"
          className="rounded-xl bg-[var(--cp-surface-2)] p-4"
        >
          <p className="text-sm font-semibold">
            {t(
              task.phase === 'Waiting'
                ? 'app22.waitingContent'
                : `app22.stage.${task.stage}`,
              t('app22.unknown'),
            )}
          </p>
          {task.progress !== null && (
            <progress
              value={task.progress}
              max={100}
              aria-label={t('app22.progress')}
            />
          )}
          <details className="mt-2">
            <summary className="min-h-11 cursor-pointer py-3 text-xs">
              {t('app22.diagnostics')}
            </summary>
            <p className="break-all text-xs">
              {task.schema_id} · {task.phase} · {task.outcome ?? '—'} ·{' '}
              {task.stage}
            </p>
          </details>
        </section>
      )}
      {task.desired_state_committed && (
        <p
          data-testid="app-installer-commit-boundary"
          className="rounded-xl border border-[var(--cp-border)] p-4 text-sm"
        >
          {t('app22.committed')}
        </p>
      )}
      {task.error && <ErrorCard code={task.error.code} />}
      {task.result && (
        <section className="rounded-xl border border-[var(--cp-border)] p-4">
          <h3 className="text-sm font-semibold">{t('app22.installResult')}</h3>
          <dl>
            <Row label={t('app22.version')}>{task.result.version}</Row>
            <Row label={t('app22.owner')}>{task.owner_user_id}</Row>
            <Row label="AppInstanceId">{task.app_instance_id}</Row>
            <Row label={t('app22.address')}>
              {task.result.address ?? t('app22.assigned')}
            </Row>
          </dl>
          <details>
            <summary className="min-h-11 cursor-pointer py-3 text-xs">
              {t('app22.diagnostics')}
            </summary>
            <dl>
              <Row label="AppName">
                {task.result.app_name ?? t('app22.assigned')}
              </Row>
              <Row label="AppHostName">
                {task.result.app_host_name ?? t('app22.assigned')}
              </Row>
              <Row label="AppIndex">
                {task.result.app_index ?? t('app22.assigned')}
              </Row>
            </dl>
          </details>
        </section>
      )}
      {task.result && service && <RuntimeSummary service={service} />}
      <details className="rounded-xl border border-[var(--cp-border)] p-4">
        <summary className="min-h-11 cursor-pointer py-3 text-sm">
          {t('app22.approvedPlan')}
        </summary>
        <dl>
          <Row label={t('app22.target')}>
            {task.plan.target.label} · {task.plan.target.os}/
            {task.plan.target.arch}
          </Row>
          <Row label={t('app22.components')}>
            {task.plan.input.install_params.selected_components.join(', ')}
          </Row>
          <Row label="AppDID">{task.app.did}</Row>
          <Row label="AppDoc Object ID">{task.app.object_id}</Row>
        </dl>
      </details>
      <footer className="flex flex-wrap gap-2 border-t border-[var(--cp-border)] pt-4">
        {task.available_actions.includes('cancel') &&
          !task.desired_state_committed && (
            <Button
              onClick={() => {
                store.cancelTask(task.task_id)
              }}
            >
              {t('app22.cancelTask')}
            </Button>
          )}
        {task.available_actions.includes('inspect') && (
          <Button
            onClick={() => {
              const id = store.inspectTask(task.task_id)
              if (id) onInspect(id)
            }}
          >
            {t('app22.recheck')}
          </Button>
        )}
        {task.available_actions.includes('retry') && task.error?.retryable && (
          <Button
            primary
            onClick={() => {
              const id = store.retryTask(task.task_id)
              if (id) onFollowTask(id)
            }}
          >
            {t('common.retry')}
          </Button>
        )}
        {task.available_actions.includes('resume') && (
          <Button
            primary
            onClick={() => {
              const id = store.resumeTask(task.task_id)
              if (id) onFollowTask(id)
            }}
          >
            {t('app22.resume')}
          </Button>
        )}
        {task.available_actions.includes('change-source') &&
          !task.desired_state_committed && (
            <Button onClick={onChangeSource}>{t('app22.changeSource')}</Button>
          )}
        {task.error && (
          <Button
            onClick={() => {
              void navigator.clipboard
                .writeText(store.safeDiagnostics(task))
                .then(
                  () => setCopied(true),
                  () => setCopied(false),
                )
            }}
          >
            {t(copied ? 'common.copied' : 'app22.copyDetails')}
          </Button>
        )}
        <a
          className="inline-flex min-h-11 items-center rounded-xl border border-[var(--cp-border)] px-4 text-sm"
          href={`/taskcenter?taskid=${encodeURIComponent(task.task_id)}`}
        >
          {t('app22.taskCenter')}
        </a>
        <Button onClick={onBackground}>
          {t(task.phase === 'Terminal' ? 'common.close' : 'app22.background')}
        </Button>
        {task.result && (
          <Button primary onClick={() => onViewApp(task.app_instance_id)}>
            {t('app22.viewApp')}
          </Button>
        )}
      </footer>
    </div>
  )
}
export interface AppInstallerProps {
  launchParams: AppInstallerInternalParams
  onBackground: () => void
  onChangeSource: () => void
  onClose: () => void
  onViewApp: (id: string) => void
  onTaskCreated?: (id: string) => void
}
function InstallerContent({
  launchParams,
  onBackground,
  onChangeSource,
  onClose,
  onViewApp,
  onTaskCreated,
}: AppInstallerProps) {
  const store = useSharedAppServiceStore()
  const { t } = useI18n()
  const [state, setState] = useState<{
    draft_id?: string
    task_id?: string
    error?: string
  }>({})
  const [configure, setConfigure] = useState(false)
  const launchKey = JSON.stringify(launchParams)
  const scopeKey = `${store.scope.zone_id}:${store.scope.user_id}:${store.scope.role}`
  useEffect(() => {
    let canceled = false
    const controller = new AbortController()
    const launch = JSON.parse(launchKey) as AppInstallerInternalParams
    const resolve = async () => {
      setState({})
      setConfigure(false)
      const keys = Object.keys(launch)
      if ('task_id' in launch && keys.length !== 1) {
        setState({ error: 'conflicting_parameters' })
        return
      }
      if (
        'task_id' in launch &&
        !taskIdSchema.safeParse(launch.task_id).success
      ) {
        setState({ error: 'invalid_task_id' })
        return
      }
      if (
        'draft_id' in launch &&
        (keys.length !== 1 || typeof launch.draft_id !== 'string')
      ) {
        setState({ error: 'unknown_parameter' })
        return
      }
      if (
        'identifier' in launch &&
        keys.some((key) => !['identifier', 'ref', 'options'].includes(key))
      ) {
        setState({ error: 'unknown_parameter' })
        return
      }
      if ('task_id' in launch) {
        setState({ task_id: launch.task_id })
        return
      }
      if ('draft_id' in launch) {
        setState({ draft_id: launch.draft_id })
        await store.inspectDraft(launch.draft_id)
        return
      }
      const query = new URLSearchParams({
        identifier: launch.identifier,
        ...(launch.ref ? { ref: launch.ref } : {}),
        ...(launch.options ? { options: JSON.stringify(launch.options) } : {}),
      })
      const valid = parseAppInstallerLaunchQuery(query.toString())
      if (!valid.ok) {
        setState({ error: valid.code })
        return
      }
      const result = await store.analyzeInstallSource(
        launch.identifier,
        undefined,
        controller.signal,
      )
      if (canceled) return
      if (!result.ok) {
        setState({ error: result.code })
        return
      }
      result.source.referrer = launch.ref
      const node = targets.find(
        (n) =>
          n.node_id === launch.options?.target?.node_id ||
          n.node_did === launch.options?.target?.node_did,
      )
      const autoStart = launch.options?.install_params?.auto_start
      const existingDraft = Object.values(store.drafts).find(
        (draft) =>
          draft.source.kind === 'identifier' &&
          draft.source.display_name === result.source.display_name,
      )
      const draft_id =
        existingDraft?.draft_id ??
        store.createDraft(
          result.source,
          launch.options
            ? {
                target_node_id: node?.node_id,
                offline: launch.options.offline,
                auto_start:
                  typeof autoStart === 'boolean' ? autoStart : undefined,
              }
            : undefined,
        )
      setState({ draft_id })
      await store.inspectDraft(draft_id)
    }
    void resolve()
    return () => {
      canceled = true
      controller.abort()
    }
  }, [launchKey, scopeKey, store])
  const draft = state.draft_id ? store.getDraft(state.draft_id) : null
  const task = state.task_id ? store.getTask(state.task_id) : null
  const exit = (callback: () => void) => {
    if (draft) store.releaseDraft(draft.draft_id)
    callback()
  }
  const followTask = (task_id: string) => {
    setState({ task_id })
    onTaskCreated?.(task_id)
  }
  const error =
    state.error ??
    (state.task_id && !task
      ? store.taskReadError(state.task_id)
      : state.draft_id && !draft
        ? 'DRAFT_NOT_FOUND'
        : null)
  return (
    <section
      data-testid="app-installer-dialog"
      className="min-w-0 rounded-[22px] border border-[var(--cp-border)] bg-[var(--cp-surface)] text-[var(--cp-text)] shadow-[var(--cp-window-shadow)]"
    >
      <header className="flex items-start justify-between gap-3 border-b border-[var(--cp-border)] p-5">
        <div>
          <h1 className="font-display text-lg font-semibold">
            {t('app22.title')}
          </h1>
          <p className="mt-1 text-xs text-[var(--cp-muted)]">
            {t(task ? 'app22.taskHint' : 'app22.draftHint')}
          </p>
        </div>
        <button
          type="button"
          disabled={draft?.status === 'submitting'}
          onClick={() => exit(task ? onBackground : onClose)}
          aria-label={t('common.close')}
          className="flex size-11 shrink-0 items-center justify-center rounded-xl"
        >
          <X size={19} />
        </button>
      </header>
      <div className="space-y-5 p-5 sm:p-6">
        {error ? (
          <div data-testid="app-installer-launch-error">
            <ErrorCard code={error} />
            <div className="mt-4">
              <Button onClick={() => exit(onClose)}>{t('common.close')}</Button>
            </div>
          </div>
        ) : task ? (
          <TaskStep
            key={task.task_id}
            task={task}
            onInspect={(id) => {
              setState({ draft_id: id })
              setConfigure(false)
              void store.inspectDraft(id)
            }}
            onFollowTask={followTask}
            onViewApp={onViewApp}
            onBackground={onBackground}
            onChangeSource={onChangeSource}
          />
        ) : draft ? (
          configure ? (
            <PlanStep
              key={draft.draft_id}
              draft={draft}
              onBack={() => setConfigure(false)}
              onSubmitted={followTask}
              onViewApp={(id) => exit(() => onViewApp(id))}
            />
          ) : (
            <CheckStep
              draft={draft}
              onConfigure={() => setConfigure(true)}
              onViewApp={(id) => exit(() => onViewApp(id))}
              onChangeSource={() => exit(onChangeSource)}
            />
          )
        ) : (
          <p role="status" className="flex items-center gap-2">
            <Loader2 className="animate-spin" size={17} />
            {t('app22.check.inspecting')}
          </p>
        )}
      </div>
    </section>
  )
}
export function AppInstaller(props: AppInstallerProps) {
  const mobile = useMediaQuery('(max-width: 767px)')
  return (
    <WindowDialogProvider
      permissions={{ fullscreen: true }}
      surface={mobile ? 'mobile' : 'desktop'}
    >
      <InstallerContent {...props} />
    </WindowDialogProvider>
  )
}
export function AppInstallerRoute() {
  const location = useLocation()
  const navigate = useNavigate()
  const { t } = useI18n()
  const [detailId, setDetailId] = useState<string | null>(null)
  const parsed = useMemo(
    () => parseAppInstallerLaunchQuery(location.search),
    [location.search],
  )
  const close = () => {
    void navigate('/')
  }
  const normalize = (id: string) => {
    const url = new URL(window.location.href)
    url.search = new URLSearchParams({ task_id: id }).toString()
    window.history.replaceState(
      window.history.state,
      '',
      `${url.pathname}${url.search}`,
    )
  }
  return (
    <main className="relative z-10 min-h-dvh overflow-x-hidden px-3 py-5 sm:p-6">
      <div className="mx-auto w-full max-w-3xl">
        {detailId ? (
          <AppServiceStoreProvider>
            <DetailPage
              serviceId={detailId}
              onNavigate={() => setDetailId(null)}
            />
          </AppServiceStoreProvider>
        ) : parsed.ok ? (
          <AppInstaller
            launchParams={parsed.params}
            onBackground={close}
            onChangeSource={close}
            onClose={close}
            onViewApp={setDetailId}
            onTaskCreated={normalize}
          />
        ) : (
          <div
            data-testid="app-installer-launch-error"
            className="rounded-xl bg-[var(--cp-surface)] p-5"
          >
            <ErrorCard code={parsed.code} />
            <Button onClick={close}>{t('common.close')}</Button>
          </div>
        )}
      </div>
    </main>
  )
}
