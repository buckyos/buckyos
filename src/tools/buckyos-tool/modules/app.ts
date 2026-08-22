import { isAbsolute, resolve } from 'node:path'
import { namelib, ndm_proxy, ndn } from 'buckyos'
import type { CommandDefinition, CommandModule, JsonSchema } from '../core/command.ts'
import type { CommandContext } from '../core/context.ts'
import {
  EXIT_AUTH,
  EXIT_INTERNAL,
  EXIT_OPERATION,
  EXIT_PERMISSION,
  EXIT_TIMEOUT,
  EXIT_UNAVAILABLE,
  ToolError,
  UsageError,
} from '../core/errors.ts'
import { type TaskObservation, waitForTask } from '../core/task.ts'

const CONTROL_PANEL_SERVICE = 'control-panel'
const PLAN_SCHEMA_VERSION = 4
const ZIP_LOCAL_MAGIC = [0x50, 0x4b, 0x03, 0x04] as const
const OBJECT_OUTPUT: JsonSchema = { type: 'object', additionalProperties: true }
const INSTALL_POLICIES = [
  'strict-public',
  'normal',
  'trusted-share',
  'local-developer',
  'system-internal',
] as const
const APP_CLASSES = ['user_installed', 'zone_installed'] as const

type PikgPurpose = 'inspect' | 'install'
type SourceKind = 'catalog' | 'pikg' | 'url'

export interface PikgSnapshot {
  kind: 'pikg' | 'url'
  display: string
  bytes: Uint8Array
  digest: string
  size: number
}

export interface PikgStagingMetadata {
  schema_version: number
  handle: string
  pikg_digest: string
  size: number
  purpose: PikgPurpose
  expires_at?: number
}

export interface AppModuleDependencies {
  download?: (url: URL, signal: AbortSignal) => Promise<Uint8Array>
  stagePikg?: (
    ctx: CommandContext,
    snapshot: PikgSnapshot,
    purpose: PikgPurpose,
  ) => Promise<PikgStagingMetadata>
  sleep?: (milliseconds: number, signal: AbortSignal) => Promise<void>
}

interface CatalogSource {
  kind: 'catalog'
  display: string
  serviceSource: { kind: 'identifier'; identifier: string }
}

interface StagedSource {
  kind: 'pikg' | 'url'
  display: string
  snapshot: PikgSnapshot
  staging: PikgStagingMetadata
  serviceSource: { kind: 'local_pikg'; staging_handle: string }
}

type PreparedSource = CatalogSource | StagedSource

interface InstallInspection {
  schema_version: number
  plan: Record<string, unknown>
  resolution_status: Record<string, unknown>
  status: Record<string, unknown>
}

export function createAppModule(dependencies: AppModuleDependencies = {}): CommandModule {
  const commands = [
    fetchCommand(dependencies),
    listCommand(),
    getCommand(),
    installCommand(dependencies),
    upgradeCommand(dependencies),
    uninstallCommand(dependencies),
    lifecycleCommand('start', 'Start an installed App'),
    lifecycleCommand('stop', 'Stop an installed App'),
    restartCommand(),
    statusCommand(),
  ]
  return {
    name: 'app',
    summary: 'Inspect, install, upgrade, and control Apps',
    commands: commands.map((command) => {
      const handler = command.handler
      return {
        ...command,
        handler: async (ctx, input) => sanitizeAppOutput(await handler(ctx, input)),
      }
    }),
  }
}

function fetchCommand(dependencies: AppModuleDependencies): CommandDefinition {
  return {
    verb: 'fetch',
    summary: 'Inspect an App source and optionally write a fresh-install plan',
    positionals: [
      {
        name: 'source',
        description: 'Catalog name/DID, local PIKG path, or HTTP(S) URL',
        required: false,
      },
    ],
    options: sourceOptions([
      { name: 'plan', description: 'Write the v4 InstallPlan to this JSON file', type: 'string' },
      appClassOption(),
      ownerOption(),
      policyOption(),
    ]),
    inputSchema: sourceInputSchema({
      plan: { type: 'string', minLength: 1 },
      app_class: { type: 'string', enum: [...APP_CLASSES] },
      owner_user_id: { type: 'string', minLength: 1 },
      policy: { type: 'string', enum: [...INSTALL_POLICIES] },
      target: { type: 'object', additionalProperties: true },
      install_params: { type: 'object', additionalProperties: true },
    }),
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'read' },
    asyncMode: 'sync',
    requiresSession: true,
    examples: [
      'buckyos app fetch did:bns:app1.alice',
      'buckyos app fetch ./demo-0.1.0.pikg --plan ./demo.install-plan.json',
    ],
    handler: async (ctx, input) => await fetchApp(ctx, input, dependencies),
  }
}

function listCommand(): CommandDefinition {
  return {
    verb: 'list',
    summary: 'List visible installed Apps',
    inputSchema: emptyInputSchema(),
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'read' },
    asyncMode: 'sync',
    requiresSession: true,
    examples: ['buckyos app list'],
    handler: async (ctx) => await callControl(ctx, 'apps.list', {}),
  }
}

function getCommand(): CommandDefinition {
  return selectorCommand({
    verb: 'get',
    summary: 'Get installation and runtime details for one App',
    access: 'read',
    asyncMode: 'sync',
    handler: async (ctx, input) => {
      const selector = normalizeAppSelector(expectString(input, 'app_name'))
      const [details, status] = await Promise.all([
        callControl(ctx, 'apps.details', { selector }),
        callControl(ctx, 'apps.status', { selector }),
      ])
      return { details, status }
    },
  })
}

function installCommand(dependencies: AppModuleDependencies): CommandDefinition {
  return {
    verb: 'install',
    summary: 'Install from a v4 plan or upgrade an existing App from a source',
    positionals: [
      {
        name: 'source',
        description: 'Catalog name/DID, local PIKG path, or HTTP(S) URL',
        required: false,
      },
    ],
    options: sourceOptions([
      { name: 'plan', description: 'Fresh-install v4 InstallPlan JSON file', type: 'string' },
      {
        name: 'dry-run',
        property: 'dry_run',
        description: 'Preflight and print without submitting',
        type: 'boolean',
      },
      {
        name: 'no-wait',
        property: 'no_wait',
        description: 'Return immediately after task creation',
        type: 'boolean',
      },
      policyOption(),
    ]),
    inputSchema: sourceInputSchema({
      plan: { type: 'string', minLength: 1 },
      dry_run: { type: 'boolean' },
      no_wait: { type: 'boolean' },
      policy: { type: 'string', enum: [...INSTALL_POLICIES] },
    }),
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'write' },
    asyncMode: 'either',
    requiresSession: true,
    examples: [
      'buckyos app install did:bns:app1.alice --plan ./app1.install-plan.json',
      'buckyos --yes app install https://example.com/app1-1.2.0.pikg',
    ],
    handler: async (ctx, input) => await installApp(ctx, input, dependencies),
  }
}

function upgradeCommand(dependencies: AppModuleDependencies): CommandDefinition {
  return {
    verb: 'upgrade',
    summary: 'Check and apply Catalog upgrades',
    positionals: [
      { name: 'app_name', description: 'Installed App name or DID; omit for all', required: false },
    ],
    options: [
      {
        name: 'dry-run',
        property: 'dry_run',
        description: 'Only show the upgrade preflight',
        type: 'boolean',
      },
      {
        name: 'no-wait',
        property: 'no_wait',
        description: 'Return immediately after task creation',
        type: 'boolean',
      },
      policyOption(),
    ],
    inputSchema: {
      type: 'object',
      properties: {
        app_name: { type: 'string', minLength: 1 },
        dry_run: { type: 'boolean' },
        no_wait: { type: 'boolean' },
        policy: { type: 'string', enum: [...INSTALL_POLICIES] },
      },
      additionalProperties: false,
    },
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'write' },
    asyncMode: 'either',
    requiresSession: true,
    examples: ['buckyos app upgrade', 'buckyos --yes app upgrade did:bns:app1.alice'],
    handler: async (ctx, input) => await upgradeApps(ctx, input, dependencies),
  }
}

function uninstallCommand(dependencies: AppModuleDependencies): CommandDefinition {
  return {
    ...selectorCommand({
      verb: 'uninstall',
      summary: 'Uninstall an App and explicitly retain or delete its managed data',
      access: 'destructive',
      asyncMode: 'task',
      handler: async (ctx, input) => await uninstallApp(ctx, input, dependencies),
    }),
    options: [
      {
        name: 'data',
        description: 'Managed data disposition',
        type: 'string',
        required: true,
        enum: ['retain', 'delete'],
      },
      {
        name: 'dry-run',
        property: 'dry_run',
        description: 'Only show the uninstall preflight',
        type: 'boolean',
      },
      {
        name: 'no-wait',
        property: 'no_wait',
        description: 'Return immediately after task creation',
        type: 'boolean',
      },
    ],
    inputSchema: {
      type: 'object',
      properties: {
        app_name: { type: 'string', minLength: 1 },
        data: { type: 'string', enum: ['retain', 'delete'] },
        dry_run: { type: 'boolean' },
        no_wait: { type: 'boolean' },
      },
      required: ['app_name', 'data'],
      additionalProperties: false,
    },
    examples: ['buckyos app uninstall app1 --data retain'],
  }
}

function lifecycleCommand(verb: 'start' | 'stop', summary: string): CommandDefinition {
  return {
    ...selectorCommand({
      verb,
      summary,
      access: 'write',
      asyncMode: verb === 'start' ? 'task' : 'either',
      handler: async (ctx, input) => await mutateLifecycle(ctx, input, verb),
    }),
    options: [
      {
        name: 'no-wait',
        property: 'no_wait',
        description: 'Return immediately after task creation',
        type: 'boolean',
      },
    ],
    inputSchema: {
      type: 'object',
      properties: {
        app_name: { type: 'string', minLength: 1 },
        no_wait: { type: 'boolean' },
      },
      required: ['app_name'],
      additionalProperties: false,
    },
  }
}

function restartCommand(): CommandDefinition {
  return {
    ...selectorCommand({
      verb: 'restart',
      summary: 'Recreate the runtime instances for an installed App',
      access: 'write',
      asyncMode: 'task',
      handler: async (ctx, input) => await mutateLifecycle(ctx, input, 'restart'),
    }),
    options: [
      {
        name: 'strategy',
        description: 'Restart strategy; rolling is currently unsupported',
        type: 'string',
        enum: ['recreate', 'rolling'],
      },
      {
        name: 'no-wait',
        property: 'no_wait',
        description: 'Return immediately after task creation',
        type: 'boolean',
      },
    ],
    inputSchema: {
      type: 'object',
      properties: {
        app_name: { type: 'string', minLength: 1 },
        strategy: { type: 'string', enum: ['recreate', 'rolling'] },
        no_wait: { type: 'boolean' },
      },
      required: ['app_name'],
      additionalProperties: false,
    },
    examples: ['buckyos app restart app1'],
  }
}

function statusCommand(): CommandDefinition {
  return {
    verb: 'status',
    summary: 'Show desired, task, scheduled, runtime, version, and readiness state',
    positionals: [
      { name: 'app_name', description: 'Installed App name or DID; omit for all', required: false },
    ],
    inputSchema: {
      type: 'object',
      properties: { app_name: { type: 'string', minLength: 1 } },
      additionalProperties: false,
    },
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'read' },
    asyncMode: 'sync',
    requiresSession: true,
    examples: ['buckyos app status', 'buckyos app status did:bns:app1.alice'],
    handler: async (ctx, input) => {
      if (typeof input.app_name === 'string') {
        const selector = normalizeAppSelector(input.app_name)
        const [status, details] = await Promise.all([
          callControl(ctx, 'apps.status', { selector }),
          callControl(ctx, 'apps.details', { selector }),
        ])
        const detailObject = expectObject(details, 'apps.details response')
        const summary = expectObject(detailObject.summary, 'apps.details summary')
        return {
          ...expectObject(status, 'apps.status response'),
          web_hosts: Array.isArray(summary.web_hosts) ? summary.web_hosts : [],
        }
      }
      const listed = expectObject(await callControl(ctx, 'apps.list', {}), 'apps.list response')
      const apps = Array.isArray(listed.apps) ? listed.apps : []
      const items = await Promise.all(apps.map(async (item) => {
        const app = expectObject(item, 'apps.list item')
        const selector = expectString(app, 'installation_id')
        return {
          ...expectObject(
            await callControl(ctx, 'apps.status', { selector }),
            'apps.status response',
          ),
          web_hosts: Array.isArray(app.web_hosts) ? app.web_hosts : [],
        }
      }))
      return { total: items.length, items }
    },
  }
}

async function fetchApp(
  ctx: CommandContext,
  input: Record<string, unknown>,
  dependencies: AppModuleDependencies,
): Promise<Record<string, unknown>> {
  const source = await prepareSource(ctx, input, 'inspect', dependencies)
  try {
    let inspection = await inspectSource(ctx, source, input)
    const planPathInput = optionalString(input.plan)
    let planPath: string | undefined
    if (planPathInput) {
      inspection = await finalizePlanChoices(ctx, source, input, inspection)
      const plan = inspection.plan
      assertPortablePlan(plan)
      planPath = resolveFromCwd(ctx, planPathInput)
      await writePlanFile(ctx, planPath, plan)
    }
    return {
      source: sourceSummary(source),
      app: inspection.plan.app,
      source_identity: inspection.plan.source_identity,
      resolution: inspection.resolution_status,
      status: inspection.status,
      plan_path: planPath ?? null,
      plan_fingerprint: planPath ? inspection.plan.plan_fingerprint : null,
    }
  } finally {
    await releaseStaging(ctx, source)
  }
}

async function installApp(
  ctx: CommandContext,
  input: Record<string, unknown>,
  dependencies: AppModuleDependencies,
): Promise<Record<string, unknown>> {
  rejectDryRunNoWait(input)
  const submittedPlan = optionalString(input.plan)
    ? await readPlanFile(resolveFromCwd(ctx, String(input.plan)))
    : undefined
  const source = await prepareSource(ctx, input, 'install', dependencies)
  let keepStaging = false
  try {
    const baseOverrides = submittedPlan ? planScopeAndOptions(submittedPlan) : {}
    let inspection = await inspectSource(ctx, source, input, baseOverrides)
    verifyPikgBinding(source, submittedPlan ?? inspection.plan)
    const app = expectObject(inspection.plan.app, 'inspection.plan.app')
    const appDid = expectString(app, 'did')
    const plannedOwner = submittedPlan
      ? expectString(
        expectObject(submittedPlan.installation_scope, 'plan.installation_scope'),
        'owner_user_id',
      )
      : undefined
    const installed = await findInstalled(ctx, appDid, plannedOwner)

    if (submittedPlan && installed) {
      throw new ToolError(
        'PLAN_NOT_APPLICABLE',
        'a fresh-install plan cannot be applied over an installed App',
      )
    }
    if (!submittedPlan && !installed) {
      throw new ToolError(
        'PLAN_REQUIRED',
        `App is not installed; run app fetch ${source.display} --plan <path> first`,
      )
    }

    if (submittedPlan) {
      assertPlanMatchesInspection(submittedPlan, inspection)
    } else {
      const scope = installedScope(installed!)
      inspection = await inspectSource(ctx, source, input, { ...scope, action: 'upgrade' })
      verifyPikgBinding(source, inspection.plan)
    }

    if (input.dry_run === true) {
      return {
        action: 'dry_run',
        source: sourceSummary(source),
        plan: inspection.plan,
        status: inspection.status,
      }
    }

    if (planUse(inspection.plan) === 'SATISFIED') {
      return satisfiedResult(inspection)
    }
    await confirmChange(ctx, 'install', confirmationSummary(inspection))
    const params = {
      ...sourceRpcParams(source),
      ...planScopeAndOptions(inspection.plan),
      options: installOptions(input),
      plan: submittedPlan ?? null,
      approved_plan_fingerprint: expectString(inspection.plan, 'plan_fingerprint'),
      idempotency_key: idempotencyKey(ctx),
    }
    const submitted = expectObject(
      await callControl(ctx, 'apps.submit', params),
      'apps.submit response',
    )
    if (submitted.action === 'satisfied' || submitted.task_id === null) {
      return submitted
    }
    const taskId = expectString(submitted, 'task_id')
    keepStaging = true
    if (input.no_wait === true) return submitted
    try {
      const status = await waitForTask(ctx, taskId, {
        observe: observeInstallTask,
        sleep: dependencies.sleep,
      })
      return { ...submitted, status }
    } finally {
      keepStaging = false
    }
  } finally {
    if (!keepStaging) await releaseStaging(ctx, source)
  }
}

async function upgradeApps(
  ctx: CommandContext,
  input: Record<string, unknown>,
  dependencies: AppModuleDependencies,
): Promise<Record<string, unknown>> {
  rejectDryRunNoWait(input)
  const selector = typeof input.app_name === 'string'
    ? normalizeAppSelector(input.app_name)
    : undefined
  const check = expectObject(
    await callControl(ctx, 'apps.upgrade.check', selector ? { selector } : {}),
    'apps.upgrade.check response',
  )
  const items = Array.isArray(check.items) ? check.items : []
  const actionable = items.filter((value) => {
    const item = isObject(value) ? value : {}
    return item.state === 'UPDATE_AVAILABLE' || item.state === 'PERMISSION_RECONFIRM_REQUIRED'
  })

  if (input.dry_run === true) return { action: 'dry_run', check }
  if (actionable.length === 0) return { action: 'satisfied', total: items.length, items }

  await confirmChange(ctx, 'upgrade', {
    total: items.length,
    update_count: actionable.length,
    items,
  })

  let submitted: Record<string, unknown>
  let installTask = false
  if (selector) {
    const item = expectObject(actionable[0], 'upgrade item')
    const appDid = expectString(item, 'app_did')
    const details = await findInstalled(ctx, appDid)
    if (!details) throw new ToolError('RESOURCE_NOT_FOUND', `App is not installed: ${appDid}`)
    const source: CatalogSource = {
      kind: 'catalog',
      display: appDid,
      serviceSource: { kind: 'identifier', identifier: appDid },
    }
    const inspection = await inspectSource(ctx, source, input, {
      ...installedScope(details),
      action: 'upgrade',
    })
    submitted = expectObject(
      await callControl(ctx, 'apps.submit', {
        ...sourceRpcParams(source),
        ...planScopeAndOptions(inspection.plan),
        options: installOptions(input),
        approved_plan_fingerprint: expectString(inspection.plan, 'plan_fingerprint'),
        idempotency_key: idempotencyKey(ctx),
      }),
      'apps.submit response',
    )
    installTask = submitted.task_id !== null
  } else {
    submitted = expectObject(
      await callControl(ctx, 'apps.upgrade', { idempotency_key: idempotencyKey(ctx) }),
      'apps.upgrade response',
    )
  }
  if (submitted.task_id === null || submitted.action === 'satisfied') return submitted
  const taskId = expectString(submitted, 'task_id')
  if (input.no_wait === true) return submitted
  const status = await waitForTask(ctx, taskId, {
    observe: installTask ? observeInstallTask : undefined,
    sleep: dependencies.sleep,
  })
  return { ...submitted, status }
}

async function uninstallApp(
  ctx: CommandContext,
  input: Record<string, unknown>,
  dependencies: AppModuleDependencies,
): Promise<Record<string, unknown>> {
  rejectDryRunNoWait(input)
  const selector = normalizeAppSelector(expectString(input, 'app_name'))
  const status = await callControl(ctx, 'apps.status', { selector })
  const preflight = { action: 'uninstall', data_disposition: input.data, status }
  if (input.dry_run === true) return { action: 'dry_run', preflight }
  await confirmChange(ctx, 'uninstall', preflight)
  const submitted = expectObject(
    await callControl(ctx, 'apps.uninstall', {
      selector,
      data_disposition: input.data,
      idempotency_key: idempotencyKey(ctx),
    }),
    'apps.uninstall response',
  )
  const taskId = expectString(submitted, 'task_id')
  if (input.no_wait === true) return submitted
  const task = await waitForTask(ctx, taskId, { sleep: dependencies.sleep })
  return { ...submitted, status: task }
}

async function mutateLifecycle(
  ctx: CommandContext,
  input: Record<string, unknown>,
  verb: 'start' | 'stop' | 'restart',
): Promise<Record<string, unknown>> {
  if (input.strategy === 'rolling') {
    throw new ToolError(
      'UNSUPPORTED_STRATEGY',
      'rolling restart is not supported by the current deployment strategy',
    )
  }
  const selector = normalizeAppSelector(expectString(input, 'app_name'))
  const submitted = expectObject(
    await callControl(ctx, `apps.${verb}`, {
      selector,
      idempotency_key: idempotencyKey(ctx),
      ...(verb === 'restart' ? { restart_strategy: input.strategy ?? 'recreate' } : {}),
    }),
    `apps.${verb} response`,
  )
  const taskId = expectString(submitted, 'task_id')
  if (input.no_wait === true) return submitted
  const task = await waitForTask(ctx, taskId)
  return { ...submitted, status: task }
}

async function prepareSource(
  ctx: CommandContext,
  input: Record<string, unknown>,
  purpose: PikgPurpose,
  dependencies: AppModuleDependencies,
): Promise<PreparedSource> {
  const classified = await classifySource(ctx, input)
  if (classified.kind === 'catalog') return classified

  const snapshot = classified.kind === 'url'
    ? await downloadPikg(classified.url, dependencies.download, ctx.signal)
    : await readPikg(classified.path, classified.kind)
  const stage = dependencies.stagePikg ?? stagePikg
  const metadata = await stage(ctx, snapshot, purpose)
  if (metadata.schema_version !== PLAN_SCHEMA_VERSION) {
    throw new ToolError('UNSUPPORTED_SCHEMA_VERSION', 'staging returned a non-v4 schema')
  }
  if (metadata.pikg_digest !== snapshot.digest || metadata.size !== snapshot.size) {
    throw new ToolError(
      'PIKG_DIGEST_MISMATCH',
      'the staged PIKG does not match the client byte snapshot',
      EXIT_OPERATION,
      false,
      {
        expected_digest: snapshot.digest,
        staged_digest: metadata.pikg_digest,
        expected_size: snapshot.size,
        staged_size: metadata.size,
      },
    )
  }
  if (metadata.purpose !== purpose) {
    throw new ToolError('INVALID_SERVICE_RESPONSE', 'staging purpose does not match the request', 9)
  }
  return {
    kind: snapshot.kind,
    display: snapshot.display,
    snapshot,
    staging: metadata,
    serviceSource: { kind: 'local_pikg', staging_handle: metadata.handle },
  }
}

async function classifySource(
  ctx: CommandContext,
  input: Record<string, unknown>,
): Promise<
  | CatalogSource
  | { kind: 'pikg'; path: string }
  | { kind: 'url'; url: URL }
> {
  const positional = optionalString(input.source)
  const pikg = optionalString(input.pikg)
  if (positional && pikg) {
    throw new UsageError('ARGUMENT_CONFLICT', '<source> and --pikg are mutually exclusive')
  }
  const raw = pikg ?? positional
  if (!raw) throw new UsageError('MISSING_ARGUMENT', 'source or --pikg is required')
  const forced = pikg ? 'pikg' : optionalString(input.from)
  if (pikg && forced && forced !== 'pikg') {
    throw new UsageError('ARGUMENT_CONFLICT', '--pikg conflicts with --from')
  }

  if (forced === 'url') return { kind: 'url', url: parsePikgUrl(raw) }
  if (forced === 'pikg') return { kind: 'pikg', path: resolveFromCwd(ctx, raw) }
  if (forced === 'catalog') {
    return {
      kind: 'catalog',
      display: normalizeCatalogIdentifier(raw),
      serviceSource: { kind: 'identifier', identifier: normalizeCatalogIdentifier(raw) },
    }
  }
  if (forced) throw new UsageError('INVALID_ARGUMENT', `invalid --from value: ${forced}`)

  if (/^https?:\/\//i.test(raw)) return { kind: 'url', url: parsePikgUrl(raw) }
  const candidatePath = resolveFromCwd(ctx, raw)
  const stat = await tryLstat(candidatePath)
  if (stat) {
    if (!stat.isFile) {
      throw new UsageError('INVALID_PIKG_SOURCE', `PIKG source is not a regular file: ${raw}`)
    }
    return { kind: 'pikg', path: candidatePath }
  }
  if (looksLikeLocalPath(raw)) {
    throw new UsageError('PIKG_NOT_FOUND', `PIKG source does not exist: ${raw}`)
  }
  const identifier = normalizeCatalogIdentifier(raw)
  return {
    kind: 'catalog',
    display: identifier,
    serviceSource: { kind: 'identifier', identifier },
  }
}

async function readPikg(path: string, kind: 'pikg'): Promise<PikgSnapshot> {
  let file: Deno.FsFile
  try {
    file = await Deno.open(path, { read: true })
  } catch (error) {
    throw new UsageError('PIKG_READ_FAILED', `failed to open PIKG: ${errorMessage(error)}`)
  }
  try {
    const stat = await file.stat()
    if (!stat.isFile) throw new UsageError('INVALID_PIKG_SOURCE', 'PIKG source is not a file')
    if (stat.size > Number.MAX_SAFE_INTEGER) {
      throw new UsageError('PIKG_TOO_LARGE', 'PIKG is too large for this client')
    }
    const bytes = new Uint8Array(stat.size)
    let offset = 0
    while (offset < bytes.length) {
      const read = await file.read(bytes.subarray(offset))
      if (read === null) break
      offset += read
    }
    if (offset !== bytes.length) {
      throw new UsageError('PIKG_READ_FAILED', 'PIKG changed size while being read')
    }
    validatePikgSnapshot(bytes)
    return {
      kind,
      display: path,
      bytes,
      digest: await sha256Id(bytes),
      size: bytes.length,
    }
  } finally {
    file.close()
  }
}

async function downloadPikg(
  url: URL,
  downloader: AppModuleDependencies['download'],
  signal: AbortSignal,
): Promise<PikgSnapshot> {
  let bytes: Uint8Array
  try {
    bytes = downloader ? await downloader(url, signal) : await defaultDownload(url, signal)
  } catch (error) {
    if (error instanceof ToolError) throw error
    throw new ToolError(
      'PIKG_DOWNLOAD_FAILED',
      `failed to download PIKG: ${errorMessage(error)}`,
      5,
      true,
    )
  }
  validatePikgSnapshot(bytes)
  return {
    kind: 'url',
    display: safeUrl(url),
    bytes,
    digest: await sha256Id(bytes),
    size: bytes.length,
  }
}

async function defaultDownload(url: URL, signal: AbortSignal): Promise<Uint8Array> {
  const response = await fetch(url, { method: 'GET', redirect: 'follow', signal })
  if (!response.ok) {
    throw new ToolError(
      'PIKG_DOWNLOAD_FAILED',
      `PIKG download returned HTTP ${response.status}`,
      5,
      response.status >= 500,
    )
  }
  return new Uint8Array(await response.arrayBuffer())
}

async function stagePikg(
  ctx: CommandContext,
  snapshot: PikgSnapshot,
  purpose: PikgPurpose,
): Promise<PikgStagingMetadata> {
  const session = ctx.session
  if (!session) throw new ToolError('AUTH_REQUIRED', 'authenticated session is required', 3)
  const token = (await session.ensureValid()).token
  const hash = ndn.sha256Bytes(snapshot.bytes)
  const chunkId = ndn.ChunkId.fromMix256Result(snapshot.size, hash).toString()
  const proxy = ndm_proxy.createNdmProxyClient({
    endpoint: gatewayOrigin(ctx.connection.endpoint),
    sessionToken: token,
    fetcher: (request: RequestInfo | URL, init?: RequestInit) =>
      fetch(request, { ...init, signal: ctx.signal }),
  })
  try {
    await proxy.putChunk(chunkId, snapshot.bytes)
  } catch (error) {
    throw new ToolError(
      'PIKG_UPLOAD_FAILED',
      `failed to upload PIKG snapshot: ${errorMessage(error)}`,
      5,
      true,
    )
  }
  const value = expectObject(
    await callControl(ctx, 'apps.staging.finalize', {
      source_obj_id: chunkId,
      purpose,
    }),
    'apps.staging.finalize response',
  )
  return {
    schema_version: expectNumber(value, 'schema_version'),
    handle: expectString(value, 'handle'),
    pikg_digest: expectString(value, 'pikg_digest'),
    size: expectNumber(value, 'size'),
    purpose: expectString(value, 'purpose') as PikgPurpose,
    expires_at: optionalNumber(value.expires_at),
  }
}

async function inspectSource(
  ctx: CommandContext,
  source: PreparedSource,
  input: Record<string, unknown>,
  overrides: Record<string, unknown> = {},
): Promise<InstallInspection> {
  const result = await callControl(ctx, 'apps.inspect', {
    ...sourceRpcParams(source),
    ...scopeAndChoiceParams(input),
    ...overrides,
    options: installOptions(input),
  })
  return expectInspection(result)
}

async function finalizePlanChoices(
  ctx: CommandContext,
  source: PreparedSource,
  input: Record<string, unknown>,
  initial: InstallInspection,
): Promise<InstallInspection> {
  let inspection = initial
  const configReady = readinessValue(inspection, 'config') === 'READY'
  if (ctx.config.nonInteractive) {
    if (!configReady && input.target === undefined && input.install_params === undefined) {
      throw new ToolError(
        'PLAN_INPUT_REQUIRED',
        'Installer defaults do not form a complete install plan; provide target/install_params with --input',
      )
    }
    return inspection
  }

  await ctx.io.stderr(`${JSON.stringify(confirmationSummary(inspection), null, 2)}\n`)
  if (!ctx.io.inputIsTerminal || ctx.confirmed) return inspection
  const answer = (await ctx.io.prompt('Accept default install plan? [Y/e/n] '))?.trim()
    .toLowerCase()
  if (answer === 'n' || answer === 'no') {
    throw new ToolError(
      'CONFIRMATION_DECLINED',
      'install plan creation was declined',
      EXIT_PERMISSION,
    )
  }
  if (answer === 'e' || answer === 'edit') {
    const raw = await ctx.io.prompt('Plan choices JSON ({"target":...,"install_params":...}): ')
    const choices = parsePlanChoices(raw)
    inspection = expectInspection(
      await callControl(ctx, 'apps.plan.recompute', {
        ...sourceRpcParams(source),
        ...planScopeAndOptions(initial.plan),
        ...choices,
        plan: initial.plan,
        options: installOptions(input),
      }),
    )
  }
  if (readinessValue(inspection, 'config') !== 'READY') {
    throw new ToolError(
      'PLAN_INPUT_REQUIRED',
      'the selected plan still has unresolved configuration requirements',
    )
  }
  return inspection
}

async function findInstalled(
  ctx: CommandContext,
  appDid: string,
  ownerUserId?: string,
): Promise<Record<string, unknown> | undefined> {
  try {
    return expectObject(
      await callControl(ctx, 'apps.details', {
        selector: appDid,
        ...(ownerUserId ? { owner_user_id: ownerUserId } : {}),
      }),
      'apps.details response',
    )
  } catch (error) {
    if (error instanceof ToolError && error.code === 'RESOURCE_NOT_FOUND') return undefined
    throw error
  }
}

async function observeInstallTask(
  ctx: CommandContext,
  taskId: string,
): Promise<TaskObservation> {
  const status = expectObject(
    await callControl(ctx, 'apps.install.status', { task_id: taskId }),
    'apps.install.status response',
  )
  const error = isObject(status.error) ? status.error : undefined
  const sanitizedDetails = error && isObject(error.details)
    ? sanitizeAppOutput(error.details)
    : undefined
  return {
    phase: expectString(status, 'task_phase'),
    outcome: optionalString(status.task_outcome),
    message: installProgressMessage(status),
    error: error
      ? {
        code: optionalString(error.code),
        message: optionalString(error.message),
        retryable: typeof error.retryable === 'boolean' ? error.retryable : undefined,
        details: isObject(sanitizedDetails) ? sanitizedDetails : undefined,
      }
      : undefined,
    data: status,
  }
}

async function confirmChange(
  ctx: CommandContext,
  action: string,
  summary: Record<string, unknown>,
): Promise<void> {
  await ctx.io.stderr(`${JSON.stringify({ action, preflight: summary }, null, 2)}\n`)
  if (ctx.confirmed) return
  if (ctx.config.nonInteractive || !ctx.io.inputIsTerminal) {
    throw new ToolError(
      'CONFIRMATION_REQUIRED',
      `${action} requires --yes in non-interactive execution`,
      EXIT_PERMISSION,
    )
  }
  const answer = (await ctx.io.prompt(`Proceed with app ${action}? [y/N] `))?.trim().toLowerCase()
  if (answer !== 'y' && answer !== 'yes') {
    throw new ToolError('CONFIRMATION_DECLINED', `${action} was declined`, EXIT_PERMISSION)
  }
}

async function callControl<T = unknown>(
  ctx: CommandContext,
  method: string,
  params: Record<string, unknown>,
): Promise<T> {
  try {
    return await ctx.clients.call<T>(CONTROL_PANEL_SERVICE, method, params, rpcOptions(ctx))
  } catch (error) {
    throw normalizeAppServiceError(error)
  }
}

function normalizeAppServiceError(error: unknown): ToolError {
  if (error instanceof ToolError) return error
  const message = errorMessage(error)
  const structured = parseEmbeddedJson(message)
  if (structured && typeof structured.code === 'string') {
    const rawDetails = isObject(structured.details) ? structured.details : structured
    const details = sanitizeAppOutput(rawDetails)
    return new ToolError(
      structured.code,
      typeof structured.message === 'string' ? structured.message : message,
      structured.code === 'AMBIGUOUS_APP_TARGET' ? EXIT_OPERATION : EXIT_OPERATION,
      structured.retryable === true,
      isObject(details) ? details : {},
    )
  }
  const lower = message.toLowerCase()
  if (
    lower.includes('invalid token') || lower.includes('unauthorized') ||
    lower.includes('rpc call error: 401') || lower.includes('session expired') ||
    lower.includes('token expired')
  ) {
    return new ToolError(
      lower.includes('expired') ? 'SESSION_EXPIRED' : 'INVALID_SESSION',
      lower.includes('expired') ? 'the session token has expired' : message,
      EXIT_AUTH,
    )
  }
  if (lower.includes('abort') || lower.includes('cancel')) {
    return new ToolError('CANCELED', 'operation canceled', EXIT_TIMEOUT)
  }
  if (lower.includes('timeout') || lower.includes('timed out')) {
    return new ToolError('TIMEOUT', 'operation timed out', EXIT_TIMEOUT, true)
  }
  if (
    lower.includes('fetch failed') || lower.includes('connection refused') ||
    lower.includes('rpc call error: 502') || lower.includes('rpc call error: 503') ||
    lower.includes('rpc call error: 504')
  ) {
    return new ToolError('SERVICE_UNAVAILABLE', message, EXIT_UNAVAILABLE, true)
  }
  if (message.includes('APP_NOT_INSTALLED') || /not found/i.test(message)) {
    return new ToolError('RESOURCE_NOT_FOUND', message)
  }
  if (/permission|forbidden/i.test(message)) {
    return new ToolError('PERMISSION_DENIED', message, EXIT_PERMISSION)
  }
  if (/rolling restart is not supported/i.test(message)) {
    return new ToolError('UNSUPPORTED_STRATEGY', message)
  }
  return new ToolError('OPERATION_FAILED', message, EXIT_OPERATION)
}

function sourceRpcParams(source: PreparedSource): Record<string, unknown> {
  return { source: source.serviceSource }
}

function scopeAndChoiceParams(input: Record<string, unknown>): Record<string, unknown> {
  return {
    ...(typeof input.app_class === 'string' ? { app_class: input.app_class } : {}),
    ...(typeof input.owner_user_id === 'string' ? { user_id: input.owner_user_id } : {}),
    ...(isObject(input.target) ? { target: input.target } : {}),
    ...(isObject(input.install_params) ? { install_params: input.install_params } : {}),
  }
}

function planScopeAndOptions(plan: Record<string, unknown>): Record<string, unknown> {
  const scope = expectObject(plan.installation_scope, 'plan.installation_scope')
  return {
    app_class: expectString(scope, 'app_class'),
    user_id: expectString(scope, 'owner_user_id'),
    target: expectObject(plan.target, 'plan.target'),
    install_params: expectObject(plan.install_params, 'plan.install_params'),
  }
}

function installedScope(details: Record<string, unknown>): Record<string, unknown> {
  return {
    app_class: expectString(details, 'app_class'),
    user_id: expectString(details, 'owner_user_id'),
  }
}

function installOptions(input: Record<string, unknown>): Record<string, unknown> {
  const policy = optionalString(input.policy) ?? 'normal'
  return { policy: policy.replaceAll('-', '_').toUpperCase() }
}

function sourceSummary(source: PreparedSource): Record<string, unknown> {
  if (source.kind === 'catalog') return { kind: source.kind, identifier: source.display }
  return {
    kind: source.kind,
    source: source.display,
    pikg_digest: source.snapshot.digest,
    size: source.snapshot.size,
  }
}

function confirmationSummary(inspection: InstallInspection): Record<string, unknown> {
  const plan = inspection.plan
  return {
    plan_fingerprint: plan.plan_fingerprint,
    plan_use: plan.plan_use,
    app: plan.app,
    installation_id: plan.installation_id,
    installation_scope: plan.installation_scope,
    target: plan.target,
    selected_packages: plan.selected_packages,
    install_params: plan.install_params,
    readiness: inspection.status.readiness,
    warnings: inspection.status.warnings ?? [],
  }
}

function satisfiedResult(inspection: InstallInspection): Record<string, unknown> {
  return {
    action: 'satisfied',
    task_id: null,
    installation_id: inspection.plan.installation_id,
    app_doc_object_id: expectObject(inspection.plan.app, 'plan.app').object_id,
  }
}

function assertPlanMatchesInspection(
  plan: Record<string, unknown>,
  inspection: InstallInspection,
): void {
  const fingerprint = expectString(plan, 'plan_fingerprint')
  if (fingerprint !== inspection.plan.plan_fingerprint) {
    throw new ToolError(
      'PLAN_STALE',
      'the plan no longer matches the authoritative source, scope, target, or configuration',
      EXIT_OPERATION,
      false,
      {
        plan_fingerprint: fingerprint,
        current_fingerprint: inspection.plan.plan_fingerprint,
      },
    )
  }
}

function verifyPikgBinding(
  source: PreparedSource,
  plan: Record<string, unknown>,
): void {
  if (source.kind === 'catalog') return
  const identity = expectObject(plan.source_identity, 'plan.source_identity')
  const digest = expectString(identity, 'pikg_digest')
  if (digest !== source.snapshot.digest) {
    throw new ToolError(
      'PLAN_STALE',
      'the PIKG byte snapshot does not match the plan source digest',
      EXIT_OPERATION,
      false,
      { plan_digest: digest, source_digest: source.snapshot.digest },
    )
  }
}

function assertPortablePlan(plan: Record<string, unknown>): void {
  if (plan.schema_version !== PLAN_SCHEMA_VERSION) {
    throw new ToolError(
      'UNSUPPORTED_SCHEMA_VERSION',
      `InstallPlan schema must be v${PLAN_SCHEMA_VERSION}`,
    )
  }
  expectString(plan, 'plan_fingerprint')
  const allowed = new Set([
    'schema_version',
    'plan_use',
    'installation_id',
    'installation_scope',
    'source_identity',
    'app',
    'resolution',
    'target',
    'selected_packages',
    'required_contents',
    'install_params',
    'service_spec_config',
    'plan_fingerprint',
    'created_at',
  ])
  for (const key of Object.keys(plan)) {
    if (!allowed.has(key)) {
      throw new ToolError('PLAN_STALE', `InstallPlan contains an unknown field: ${key}`)
    }
  }
  assertNoPlanSecrets(plan, 'plan')
}

function assertNoPlanSecrets(value: unknown, path: string): void {
  if (Array.isArray(value)) {
    value.forEach((item, index) => assertNoPlanSecrets(item, `${path}[${index}]`))
    return
  }
  if (!isObject(value)) return
  for (const [key, child] of Object.entries(value)) {
    const normalized = key.toLowerCase()
    const disallowed = normalized === 'staging_handle' || normalized === 'staging_path' ||
      normalized === 'session_token' || normalized === 'refresh_token' ||
      normalized === 'private_key' || normalized === 'password' ||
      normalized.endsWith('_token') || normalized.endsWith('_password') ||
      normalized.endsWith('_private_key') ||
      (normalized.endsWith('secret') && normalized !== 'secret_ref')
    if (disallowed) {
      throw new ToolError('INVALID_PLAN_SECRET', `${path}.${key} is forbidden in InstallPlan`)
    }
    if (
      typeof child === 'string' &&
      (/^[a-z][a-z0-9+.-]*:\/\/[^\s/@:]+:[^\s/@]+@/i.test(child) ||
        /^(?:postgres(?:ql)?|mysql|mongodb(?:\+srv)?|redis):\/\//i.test(child))
    ) {
      throw new ToolError(
        'INVALID_PLAN_SECRET',
        `${path}.${key} contains a connection string; use SecretRef`,
      )
    }
    assertNoPlanSecrets(child, `${path}.${key}`)
  }
}

async function readPlanFile(path: string): Promise<Record<string, unknown>> {
  let raw: string
  try {
    raw = await Deno.readTextFile(path)
  } catch (error) {
    throw new UsageError('PLAN_READ_FAILED', `failed to read InstallPlan: ${errorMessage(error)}`)
  }
  let value: unknown
  try {
    value = JSON.parse(raw)
  } catch {
    throw new UsageError('INVALID_PLAN_JSON', 'InstallPlan is not valid JSON')
  }
  const plan = expectObject(value, 'InstallPlan')
  assertPortablePlan(plan)
  return plan
}

async function writePlanFile(
  ctx: CommandContext,
  path: string,
  plan: Record<string, unknown>,
): Promise<void> {
  const exists = await tryLstat(path)
  if (exists) {
    if (!exists.isFile) {
      throw new UsageError('INVALID_PLAN_PATH', 'InstallPlan destination is not a regular file')
    }
    if (!ctx.confirmed) {
      if (ctx.config.nonInteractive || !ctx.io.inputIsTerminal) {
        throw new ToolError(
          'CONFIRMATION_REQUIRED',
          `InstallPlan already exists: ${path}; use --yes to overwrite it`,
          EXIT_PERMISSION,
        )
      }
      const answer = (await ctx.io.prompt(`Overwrite existing InstallPlan ${path}? [y/N] `))
        ?.trim().toLowerCase()
      if (answer !== 'y' && answer !== 'yes') {
        throw new ToolError(
          'CONFIRMATION_DECLINED',
          'InstallPlan overwrite was declined',
          EXIT_PERMISSION,
        )
      }
    }
  }
  const content = `${JSON.stringify(plan, null, 2)}\n`
  try {
    await Deno.writeTextFile(
      path,
      content,
      exists ? { mode: 0o600 } : { createNew: true, mode: 0o600 },
    )
    if (Deno.build.os !== 'windows') await Deno.chmod(path, 0o600)
  } catch (error) {
    throw new UsageError('PLAN_WRITE_FAILED', `failed to write InstallPlan: ${errorMessage(error)}`)
  }
}

async function releaseStaging(ctx: CommandContext, source: PreparedSource): Promise<void> {
  if (source.kind === 'catalog') return
  try {
    await callControl(ctx, 'apps.staging.release', { staging_handle: source.staging.handle })
  } catch (error) {
    await ctx.io.stderr(`warning: failed to release PIKG staging lease: ${errorMessage(error)}\n`)
  }
}

function expectInspection(value: unknown): InstallInspection {
  const object = expectObject(value, 'apps.inspect response')
  const schemaVersion = expectNumber(object, 'schema_version')
  if (schemaVersion !== PLAN_SCHEMA_VERSION) {
    throw new ToolError(
      'UNSUPPORTED_SCHEMA_VERSION',
      `Installer returned schema v${schemaVersion}; v${PLAN_SCHEMA_VERSION} is required`,
    )
  }
  const inspection: InstallInspection = {
    schema_version: schemaVersion,
    plan: expectObject(object.plan, 'inspection.plan'),
    resolution_status: expectObject(object.resolution_status, 'inspection.resolution_status'),
    status: expectObject(object.status, 'inspection.status'),
  }
  assertPortablePlan(inspection.plan)
  if (inspection.status.plan_fingerprint !== inspection.plan.plan_fingerprint) {
    throw new ToolError('INVALID_SERVICE_RESPONSE', 'inspection fingerprint fields disagree', 9)
  }
  return inspection
}

function parsePlanChoices(raw: string | null | undefined): Record<string, unknown> {
  if (!raw) throw new ToolError('PLAN_INPUT_REQUIRED', 'plan choices JSON is required')
  let value: unknown
  try {
    value = JSON.parse(raw)
  } catch {
    throw new UsageError('INVALID_INPUT_JSON', 'plan choices are not valid JSON')
  }
  const choices = expectObject(value, 'plan choices')
  for (const key of Object.keys(choices)) {
    if (key !== 'target' && key !== 'install_params') {
      throw new UsageError('INVALID_ARGUMENT', `unknown plan choice: ${key}`)
    }
  }
  return choices
}

function normalizeCatalogIdentifier(value: string): string {
  const raw = value.trim()
  if (
    !raw || /^https?:\/\//i.test(raw) || raw.includes('/') || raw.includes('\\') || /\s/.test(raw)
  ) {
    throw new UsageError('INVALID_APP_NAME', `invalid Catalog App identifier: ${value}`)
  }
  if (raw.startsWith('did:')) {
    return parseAppDid(raw)
  }
  return raw.includes('.') ? raw.toLowerCase() : parseAppDid(`did:bns:${raw}`)
}

function normalizeAppSelector(value: string): string {
  return normalizeCatalogIdentifier(value)
}

function parseAppDid(raw: string): string {
  let did: InstanceType<typeof namelib.DID>
  try {
    did = namelib.DID.fromStr(raw)
  } catch (error) {
    throw new UsageError('INVALID_APP_DID', `invalid App DID: ${errorMessage(error)}`)
  }
  if (did.method === 'key' || did.method === 'dev') {
    throw new UsageError('INVALID_APP_DID', 'key-class DIDs cannot identify an App')
  }
  return did.toString()
}

function parsePikgUrl(value: string): URL {
  let url: URL
  try {
    url = new URL(value)
  } catch {
    throw new UsageError('INVALID_PIKG_URL', `invalid PIKG URL: ${value}`)
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    throw new UsageError('INVALID_PIKG_URL', 'PIKG URL must use HTTP or HTTPS')
  }
  if (url.username || url.password) {
    throw new UsageError('INVALID_PIKG_URL', 'PIKG URL must not contain credentials')
  }
  return url
}

function validatePikgSnapshot(bytes: Uint8Array): void {
  if (bytes.length < ZIP_LOCAL_MAGIC.length) {
    throw new UsageError('INVALID_PIKG', 'PIKG is too small to be a ZIP container')
  }
  if (!ZIP_LOCAL_MAGIC.every((byte, index) => bytes[index] === byte)) {
    throw new UsageError('INVALID_PIKG', 'PIKG magic mismatch: expected a ZIP container')
  }
}

async function sha256Id(bytes: Uint8Array): Promise<string> {
  const digestInput = Uint8Array.from(bytes)
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', digestInput.buffer))
  return `sha256:${[...digest].map((byte) => byte.toString(16).padStart(2, '0')).join('')}`
}

function gatewayOrigin(endpoint: string): string {
  const url = new URL(endpoint)
  const marker = url.pathname.indexOf('/kapi')
  url.pathname = marker >= 0 ? url.pathname.slice(0, marker) || '/' : '/'
  url.search = ''
  url.hash = ''
  return url.toString().replace(/\/$/, '')
}

function safeUrl(url: URL): string {
  const safe = new URL(url)
  safe.search = ''
  safe.hash = ''
  return safe.toString()
}

function looksLikeLocalPath(value: string): boolean {
  return value.includes('/') || value.includes('\\') || value.startsWith('.') ||
    value.toLowerCase().endsWith('.pikg')
}

async function tryLstat(path: string): Promise<Deno.FileInfo | undefined> {
  try {
    return await Deno.lstat(path)
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return undefined
    throw new UsageError('FILE_ACCESS_FAILED', `failed to inspect ${path}: ${errorMessage(error)}`)
  }
}

function resolveFromCwd(ctx: CommandContext, path: string): string {
  return isAbsolute(path) ? resolve(path) : resolve(ctx.cwd, path)
}

function parseEmbeddedJson(message: string): Record<string, unknown> | undefined {
  for (let index = message.indexOf('{'); index >= 0; index = message.indexOf('{', index + 1)) {
    try {
      const value = JSON.parse(message.slice(index))
      if (isObject(value)) return value
    } catch {
      // Try the next opening brace; kRPC prefixes structured service errors.
    }
  }
  return undefined
}

function readinessValue(inspection: InstallInspection, field: string): string | undefined {
  const readiness = isObject(inspection.status.readiness) ? inspection.status.readiness : undefined
  return readiness ? optionalString(readiness[field]) : undefined
}

function planUse(plan: Record<string, unknown>): string | undefined {
  return optionalString(plan.plan_use)
}

function installProgressMessage(status: Record<string, unknown>): string | undefined {
  const stage = optionalString(status.stage)
  const progress = isObject(status.progress) ? status.progress : undefined
  const percent = progress && typeof progress.percent === 'number'
    ? `${progress.percent}%`
    : undefined
  return [stage, percent].filter(Boolean).join(' ') || undefined
}

function idempotencyKey(ctx: CommandContext): string {
  return ctx.idempotencyKey ?? crypto.randomUUID()
}

function rejectDryRunNoWait(input: Record<string, unknown>): void {
  if (input.dry_run === true && input.no_wait === true) {
    throw new UsageError('ARGUMENT_CONFLICT', '--dry-run and --no-wait are mutually exclusive')
  }
}

function selectorCommand(options: {
  verb: string
  summary: string
  access: 'read' | 'write' | 'destructive'
  asyncMode: 'sync' | 'task' | 'either'
  handler: CommandDefinition['handler']
}): CommandDefinition {
  return {
    verb: options.verb,
    summary: options.summary,
    positionals: [{ name: 'app_name', description: 'Installed App name or DID' }],
    inputSchema: {
      type: 'object',
      properties: { app_name: { type: 'string', minLength: 1 } },
      required: ['app_name'],
      additionalProperties: false,
    },
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: options.access },
    asyncMode: options.asyncMode,
    requiresSession: true,
    examples: [`buckyos app ${options.verb} app1`],
    handler: options.handler,
  }
}

function sourceOptions(
  extra: NonNullable<CommandDefinition['options']>,
): NonNullable<CommandDefinition['options']> {
  return [
    {
      name: 'from',
      description: 'Force source type',
      type: 'string',
      enum: ['catalog', 'pikg', 'url'],
    },
    { name: 'pikg', description: 'Explicit local PIKG path', type: 'string' },
    ...extra,
  ]
}

function sourceInputSchema(extra: Record<string, JsonSchema>): JsonSchema {
  return {
    type: 'object',
    properties: {
      source: { type: 'string', minLength: 1 },
      from: { type: 'string', enum: ['catalog', 'pikg', 'url'] },
      pikg: { type: 'string', minLength: 1 },
      ...extra,
    },
    additionalProperties: false,
  }
}

function emptyInputSchema(): JsonSchema {
  return { type: 'object', properties: {}, additionalProperties: false }
}

function appClassOption(): NonNullable<CommandDefinition['options']>[number] {
  return {
    name: 'app-class',
    property: 'app_class',
    description: 'Installation App class',
    type: 'string',
    enum: [...APP_CLASSES],
  }
}

function ownerOption(): NonNullable<CommandDefinition['options']>[number] {
  return {
    name: 'owner',
    property: 'owner_user_id',
    description: 'Installation owner user',
    type: 'string',
  }
}

function policyOption(): NonNullable<CommandDefinition['options']>[number] {
  return {
    name: 'policy',
    description: 'Installer trust policy',
    type: 'string',
    enum: [...INSTALL_POLICIES],
  }
}

function rpcOptions(ctx: CommandContext): {
  traceId: string
  timeoutMs: number
  signal: AbortSignal
} {
  return {
    traceId: ctx.traceId,
    timeoutMs: Math.max(1, (ctx.deadline ?? Date.now()) - Date.now()),
    signal: ctx.signal,
  }
}

function expectObject(value: unknown, label: string): Record<string, unknown> {
  if (!isObject(value)) {
    throw new ToolError('INVALID_SERVICE_RESPONSE', `${label} must be an object`, EXIT_INTERNAL)
  }
  return value
}

function isObject(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value)
}

function expectString(object: Record<string, unknown>, property: string): string
function expectString(value: unknown, property: string): string
function expectString(value: unknown, property: string): string {
  const candidate = isObject(value) ? value[property] : value
  if (typeof candidate !== 'string' || !candidate.trim()) {
    throw new ToolError('INVALID_SERVICE_RESPONSE', `${property} must be a non-empty string`, 9)
  }
  return candidate
}

function expectNumber(object: Record<string, unknown>, property: string): number {
  const value = object[property]
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw new ToolError('INVALID_SERVICE_RESPONSE', `${property} must be a non-negative integer`, 9)
  }
  return value
}

function optionalString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value : undefined
}

function optionalNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function sanitizeAppOutput(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sanitizeAppOutput)
  if (!isObject(value)) return value
  const result: Record<string, unknown> = {}
  const forbidden = new Set([
    'app_instance_id',
    'spec_path',
    'staging_handle',
    'staging_path',
    'source_url',
    'session_token',
    'refresh_token',
    'private_key',
    'password',
    'connection_string',
    'database_url',
  ])
  for (const [key, child] of Object.entries(value)) {
    if (!forbidden.has(key.toLowerCase())) result[key] = sanitizeAppOutput(child)
  }
  return result
}
