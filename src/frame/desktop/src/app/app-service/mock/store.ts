import {
  appDocCandidateSchema,
  createInstallInputSchema,
  hostnameDidSchema,
  type InstallInput,
} from '../schemas'
import type {
  AppDocumentView,
  AppServiceItem,
  AppServiceViewStatus,
  InspectionDraft,
  InstallPlan,
  InstallTask,
  MockScenario,
  PickedPikgFile,
  PlanReadiness,
  ScopeContext,
  SourceParseResult,
  SourceReference,
  SubmitResult,
  RuntimeView,
} from '../types'
import type { SudoByPasswordParams, SudoGrant } from '../../../components/sudo'
import {
  appDocument,
  defaultInput,
  emptyRuntime,
  fixtureFor,
  scenarios,
  seedServices,
  targets,
} from './fixtures'

export const MODEL_VERSION = 4
export const contextStorageKey = 'buckyos.app-service.context.v4'
export function readScope(): ScopeContext {
  try {
    const value = JSON.parse(
      localStorage.getItem(contextStorageKey) ?? 'null',
    ) as ScopeContext | null
    if (
      value &&
      /^[a-z0-9.-]+$/.test(value.zone_id) &&
      /^[a-z0-9-]+$/.test(value.user_id) &&
      ['admin', 'user'].includes(value.role)
    )
      return value
  } catch {
    return { zone_id: 'prototype-zone', user_id: 'alice', role: 'admin' }
  }
  return { zone_id: 'prototype-zone', user_id: 'alice', role: 'admin' }
}
export function scopeStorageKey(scope: ScopeContext = readScope()) {
  return `buckyos.app-service.v${MODEL_VERSION}:${scope.zone_id}:${scope.user_id}:${scope.role}`
}
const delay = (ms: number) =>
  new Promise<void>((resolve) => window.setTimeout(resolve, ms))
const uid = (prefix: string) =>
  `${prefix}-${crypto.randomUUID().replaceAll('-', '')}`
const clone = <T>(value: T): T => structuredClone(value)
function safeInput(input: InstallInput, app: AppDocumentView): InstallInput {
  const result = clone(input)
  for (const env of app.environment)
    if (env.sensitive) result.install_params.bash_envs[env.name] = ''
  return result
}
function safePlan(plan: InstallPlan): InstallPlan {
  return { ...clone(plan), input: safeInput(plan.input, plan.app) }
}
function freezePlan(plan: InstallPlan): InstallPlan {
  const freeze = (value: unknown) => {
    if (value && typeof value === 'object') {
      Object.values(value).forEach(freeze)
      Object.freeze(value)
    }
  }
  freeze(plan)
  return plan
}
async function fingerprint(value: unknown) {
  const canonical = (v: unknown): unknown =>
    Array.isArray(v)
      ? v.map(canonical)
      : v && typeof v === 'object'
        ? Object.fromEntries(
            Object.entries(v)
              .sort(([a], [b]) => a.localeCompare(b))
              .map(([k, x]) => [k, canonical(x)]),
          )
        : v
  const bytes = await crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode(JSON.stringify(canonical(value))),
  )
  return `planfp:${Array.from(new Uint8Array(bytes), (n) => n.toString(16).padStart(2, '0')).join('')}`
}
export function runtimeEvidenceValid(runtime: RuntimeView) {
  return Boolean(
    runtime.evidence &&
    runtime.expected_deployment &&
    runtime.evidence.observed_at <= Date.now() &&
    runtime.evidence.valid_until > Date.now() &&
    runtime.evidence.valid_until > runtime.evidence.observed_at &&
    JSON.stringify(runtime.evidence.deployment) ===
      JSON.stringify(runtime.expected_deployment),
  )
}

export function runtimeStatus(runtime: RuntimeView) {
  return runtime.status === 'running' && !runtimeEvidenceValid(runtime)
    ? 'unknown'
    : runtime.status
}

export class AppServiceMockStore {
  scope = readScope()
  services: AppServiceItem[] = []
  tasks: Record<string, InstallTask> = {}
  drafts: Record<string, InspectionDraft> = {}
  viewStatus: AppServiceViewStatus = 'loading'
  readonly targets = targets
  private revision = 0
  private listeners = new Set<() => void>()
  private timers = new Set<number>()
  private idempotency: Record<
    string,
    { fingerprint: string; result: SubmitResult }
  > = {}
  private grants = new Map<
    string,
    { fingerprint: string; expires: number; scope: string }
  >()
  private staleSeen = new Set<string>()
  private running = new Set<string>()
  private pendingSubmissions = new Map<string, Promise<SubmitResult>>()

  constructor() {
    this.load()
    window.addEventListener('storage', (event) => {
      if (event.key !== contextStorageKey) return
      this.timers.forEach(window.clearTimeout)
      this.timers.clear()
      this.running.clear()
      this.grants.clear()
      this.scope = readScope()
      this.load()
      this.emit()
    })
  }
  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }
  getRevision = () => this.revision
  private load() {
    this.tasks = {}
    this.drafts = {}
    this.idempotency = {}
    this.services = seedServices()
    try {
      const data = JSON.parse(
        localStorage.getItem(scopeStorageKey(this.scope)) ?? 'null',
      )
      if (
        data?.model_version === MODEL_VERSION &&
        data.scope === scopeStorageKey(this.scope)
      ) {
        this.tasks = data.tasks ?? {}
        for (const task of Object.values(this.tasks)) {
          if (
            !['Running', 'Waiting', 'Terminal'].includes(task.phase) ||
            (task.outcome !== null &&
              !['Succeeded', 'Failed', 'Canceled'].includes(task.outcome))
          ) {
            task.phase = 'Unknown'
            task.available_actions = []
          }
          if (
            !['acquire', 'verify', 'prepare', 'deploy', 'activate'].includes(
              task.stage,
            )
          )
            task.stage = 'unknown'
        }
        this.services = data.services ?? this.services
        this.idempotency = data.idempotency ?? {}
        for (const draft of Object.values(
          data.drafts ?? {},
        ) as InspectionDraft[])
          this.drafts[draft.draft_id] = {
            ...draft,
            plan: null,
            readiness: null,
            status: 'dirty',
            error: null,
          }
      }
    } catch {
      this.tasks = {}
      this.drafts = {}
      this.idempotency = {}
      this.services = seedServices()
    }
    const scenario = new URLSearchParams(location.search).get(
      'appServiceScenario',
    )
    if (scenario === 'empty')
      this.services = this.services.filter((s) => s.layer !== 'app')
    this.viewStatus = 'loading'
    this.schedule(
      () => {
        this.viewStatus = scenario === 'error' ? 'error' : 'ready'
        Object.values(this.tasks).forEach((task) => {
          if (task.phase === 'Running') this.runTask(task.task_id)
          if (
            task.outcome === 'Succeeded' &&
            ['starting', 'deploying'].includes(
              this.getById(task.app_instance_id)?.status ?? '',
            )
          )
            this.resumeRuntime(task)
        })
        this.emit()
      },
      scenario === 'loading' ? 1600 : 220,
    )
  }
  retryLoad() {
    this.viewStatus = 'loading'
    this.emit()
    this.schedule(() => {
      this.viewStatus = 'ready'
      this.emit()
    }, 400)
  }
  getAllServices() {
    return this.services.filter(
      (s) =>
        s.layer !== 'app' ||
        this.scope.role === 'admin' ||
        s.owner_user_id === this.scope.user_id,
    )
  }
  getByLayer(layer: AppServiceItem['layer']) {
    return this.getAllServices().filter((s) => s.layer === layer)
  }
  getById(id: string) {
    return this.getAllServices().find((s) => s.id === id) ?? null
  }
  getTasks() {
    return Object.values(this.tasks)
      .filter(
        (task) =>
          this.scope.role === 'admin' ||
          task.owner_user_id === this.scope.user_id,
      )
      .sort((a, b) => b.updated_at - a.updated_at)
  }
  getTask(id: string) {
    return (
      this.getTasks().find(
        (task) =>
          task.task_id === id &&
          ['app.install/v1', 'app.update/v1'].includes(task.schema_id),
      ) ?? null
    )
  }
  taskReadError(id: string) {
    const task = this.tasks[id]
    return id === 't-forbidden' ||
      (task &&
        this.scope.role !== 'admin' &&
        task.owner_user_id !== this.scope.user_id)
      ? 'TASK_FORBIDDEN'
      : task && !['app.install/v1', 'app.update/v1'].includes(task.schema_id)
        ? 'TASK_TYPE_UNSUPPORTED'
        : 'TASK_NOT_FOUND'
  }
  getDraft(id: string) {
    return this.drafts[id] ?? null
  }

  async analyzeInstallSource(
    input: string | PickedPikgFile,
    onProgress?: (state: 'preparing' | 'importing', percent: number) => void,
    signal?: AbortSignal,
  ): Promise<SourceParseResult> {
    const scope = scopeStorageKey(this.scope)
    const canceled = () =>
      signal?.aborted || scope !== scopeStorageKey(this.scope)
    onProgress?.('preparing', 0)
    await delay(160)
    if (canceled()) return { ok: false, code: 'IMPORT_CANCELED' }
    let kind: SourceReference['kind'] = 'identifier'
    let name: string
    let fixture: string | null = null
    let scenario: MockScenario = 'normal'
    let size: number | null = null
    let content = false
    if (typeof input !== 'string') {
      kind = input.location === 'device' ? 'local-pikg' : 'personal-server-pikg'
      name = input.name.replace(/[\\/]/g, '_').slice(0, 255)
      size = input.sizeBytes
      if (input.file) {
        if (input.file.size > 65536)
          return { ok: false, code: 'PIKG_BRIDGE_REQUIRED' }
        try {
          const envelope = JSON.parse(await input.file.text())
          if (
            envelope.format !== 'buckyos-pikg-mock-v1' ||
            typeof envelope.app !== 'string' ||
            !fixtureFor(envelope.app) ||
            (envelope.scenario && !scenarios.includes(envelope.scenario))
          )
            return { ok: false, code: 'INVALID_PIKG' }
          fixture = fixtureFor(envelope.app)
          scenario = envelope.scenario ?? 'normal'
          content = envelope.content_available === true
        } catch {
          return { ok: false, code: 'INVALID_PIKG' }
        }
      } else {
        fixture = input.fixture ? fixtureFor(input.fixture) : null
        content = Boolean(fixture)
      }
      if (!fixture || size === 0) return { ok: false, code: 'INVALID_PIKG' }
    } else {
      const value = input.trim()
      if (!value || value.length > 32768)
        return { ok: false, code: 'INVALID_SOURCE' }
      if (value.startsWith('{')) {
        try {
          const parsed = appDocCandidateSchema.safeParse(JSON.parse(value))
          if (!parsed.success) return { ok: false, code: 'INVALID_APPDOC' }
        } catch {
          return { ok: false, code: 'INVALID_APPDOC' }
        }
        return { ok: false, code: 'APPDOC_IMPORT_REQUIRED' }
      }
      if (/^https?:/i.test(value)) {
        try {
          const url = new URL(value)
          if (url.username || url.password)
            return { ok: false, code: 'INVALID_URL' }
        } catch {
          return { ok: false, code: 'INVALID_URL' }
        }
        return { ok: false, code: 'URL_IMPORT_REQUIRED' }
      }
      if (/^appdoc:[0-9a-f]{64}$/.test(value))
        return { ok: false, code: 'OBJECT_NOT_LOCAL' }
      if (
        value.startsWith('did:') &&
        !hostnameDidSchema.safeParse(value).success
      )
        return { ok: false, code: 'INVALID_DID' }
      fixture = fixtureFor(value)
      if (!fixture) return { ok: false, code: 'SOURCE_UNAVAILABLE' }
      scenario =
        scenarios.find((s) => s !== 'normal' && value.endsWith(`-${s}`)) ??
        'normal'
      name = value
      content = false
    }
    for (const progress of [25, 65, 100]) {
      onProgress?.('importing', progress)
      await delay(100)
      if (canceled()) return { ok: false, code: 'IMPORT_CANCELED' }
    }
    if (scenario === 'import-fail') return { ok: false, code: 'IMPORT_FAILED' }
    return {
      ok: true,
      source: {
        reference_id: uid('source'),
        kind,
        display_name: name,
        size_bytes: size,
        fixture: fixture!,
        scenario,
        content_available: content,
        state: 'ready',
        expires_at:
          kind === 'identifier'
            ? null
            : Date.now() + (scenario === 'expired-reference' ? 0 : 1_800_000),
      },
    }
  }
  createDraft(
    source: SourceReference,
    suggestions?: {
      target_node_id?: string
      offline?: boolean
      auto_start?: boolean
    },
  ) {
    const app = appDocument(source.fixture, source.scenario)
    const input = defaultInput(app)
    if (suggestions?.target_node_id)
      input.target_node_id = suggestions.target_node_id
    if (suggestions?.offline !== undefined) input.offline = suggestions.offline
    if (suggestions?.auto_start !== undefined)
      input.install_params.auto_start = suggestions.auto_start
    const draft_id = uid('draft')
    const draft: InspectionDraft = {
      draft_id,
      source: clone(source),
      app,
      input,
      status: 'dirty',
      plan: null,
      readiness: null,
      inspection_revision: 0,
      error: null,
      suggestions: Boolean(suggestions),
    }
    this.drafts[draft_id] = draft
    this.persist()
    this.emit()
    return draft_id
  }
  releaseDraft(id: string) {
    const draft = this.getDraft(id)
    if (!draft) return
    draft.source.state = 'released'
    draft.inspection_revision++
    this.revokeApproval(draft.plan?.plan_fingerprint)
    delete this.drafts[id]
    this.persist()
    this.emit()
  }
  editDraft(id: string, input: InstallInput) {
    const draft = this.getDraft(id)
    if (!draft || draft.status === 'submitting') return
    this.revokeApproval(draft.plan?.plan_fingerprint)
    draft.input = clone(input)
    draft.status = 'dirty'
    draft.plan = null
    draft.error = null
    draft.inspection_revision++
    this.persist()
    this.emit()
  }
  private revokeApproval(fp?: string) {
    for (const [key, value] of this.grants)
      if (!fp || value.fingerprint === fp) this.grants.delete(key)
  }
  async inspectDraft(id: string) {
    const draft = this.getDraft(id)
    if (!draft || draft.status === 'submitting') return
    const revision = ++draft.inspection_revision
    this.revokeApproval(draft.plan?.plan_fingerprint)
    draft.status = 'inspecting'
    draft.plan = null
    draft.error = null
    this.emit()
    await delay(460)
    if (this.getDraft(id) !== draft || revision !== draft.inspection_revision)
      return
    const source = draft.source
    const scenario = source.scenario
    const input = clone(draft.input)
    const app = draft.app
    const local =
      source.kind === 'local-pikg' || source.kind === 'personal-server-pikg'
    const target =
      this.targets.find((n) => n.node_id === input.target_node_id) ??
      this.targets[0]
    const readiness: PlanReadiness = {
      document: 'READY',
      signature: 'READY',
      owner: 'READY',
      authority: 'READY',
      content:
        source.content_available && scenario !== 'content-missing'
          ? 'READY'
          : 'NOT_READY',
      target: target.online ? 'READY' : 'UNKNOWN',
      config: 'READY',
      document_status: 'Active',
      local_developer_authority: false,
      install: 'OFFLINE_READY',
      issues: [],
    }
    const block = (
      field: string,
      code: string,
      action: 'recheck' | 'change-source' | 'edit' = 'change-source',
    ) => readiness.issues.push({ field, code, action })
    if (
      source.state !== 'ready' ||
      (source.expires_at !== null && source.expires_at <= Date.now())
    ) {
      source.state = 'expired'
      block('source', 'REFERENCE_EXPIRED')
    }
    if (scenario === 'developer') {
      readiness.document_status = 'Missing'
      readiness.signature = 'UNKNOWN'
      readiness.authority = 'NOT_READY'
      readiness.owner = 'UNKNOWN'
    }
    if (scenario === 'trust-pending') {
      readiness.document_status = 'Unknown'
      readiness.authority = 'UNKNOWN'
      block('authority', 'TRUST_RESOLUTION_REQUIRED', 'recheck')
    }
    for (const [value, status] of [
      ['revoked', 'Revoked'],
      ['tombstoned', 'Tombstoned'],
      ['migrated', 'Migrated'],
    ] as const)
      if (scenario === value) {
        readiness.document_status = status
        readiness.authority = 'NOT_READY'
        block('authority', `IDENTITY_${value.toUpperCase()}`)
      }
    for (const [value, field, code] of [
      ['document-fail', 'document', 'INVALID_APPDOC'],
      ['signature-fail', 'signature', 'SIGNATURE_INVALID'],
      ['owner-fail', 'owner', 'OWNER_MISMATCH'],
    ] as const)
      if (scenario === value) {
        readiness[field] = 'NOT_READY'
        block(field, code)
      }
    if (
      input.policy === 'LOCAL_DEVELOPER' &&
      local &&
      ['Active', 'Missing', 'Expired', 'Unknown'].includes(
        readiness.document_status,
      ) &&
      readiness.document === 'READY' &&
      !['signature-fail', 'owner-fail'].includes(scenario)
    ) {
      readiness.local_developer_authority = true
      readiness.issues = readiness.issues.filter(
        (issue) => issue.code !== 'TRUST_RESOLUTION_REQUIRED',
      )
    } else if (scenario === 'developer')
      block('authority', 'LOCAL_DEVELOPER_REQUIRED', 'edit')
    if (scenario === 'unsupported' || target.os !== 'linux') {
      readiness.target = 'NOT_READY'
      block('target_node_id', 'UNSUPPORTED_TARGET', 'edit')
    } else if (!target.online) block('target_node_id', 'TARGET_OFFLINE', 'edit')
    if (input.offline && readiness.content !== 'READY')
      block('content', 'OFFLINE_CONTENT_UNAVAILABLE', 'edit')
    const valid = createInstallInputSchema(app, this.targets, local).safeParse(
      input,
    )
    if (!valid.success) {
      readiness.config = 'NOT_READY'
      valid.error.issues.forEach((issue) =>
        block(
          issue.path.join('.'),
          issue.code === 'custom' ? issue.message : 'INVALID_FIELD',
          'edit',
        ),
      )
    }
    if (scenario === 'config-invalid') {
      readiness.config = 'NOT_READY'
      block('install_params.bash_envs.LANG', 'CONFIG_CONFLICT', 'edit')
    }
    const instance = `${app.app_id}@${this.scope.user_id}`
    const installed = this.getById(instance)?.record
    const versionParts = (version: string) => version.split('.').map(Number)
    const older =
      installed &&
      versionParts(app.version).some(
        (n, i, parts) =>
          parts
            .slice(0, i)
            .every((p, j) => p === versionParts(installed.version)[j]) &&
          n < (versionParts(installed.version)[i] ?? 0),
      )
    if (older) block('version', 'DOWNGRADE_NOT_SUPPORTED')
    const planUse =
      installed?.app_doc_object_id === app.object_id
        ? 'SATISFIED'
        : installed
          ? 'UPGRADE'
          : 'FRESH_INSTALL'
    readiness.install = readiness.issues.length
      ? 'BLOCKED'
      : readiness.content === 'READY'
        ? 'OFFLINE_READY'
        : 'CONTENT_DOWNLOAD_REQUIRED'
    const material = {
      schema_version: 4 as const,
      app_instance_id: instance,
      owner_user_id: this.scope.user_id,
      app: clone(app),
      target: clone(target),
      input,
      plan_use: planUse as InstallPlan['plan_use'],
      previous_version: installed?.version ?? null,
      previous_generation: installed?.deployment?.spec_generation ?? 0,
      download_bytes:
        readiness.content === 'READY'
          ? 0
          : 83_886_080 +
            (input.install_params.selected_components.includes('worker')
              ? 20_971_520
              : 0),
      source,
      readiness,
      inspection_revision: revision,
    }
    const fp = await fingerprint(material)
    if (this.getDraft(id) !== draft || revision !== draft.inspection_revision)
      return
    draft.plan = freezePlan({
      schema_version: 4,
      app_instance_id: instance,
      owner_user_id: this.scope.user_id,
      app: material.app,
      target: material.target,
      input,
      plan_use: material.plan_use,
      previous_version: material.previous_version,
      previous_generation: material.previous_generation,
      download_bytes: material.download_bytes,
      plan_fingerprint: fp,
      created_at: Date.now(),
      expires_at: Date.now() + 180_000,
    })
    draft.readiness = readiness
    draft.status = readiness.issues.length ? 'blocked' : 'ready'
    this.persist()
    this.emit()
  }
  authorize = async (
    params: SudoByPasswordParams,
    draftId: string,
  ): Promise<SudoGrant> => {
    await delay(260)
    const draft = this.getDraft(draftId)
    if (this.scope.role !== 'admin' || params.username !== this.scope.user_id)
      throw new Error('ADMIN_REQUIRED')
    if (!['prototype-admin', 'prototype-expired'].includes(params.password))
      throw new Error('INCORRECT_PASSWORD')
    if (!draft?.plan || draft.status !== 'ready') throw new Error('PLAN_STALE')
    const sessionToken = uid('sudo')
    const expiresAtMs =
      Date.now() + (params.password === 'prototype-expired' ? -1 : 180_000)
    this.grants.set(sessionToken, {
      fingerprint: draft.plan.plan_fingerprint,
      expires: expiresAtMs,
      scope: scopeStorageKey(this.scope),
    })
    return {
      sessionToken,
      expiresAtMs,
      expiresInSeconds: Math.max(0, (expiresAtMs - Date.now()) / 1000),
      username: params.username,
      appid: params.appid,
    }
  }
  async submitDraft(
    id: string,
    fp: string,
    key: string,
    grant: SudoGrant,
  ): Promise<SubmitResult> {
    const pendingKey = `${scopeStorageKey(this.scope)}:${key}:${fp}`
    const pending = this.pendingSubmissions.get(pendingKey)
    if (pending) return pending
    const operation = this.submitChecked(id, fp, key, grant)
    this.pendingSubmissions.set(pendingKey, operation)
    try {
      return await operation
    } finally {
      this.pendingSubmissions.delete(pendingKey)
    }
  }
  private async submitChecked(
    id: string,
    fp: string,
    key: string,
    grant: SudoGrant,
  ): Promise<SubmitResult> {
    const previous = this.idempotency[key]
    if (previous) {
      if (previous.fingerprint !== fp) throw new Error('PLAN_STALE')
      return clone(previous.result)
    }
    const draft = this.getDraft(id)
    const authorization = this.grants.get(grant.sessionToken)
    if (
      !authorization ||
      authorization.expires <= Date.now() ||
      authorization.scope !== scopeStorageKey(this.scope) ||
      authorization.fingerprint !== fp
    )
      throw new Error('AUTH_EXPIRED')
    if (
      !draft?.plan ||
      draft.status !== 'ready' ||
      draft.plan.plan_fingerprint !== fp
    )
      throw new Error('PLAN_STALE')
    const plan = draft.plan
    const installed = this.getById(plan.app_instance_id)?.record
    if (
      plan.expires_at <= Date.now() ||
      (installed?.deployment?.spec_generation ?? 0) !==
        plan.previous_generation ||
      (draft.source.scenario === 'stale' && !this.staleSeen.has(id))
    ) {
      this.staleSeen.add(id)
      draft.status = 'stale'
      draft.error = 'PLAN_STALE'
      this.revokeApproval(fp)
      this.emit()
      throw new Error('PLAN_STALE')
    }
    if (
      draft.source.expires_at !== null &&
      draft.source.expires_at <= Date.now()
    ) {
      draft.status = 'blocked'
      draft.error = 'REFERENCE_EXPIRED'
      this.revokeApproval(fp)
      this.emit()
      throw new Error('REFERENCE_EXPIRED')
    }
    draft.status = 'submitting'
    this.emit()
    await delay(320)
    if (
      this.getDraft(id) !== draft ||
      authorization.scope !== scopeStorageKey(this.scope) ||
      authorization.expires <= Date.now()
    ) {
      if (this.getDraft(id) === draft) {
        draft.status = 'ready'
        this.emit()
      }
      throw new Error('AUTH_EXPIRED')
    }
    const concurrent = this.idempotency[key]
    if (concurrent) return clone(concurrent.result)
    let result: SubmitResult
    if (plan.plan_use === 'SATISFIED')
      result = {
        action: 'satisfied',
        task_id: null,
        app_instance_id: plan.app_instance_id,
      }
    else result = this.createTask(plan, draft.readiness!, draft.source, null)
    this.idempotency[key] = { fingerprint: fp, result }
    this.revokeApproval(fp)
    delete this.drafts[id]
    this.persist()
    this.emit()
    return result
  }
  private createTask(
    plan: InstallPlan,
    readiness: PlanReadiness,
    source: SourceReference,
    retryOf: string | null,
  ): SubmitResult & { action: 'submitted' } {
    const task_id = uid('t')
    const task: InstallTask = {
      task_id,
      schema_id:
        plan.plan_use === 'UPGRADE' ? 'app.update/v1' : 'app.install/v1',
      app_instance_id: plan.app_instance_id,
      owner_user_id: plan.owner_user_id,
      app: clone(plan.app),
      plan: safePlan(plan),
      readiness: clone(readiness),
      source: clone(source),
      phase: 'Running',
      outcome: null,
      stage: plan.download_bytes ? 'acquire' : 'verify',
      progress: null,
      desired_state_committed: false,
      available_actions: ['cancel'],
      retry_of: retryOf,
      error: null,
      result: null,
      tick: 0,
      updated_at: Date.now(),
    }
    this.tasks[task_id] = task
    this.ensureService(task)
    this.runTask(task_id)
    return { action: 'submitted', task_id, retry_of: retryOf }
  }
  cancelTask(id: string) {
    const task = this.getTask(id)
    if (
      !task ||
      task.desired_state_committed ||
      !task.available_actions.includes('cancel')
    )
      return false
    task.phase = 'Terminal'
    task.outcome = 'Canceled'
    task.available_actions = []
    task.updated_at = Date.now()
    const service = this.getById(task.app_instance_id)
    if (service && !service.record)
      this.services = this.services.filter((s) => s !== service)
    this.persist()
    this.emit()
    return true
  }
  retryTask(id: string): string | null {
    const task = this.getTask(id)
    if (
      !task ||
      !task.error?.retryable ||
      !task.available_actions.includes('retry')
    )
      return null
    if (task.desired_state_committed) {
      task.phase = 'Running'
      task.outcome = null
      task.error = null
      task.available_actions = []
      task.tick = 5
      this.runTask(id)
      this.persist()
      this.emit()
      return id
    }
    const existing = this.getTasks().find(
      (t) => t.retry_of === id && t.outcome !== 'Canceled',
    )
    if (existing) return existing.task_id
    const result = this.createTask(task.plan, task.readiness, task.source, id)
    this.persist()
    this.emit()
    return result.task_id
  }
  inspectTask(id: string): string | null {
    const task = this.getTask(id)
    if (
      !task ||
      !task.available_actions.includes('inspect') ||
      task.desired_state_committed
    )
      return null
    const draftId = this.createDraft(task.source)
    this.editDraft(draftId, safeInput(task.plan.input, task.app))
    return draftId
  }
  resumeTask(id: string): string | null {
    const task = this.getTask(id)
    if (
      !task ||
      task.phase !== 'Waiting' ||
      !task.available_actions.includes('resume')
    )
      return null
    task.phase = 'Running'
    task.available_actions = ['cancel']
    task.tick++
    this.runTask(id)
    this.persist()
    this.emit()
    return id
  }
  private runTask(id: string) {
    if (this.running.has(id)) return
    this.running.add(id)
    const advance = () => {
      const task = this.getTask(id)
      if (!task || task.phase !== 'Running') {
        this.running.delete(id)
        return
      }
      const scenario = task.source.scenario
      if (task.tick === 0 && scenario === 'fail-download' && !task.retry_of) {
        task.phase = 'Terminal'
        task.outcome = 'Failed'
        task.error = { code: 'DOWNLOAD_FAILED', retryable: true }
        task.available_actions = ['retry', 'change-source']
        this.running.delete(id)
        const service = this.getById(task.app_instance_id)
        if (service && !service.record) {
          service.status = service.runtime.status = 'error'
          service.runtime.reason = 'DOWNLOAD_FAILED'
        }
      } else if (task.tick === 1 && scenario === 'paused') {
        task.phase = 'Waiting'
        task.available_actions = ['resume', 'cancel']
        this.running.delete(id)
      } else if (
        task.tick === 4 &&
        scenario === 'fail-committed' &&
        !task.error
      ) {
        task.phase = 'Terminal'
        task.outcome = 'Failed'
        task.error = { code: 'SCHEDULING_FAILED', retryable: true }
        task.available_actions = ['retry']
        task.tick = 5
        this.running.delete(id)
      } else if (task.tick >= 5) {
        task.phase = 'Terminal'
        task.outcome = 'Succeeded'
        task.available_actions = []
        this.running.delete(id)
        const service = this.getById(task.app_instance_id)!
        service.record!.state = 'installed'
        task.result = clone(service.record)
        service.status = service.runtime.status = 'deploying'
        service.available_actions = []
        this.resumeRuntime(task)
      } else {
        task.stage = (
          ['acquire', 'verify', 'prepare', 'deploy', 'activate'] as const
        )[task.tick]
        if (task.stage === 'acquire' && !task.plan.download_bytes)
          task.stage = 'verify'
        if (task.tick === 3 && !this.commitDesiredState(task))
          this.running.delete(id)
        else {
          task.tick++
          this.schedule(advance, 650)
        }
      }
      task.updated_at = Date.now()
      this.persist()
      this.emit()
    }
    this.schedule(advance, 650)
  }
  private ensureService(task: InstallTask) {
    let service = this.getById(task.app_instance_id)
    if (service) {
      service.installTaskId = task.task_id
      if (!service.record)
        service.status = service.runtime.status = 'installing'
      return
    }
    const runtime = emptyRuntime(task.app.runtime_type, 'installing')
    service = {
      id: task.app_instance_id,
      app_instance_id: task.app_instance_id,
      system_service_id: null,
      app_did: task.app.did,
      owner_user_id: task.owner_user_id,
      name: task.app.show_name,
      description: task.app.description_key,
      iconKey: task.app.iconKey,
      version: task.app.version,
      layer: 'app',
      status: 'installing',
      runtime,
      record: null,
      docker: null,
      diagnostics: [],
      spec: {},
      settings: {},
      serviceInfo: { node: task.plan.target.node_id },
      logs: [],
      installTaskId: task.task_id,
      available_actions: [],
      operation: null,
      operation_error: null,
    }
    this.services.unshift(service)
  }
  private commitDesiredState(task: InstallTask) {
    const service = this.getById(task.app_instance_id)!
    if (
      (service.record?.deployment?.spec_generation ?? 0) !==
      task.plan.previous_generation
    ) {
      task.phase = 'Terminal'
      task.outcome = 'Failed'
      task.error = { code: 'PLAN_STALE', retryable: false }
      task.available_actions = ['inspect']
      return false
    }
    task.desired_state_committed = true
    task.available_actions = []
    service.installTaskId = task.task_id
    const deployment = {
      app_instance_id: task.app_instance_id,
      task_id: task.task_id,
      app_doc_object_id: task.app.object_id,
      spec_generation: task.plan.previous_generation + 1,
    }
    const host =
      service.record?.app_host_name ??
      `${task.source.fixture}-${task.owner_user_id}`
    service.record = {
      app_instance_id: task.app_instance_id,
      app_did: task.app.did,
      owner_user_id: task.owner_user_id,
      app_doc_object_id: task.app.object_id,
      task_id: task.task_id,
      version: task.app.version,
      state: 'configuration_submitted',
      deployment,
      app_name: task.source.fixture,
      app_host_name: host,
      app_index: service.record?.app_index ?? 1,
      address: task.app.endpoints.some((e) => e.name === 'www')
        ? `https://${host}.alice.buckyos.io`
        : null,
    }
    service.version = task.app.version
    service.status = 'deploying'
    service.runtime = emptyRuntime(task.app.runtime_type, 'deploying')
    service.runtime.expected_deployment = deployment
    service.spec = {
      auto_start: String(task.plan.input.install_params.auto_start),
      expected_instance_count: '1',
    }
    task.result = clone(service.record)
    return true
  }
  private resumeRuntime(task: InstallTask) {
    const service = this.getById(task.app_instance_id)
    if (
      !service ||
      service.installTaskId !== task.task_id ||
      (runtimeEvidenceValid(service.runtime) &&
        !['starting', 'deploying'].includes(service.status))
    )
      return
    this.schedule(() => {
      if (
        service.installTaskId !== task.task_id ||
        [
          'runtime-unknown',
          'offline-node',
          'runtime-timeout',
          'stale-evidence',
        ].includes(task.source.scenario)
      )
        return
      service.runtime.evidence = {
        deployment: service.runtime.expected_deployment!,
        observed_at: Date.now(),
        valid_until: Date.now() + 300_000,
      }
      service.status = service.runtime.status = task.plan.input.install_params
        .auto_start
        ? 'starting'
        : 'stopped'
      service.available_actions = task.plan.input.install_params.auto_start
        ? []
        : ['start']
      this.persist()
      this.emit()
    }, 850)
    this.schedule(() => {
      if (service.installTaskId !== task.task_id) return
      const scenario = task.source.scenario
      const unknown = [
        'runtime-unknown',
        'offline-node',
        'runtime-timeout',
        'stale-evidence',
      ].includes(scenario)
      if (!task.plan.input.install_params.auto_start && !unknown) return
      service.runtime.evidence =
        unknown && scenario !== 'stale-evidence'
          ? null
          : {
              deployment: {
                ...service.runtime.expected_deployment!,
                ...(scenario === 'stale-evidence'
                  ? { spec_generation: 0 }
                  : {}),
              },
              observed_at: Date.now(),
              valid_until: Date.now() + 300_000,
            }
      service.status = service.runtime.status = unknown
        ? 'unknown'
        : scenario === 'activation-fail'
          ? 'activation_failed'
          : 'running'
      service.runtime.reason = unknown
        ? scenario.toUpperCase().replaceAll('-', '_')
        : scenario === 'activation-fail'
          ? 'STARTUP_FAILED'
          : null
      const evidence = runtimeEvidenceValid(service.runtime)
      if (['docker', 'agent'].includes(service.runtime.type) && evidence)
        service.runtime.docker = {
          engine: 'running',
          image: 'present',
          imageName: `${task.source.fixture}:${task.app.version}`,
          container: service.status === 'running' ? 'running' : 'error',
        }
      service.docker = service.runtime.docker
      service.runtime.process_health =
        evidence && service.runtime.type === 'script'
          ? service.status === 'running'
            ? 'READY'
            : 'NOT_READY'
          : 'UNKNOWN'
      service.runtime.web_accessible =
        evidence && service.runtime.type === 'web'
          ? service.status === 'running'
            ? 'READY'
            : 'NOT_READY'
          : 'UNKNOWN'
      service.runtime.environment_ready =
        evidence && service.runtime.type === 'agent'
          ? service.status === 'running'
            ? 'READY'
            : 'NOT_READY'
          : 'UNKNOWN'
      service.available_actions = unknown
        ? []
        : service.status === 'running'
          ? ['stop']
          : ['start']
      this.persist()
      this.emit()
    }, 2000)
  }
  startService(id: string) {
    this.operateService(id, 'start')
  }
  stopService(id: string) {
    this.operateService(id, 'stop')
  }
  private operateService(id: string, action: 'start' | 'stop') {
    const service = this.getById(id)
    if (
      !service ||
      !runtimeEvidenceValid(service.runtime) ||
      this.scope.role !== 'admin' ||
      service.operation ||
      !service.available_actions.includes(action)
    )
      return
    service.operation = action
    service.operation_error = null
    service.status = service.runtime.status =
      action === 'start' ? 'starting' : 'stopping'
    this.emit()
    this.schedule(() => {
      const scenario = new URLSearchParams(location.search).get(
        'appServiceScenario',
      )
      if (scenario === 'operation-fail' || scenario === 'operation-timeout') {
        service.operation_error =
          scenario === 'operation-fail'
            ? 'OPERATION_FAILED'
            : 'OPERATION_TIMEOUT'
        service.status = service.runtime.status = 'unknown'
        service.available_actions = []
      } else {
        service.status = service.runtime.status =
          action === 'start' ? 'running' : 'stopped'
        service.available_actions = action === 'start' ? ['stop'] : ['start']
        if (service.runtime.expected_deployment)
          service.runtime.evidence = {
            deployment: service.runtime.expected_deployment,
            observed_at: Date.now(),
            valid_until: Date.now() + 300_000,
          }
        if (service.docker)
          service.docker.container = action === 'start' ? 'running' : 'stopped'
      }
      service.operation = null
      this.persist()
      this.emit()
    }, 1100)
  }
  safeDiagnostics(task: InstallTask) {
    return JSON.stringify(
      {
        task_id: task.task_id,
        app_instance_id: task.app_instance_id,
        stage: task.stage,
        phase: task.phase,
        outcome: task.outcome,
        code: task.error?.code,
        plan_fingerprint: task.plan.plan_fingerprint,
        retry_of: task.retry_of,
      },
      null,
      2,
    )
  }
  private persist() {
    const drafts = Object.fromEntries(
      Object.values(this.drafts).map((draft) => [
        draft.draft_id,
        {
          ...draft,
          input: safeInput(draft.input, draft.app),
          plan: null,
          readiness: null,
          status: 'dirty',
        },
      ]),
    )
    const tasks = Object.fromEntries(
      Object.values(this.tasks).map((task) => [
        task.task_id,
        { ...task, plan: safePlan(task.plan) },
      ]),
    )
    localStorage.setItem(
      scopeStorageKey(this.scope),
      JSON.stringify({
        model_version: MODEL_VERSION,
        scope: scopeStorageKey(this.scope),
        services: this.services,
        tasks,
        drafts,
        idempotency: this.idempotency,
      }),
    )
  }
  private schedule(callback: () => void, ms: number) {
    const timer = window.setTimeout(() => {
      this.timers.delete(timer)
      callback()
    }, ms)
    this.timers.add(timer)
  }
  private emit() {
    this.revision++
    this.listeners.forEach((listener) => listener())
  }
}
