/* ── Workflow WebUI mock store ── */

import type {
  AmendmentSummary,
  AppWorkflowMountPoint,
  ExecutorEntry,
  WorkflowApp,
  WorkflowDefinition,
  WorkflowRunSummary,
} from './types'

const SCHEMA_VERSION = '0.4'

function emptyAnalysis() {
  return { issues: [], errorCount: 0, warnCount: 0, infoCount: 0 }
}

const kbImportPipeline: WorkflowDefinition = {
  id: 'wf-kb-import-001',
  schemaVersion: SCHEMA_VERSION,
  name: 'kb_default_import_pipeline',
  description:
    'Default Knowledge-Base import pipeline: scan → enrich → embed → index.',
  version: 3,
  source: 'system',
  status: 'active',
  createdAt: '2026-04-01T08:00:00Z',
  updatedAt: '2026-04-22T10:30:00Z',
  tags: ['kb', 'system'],
  analysis: {
    issues: [
      {
        severity: 'info',
        code: 'output_mode_propagated',
        message:
          'finite_seekable propagated from scan_files to batch_embed downstream.',
        nodeId: 'scan_files',
      },
    ],
    errorCount: 0,
    warnCount: 0,
    infoCount: 1,
  },
  graph: {
    definitionId: 'wf-kb-import-001',
    definitionVersion: 3,
    schemaVersion: SCHEMA_VERSION,
    nodes: [
      {
        kind: 'task',
        id: 'scan_files',
        name: 'Scan files',
        description: 'Walk the source directory and emit file references.',
        stepType: 'autonomous',
        executor: {
          raw: '/skill/fs-scanner',
          resolvedNamespace: 'service',
          resolvedTarget: 'service::fs.scan',
        },
        outputMode: 'finite_seekable',
        idempotent: true,
        skippable: false,
        inputBindings: [
          { kind: 'literal', field: 'root', value: '${input.source_path}' },
        ],
        guards: {
          budget: { maxDuration: 'PT15M' },
          retry: { maxAttempts: 2, backoff: 'exponential', fallback: 'human' },
        },
      },
      {
        kind: 'control',
        controlType: 'for_each',
        id: 'each_file',
        name: 'For each file',
        items: { nodeId: 'scan_files', fieldPath: ['files'] },
        steps: ['enrich_meta', 'embed'],
        maxItems: 50000,
        concurrency: 8,
        effectiveConcurrency: 8,
      },
      {
        kind: 'task',
        id: 'enrich_meta',
        name: 'Enrich metadata',
        stepType: 'autonomous',
        executor: {
          raw: '/skill/meta-extract',
          resolvedNamespace: 'service',
          resolvedTarget: 'service::meta.extract',
        },
        outputMode: 'single',
        idempotent: true,
        skippable: true,
        inputBindings: [
          {
            kind: 'reference',
            field: 'file',
            nodeId: 'each_file',
            fieldPath: ['item'],
          },
        ],
      },
      {
        kind: 'task',
        id: 'embed',
        name: 'Embed chunks',
        stepType: 'autonomous',
        executor: {
          raw: '/agent/aicc',
          resolvedNamespace: 'service',
          resolvedTarget: 'service::aicc.complete',
        },
        outputMode: 'single',
        idempotent: true,
        skippable: false,
        inputBindings: [
          {
            kind: 'reference',
            field: 'meta',
            nodeId: 'enrich_meta',
            fieldPath: ['meta'],
          },
        ],
        guards: {
          budget: { maxTokens: 200_000, maxCostUsdb: 100 },
        },
      },
      {
        kind: 'task',
        id: 'index_write',
        name: 'Write index',
        stepType: 'autonomous',
        executor: {
          raw: 'service::kb.index_write',
          resolvedNamespace: 'service',
          resolvedTarget: 'service::kb.index_write',
        },
        outputMode: 'single',
        idempotent: true,
        skippable: false,
        inputBindings: [
          {
            kind: 'reference',
            field: 'embeddings',
            nodeId: 'each_file',
            fieldPath: ['results'],
          },
        ],
      },
    ],
    edges: [
      { id: 'e1', source: 'scan_files', target: 'each_file' },
      { id: 'e2', source: 'each_file', target: 'enrich_meta', implicit: true },
      { id: 'e3', source: 'enrich_meta', target: 'embed' },
      { id: 'e4', source: 'each_file', target: 'index_write' },
    ],
  },
}

const imageThumbnail: WorkflowDefinition = {
  id: 'wf-image-thumb-001',
  schemaVersion: SCHEMA_VERSION,
  name: 'image_thumbnail_pipeline',
  description: 'Generate thumbnails for new image assets.',
  version: 2,
  source: 'system',
  status: 'active',
  createdAt: '2026-03-12T09:00:00Z',
  updatedAt: '2026-04-18T11:20:00Z',
  tags: ['ai-fs', 'system'],
  analysis: emptyAnalysis(),
  graph: {
    definitionId: 'wf-image-thumb-001',
    definitionVersion: 2,
    schemaVersion: SCHEMA_VERSION,
    nodes: [
      {
        kind: 'task',
        id: 'detect_format',
        name: 'Detect format',
        stepType: 'autonomous',
        executor: {
          raw: 'operator::image.detect',
          resolvedNamespace: 'operator',
          resolvedTarget: 'operator::image.detect',
        },
        outputMode: 'single',
        idempotent: true,
        skippable: false,
        inputBindings: [],
      },
      {
        kind: 'control',
        controlType: 'branch',
        id: 'by_kind',
        name: 'By kind',
        on: { nodeId: 'detect_format', fieldPath: ['kind'] },
        paths: {
          raster: 'thumb_raster',
          vector: 'thumb_vector',
          unsupported: 'fail_unsupported',
        },
      },
      {
        kind: 'task',
        id: 'thumb_raster',
        name: 'Raster thumbnail',
        stepType: 'autonomous',
        executor: {
          raw: 'operator::image.resize',
          resolvedNamespace: 'operator',
          resolvedTarget: 'operator::image.resize',
        },
        outputMode: 'single',
        idempotent: true,
        skippable: false,
        inputBindings: [],
      },
      {
        kind: 'task',
        id: 'thumb_vector',
        name: 'Vector thumbnail',
        stepType: 'autonomous',
        executor: {
          raw: 'operator::image.rasterize',
          resolvedNamespace: 'operator',
          resolvedTarget: 'operator::image.rasterize',
        },
        outputMode: 'single',
        idempotent: true,
        skippable: false,
        inputBindings: [],
      },
      {
        kind: 'task',
        id: 'fail_unsupported',
        name: 'Mark unsupported',
        stepType: 'autonomous',
        executor: {
          raw: 'operator::tag.set',
          resolvedNamespace: 'operator',
          resolvedTarget: 'operator::tag.set',
        },
        outputMode: 'single',
        idempotent: true,
        skippable: true,
        inputBindings: [],
      },
    ],
    edges: [
      { id: 'e1', source: 'detect_format', target: 'by_kind' },
      {
        id: 'e2',
        source: 'by_kind',
        target: 'thumb_raster',
        implicit: true,
        conditionLabel: 'raster',
      },
      {
        id: 'e3',
        source: 'by_kind',
        target: 'thumb_vector',
        implicit: true,
        conditionLabel: 'vector',
      },
      {
        id: 'e4',
        source: 'by_kind',
        target: 'fail_unsupported',
        implicit: true,
        conditionLabel: 'unsupported',
      },
    ],
  },
}

const myImported: WorkflowDefinition = {
  id: 'wf-imported-001',
  schemaVersion: SCHEMA_VERSION,
  name: 'my_imported_pipeline',
  description: 'A user-imported pipeline pending review.',
  version: 1,
  source: 'user_imported',
  status: 'draft',
  createdAt: '2026-04-30T20:00:00Z',
  updatedAt: '2026-04-30T20:00:00Z',
  tags: ['imported'],
  analysis: {
    issues: [
      {
        severity: 'warn',
        code: 'for_each_concurrency_downgraded',
        message:
          'concurrency forced to 1 due to finite_sequential upstream output_mode.',
        nodeId: 'batch_step',
      },
      {
        severity: 'warn',
        code: 'idempotent_unverified',
        message: 'autonomous step "publish" is not declared idempotent.',
        nodeId: 'publish',
      },
    ],
    errorCount: 0,
    warnCount: 2,
    infoCount: 0,
  },
  graph: {
    definitionId: 'wf-imported-001',
    definitionVersion: 1,
    schemaVersion: SCHEMA_VERSION,
    nodes: [
      {
        kind: 'task',
        id: 'fetch_input',
        name: 'Fetch input',
        stepType: 'autonomous',
        executor: {
          raw: 'http::api.fetch',
          resolvedNamespace: 'http',
          resolvedTarget: 'http::api.fetch',
        },
        outputMode: 'finite_sequential',
        idempotent: true,
        skippable: false,
        inputBindings: [],
      },
      {
        kind: 'control',
        controlType: 'for_each',
        id: 'batch_step',
        name: 'Per-row processing',
        items: { nodeId: 'fetch_input', fieldPath: ['rows'] },
        steps: ['transform'],
        maxItems: 1000,
        concurrency: 4,
        effectiveConcurrency: 1,
        degradedReason:
          'upstream fetch_input is finite_sequential; concurrency capped to 1',
      },
      {
        kind: 'task',
        id: 'transform',
        name: 'Transform row',
        stepType: 'autonomous',
        executor: {
          raw: '/agent/normalize',
          resolvedNamespace: 'service',
          resolvedTarget: 'service::aicc.complete',
        },
        outputMode: 'single',
        idempotent: true,
        skippable: false,
        inputBindings: [],
      },
      {
        kind: 'task',
        id: 'review',
        name: 'Manual review',
        stepType: 'human_confirm',
        prompt: 'Please confirm the transformed dataset before publishing.',
        outputMode: 'single',
        idempotent: true,
        skippable: false,
        inputBindings: [],
        subjectRef: { nodeId: 'batch_step', fieldPath: ['results'] },
      },
      {
        kind: 'task',
        id: 'publish',
        name: 'Publish',
        stepType: 'autonomous',
        executor: {
          raw: 'appservice::publisher.push',
          resolvedNamespace: 'appservice',
          resolvedTarget: 'appservice::publisher.push',
        },
        outputMode: 'single',
        idempotent: false,
        skippable: false,
        inputBindings: [],
      },
    ],
    edges: [
      { id: 'e1', source: 'fetch_input', target: 'batch_step' },
      { id: 'e2', source: 'batch_step', target: 'transform', implicit: true },
      { id: 'e3', source: 'batch_step', target: 'review' },
      { id: 'e4', source: 'review', target: 'publish' },
    ],
  },
}

const fileOrganizer: WorkflowDefinition = {
  id: 'wf-file-org-001',
  schemaVersion: SCHEMA_VERSION,
  name: 'file_organizer_pipeline',
  description: 'Periodically classify and move files into folders.',
  version: 1,
  source: 'agent_generated',
  status: 'active',
  createdAt: '2026-04-15T12:00:00Z',
  updatedAt: '2026-04-29T08:00:00Z',
  tags: ['script-app'],
  analysis: emptyAnalysis(),
  graph: {
    definitionId: 'wf-file-org-001',
    definitionVersion: 1,
    schemaVersion: SCHEMA_VERSION,
    nodes: [
      {
        kind: 'task',
        id: 'list_files',
        name: 'List files',
        stepType: 'autonomous',
        executor: {
          raw: '/skill/fs-scanner',
          resolvedNamespace: 'service',
          resolvedTarget: 'service::fs.scan',
        },
        outputMode: 'finite_seekable',
        idempotent: true,
        skippable: false,
        inputBindings: [],
      },
      {
        kind: 'control',
        controlType: 'parallel',
        id: 'fanout',
        name: 'Classify in parallel',
        branches: ['classify_kind', 'classify_topic'],
        join: { strategy: 'all' },
      },
      {
        kind: 'task',
        id: 'classify_kind',
        name: 'Classify by kind',
        stepType: 'autonomous',
        executor: {
          raw: 'operator::file.classify_kind',
          resolvedNamespace: 'operator',
          resolvedTarget: 'operator::file.classify_kind',
        },
        outputMode: 'single',
        idempotent: true,
        skippable: false,
        inputBindings: [],
      },
      {
        kind: 'task',
        id: 'classify_topic',
        name: 'Classify by topic',
        stepType: 'autonomous',
        executor: {
          raw: '/agent/aicc',
          resolvedNamespace: 'service',
          resolvedTarget: 'service::aicc.complete',
        },
        outputMode: 'single',
        idempotent: true,
        skippable: false,
        inputBindings: [],
      },
      {
        kind: 'task',
        id: 'move_files',
        name: 'Move to folders',
        stepType: 'human_confirm',
        prompt: 'Confirm proposed moves before applying.',
        outputMode: 'single',
        idempotent: true,
        skippable: false,
        inputBindings: [],
      },
    ],
    edges: [
      { id: 'e1', source: 'list_files', target: 'fanout' },
      { id: 'e2', source: 'fanout', target: 'classify_kind', implicit: true },
      { id: 'e3', source: 'fanout', target: 'classify_topic', implicit: true },
      { id: 'e4', source: 'fanout', target: 'move_files' },
    ],
  },
}

const seedDefinitions: WorkflowDefinition[] = [
  kbImportPipeline,
  imageThumbnail,
  myImported,
  fileOrganizer,
]

function makeBinding(definitionId: string, version: number) {
  return {
    definitionId,
    definitionVersion: version,
    boundAt: '2026-04-20T10:00:00Z',
    boundBy: 'system',
  }
}

const seedApps: WorkflowApp[] = [
  {
    id: 'knowledgebase',
    name: 'KnowledgeBase',
    kind: 'app',
    description: 'Personal knowledge base.',
    mountPoints: [
      {
        id: 'document_import_pipeline',
        appId: 'knowledgebase',
        name: 'Document import pipeline',
        description: 'Runs whenever a new document is added to the KB.',
        required: true,
        allowEmpty: false,
        defaultDefinitionId: 'wf-kb-import-001',
        currentBinding: makeBinding('wf-kb-import-001', 3),
      },
      {
        id: 'log_process_pipeline',
        appId: 'knowledgebase',
        name: 'Log post-processing',
        required: false,
        allowEmpty: true,
      },
    ],
  },
  {
    id: 'ai-fs',
    name: 'AI File System',
    kind: 'app',
    description: 'AI-managed file system.',
    mountPoints: [
      {
        id: 'thumbnail_generate_pipeline',
        appId: 'ai-fs',
        name: 'Thumbnail generation',
        required: true,
        allowEmpty: false,
        defaultDefinitionId: 'wf-image-thumb-001',
        currentBinding: makeBinding('wf-image-thumb-001', 2),
      },
      {
        id: 'file_index_pipeline',
        appId: 'ai-fs',
        name: 'File indexing',
        required: false,
        allowEmpty: false,
        defaultDefinitionId: 'wf-kb-import-001',
      },
    ],
  },
  {
    id: 'file-organizer',
    name: 'File Organizer',
    kind: 'script_app',
    description: 'Script app: periodic folder cleanup.',
    mountPoints: [
      {
        id: 'default_pipeline',
        appId: 'file-organizer',
        name: 'Default pipeline',
        required: true,
        allowEmpty: false,
        defaultDefinitionId: 'wf-file-org-001',
        currentBinding: makeBinding('wf-file-org-001', 1),
      },
    ],
  },
]

const seedRuns: WorkflowRunSummary[] = [
  {
    runId: 'run-001',
    rootTaskId: 'task-wf-001',
    definitionId: 'wf-kb-import-001',
    definitionVersion: 3,
    planVersion: 1,
    status: 'running',
    triggerSource: 'app',
    appId: 'knowledgebase',
    mountPointId: 'document_import_pipeline',
    humanWaitingNodes: [],
    startedAt: '2026-05-02T07:30:00Z',
    taskmgrUrl: 'taskmgr/task-wf-001?from=workflow_webui',
  },
  {
    runId: 'run-002',
    rootTaskId: 'task-wf-002',
    definitionId: 'wf-kb-import-001',
    definitionVersion: 3,
    planVersion: 1,
    status: 'waiting_human',
    triggerSource: 'app',
    appId: 'knowledgebase',
    mountPointId: 'document_import_pipeline',
    humanWaitingNodes: ['embed'],
    startedAt: '2026-05-02T06:10:00Z',
    taskmgrUrl: 'taskmgr/task-wf-002?from=workflow_webui',
  },
  {
    runId: 'run-003',
    rootTaskId: 'task-wf-003',
    definitionId: 'wf-kb-import-001',
    definitionVersion: 2,
    planVersion: 2,
    status: 'completed',
    triggerSource: 'manual',
    appId: 'knowledgebase',
    mountPointId: 'document_import_pipeline',
    humanWaitingNodes: [],
    startedAt: '2026-05-01T22:00:00Z',
    finishedAt: '2026-05-01T22:34:00Z',
    durationMs: 34 * 60 * 1000,
    taskmgrUrl: 'taskmgr/task-wf-003?from=workflow_webui',
  },
  {
    runId: 'run-004',
    rootTaskId: 'task-wf-004',
    definitionId: 'wf-kb-import-001',
    definitionVersion: 3,
    planVersion: 1,
    status: 'failed',
    triggerSource: 'app',
    appId: 'knowledgebase',
    mountPointId: 'document_import_pipeline',
    humanWaitingNodes: [],
    startedAt: '2026-05-01T17:30:00Z',
    finishedAt: '2026-05-01T17:32:00Z',
    durationMs: 2 * 60 * 1000,
    errorSummary: 'embed step exceeded max_tokens budget',
    taskmgrUrl: 'taskmgr/task-wf-004?from=workflow_webui',
  },
  {
    runId: 'run-005',
    rootTaskId: 'task-wf-005',
    definitionId: 'wf-image-thumb-001',
    definitionVersion: 2,
    planVersion: 1,
    status: 'completed',
    triggerSource: 'app',
    appId: 'ai-fs',
    mountPointId: 'thumbnail_generate_pipeline',
    humanWaitingNodes: [],
    startedAt: '2026-05-02T05:00:00Z',
    finishedAt: '2026-05-02T05:00:08Z',
    durationMs: 8000,
    taskmgrUrl: 'taskmgr/task-wf-005?from=workflow_webui',
  },
  {
    runId: 'run-006',
    rootTaskId: 'task-wf-006',
    definitionId: 'wf-file-org-001',
    definitionVersion: 1,
    planVersion: 1,
    status: 'waiting_human',
    triggerSource: 'system',
    appId: 'file-organizer',
    mountPointId: 'default_pipeline',
    humanWaitingNodes: ['move_files'],
    startedAt: '2026-05-02T04:00:00Z',
    taskmgrUrl: 'taskmgr/task-wf-006?from=workflow_webui',
  },
]

const seedAmendments: AmendmentSummary[] = [
  {
    runId: 'run-003',
    planVersion: 2,
    submittedBy: '/agent/aicc',
    submittedAtStep: 'embed',
    approvalStatus: 'approved',
    reason: 'extend pipeline with deduplication after embed',
    operations: [
      {
        op: 'insert_after',
        target: 'embed',
        description: 'inject `dedup` autonomous step',
      },
    ],
  },
]

const seedExecutors: ExecutorEntry[] = [
  {
    id: 'service::aicc.complete',
    namespace: 'service',
    description: 'AI completion via aicc service.',
    inputSummary: '{ prompt: string, model?: string }',
    outputSummary: '{ text: string, usage }',
  },
  {
    id: 'service::fs.scan',
    namespace: 'service',
    description: 'Walk a directory and emit file references.',
    inputSummary: '{ root: string }',
    outputSummary: '{ files: FileRef[] (finite_seekable) }',
  },
  {
    id: 'service::kb.index_write',
    namespace: 'service',
    description: 'Write embeddings into the KB index.',
    inputSummary: '{ embeddings: Embedding[] }',
    outputSummary: '{ written: number }',
  },
  {
    id: 'service::meta.extract',
    namespace: 'service',
    description: 'Extract metadata for a file reference.',
    inputSummary: '{ file: FileRef }',
    outputSummary: '{ meta: object }',
  },
  {
    id: 'operator::image.detect',
    namespace: 'operator',
    description: 'Detect image format kind.',
    inputSummary: '{ file: FileRef }',
    outputSummary: '{ kind: "raster" | "vector" | "unsupported" }',
  },
  {
    id: 'operator::image.resize',
    namespace: 'operator',
    description: 'Resize a raster image.',
    inputSummary: '{ file, width, height }',
    outputSummary: '{ thumbnail: FileRef }',
  },
  {
    id: 'http::api.fetch',
    namespace: 'http',
    description: 'Generic HTTP fetcher.',
    inputSummary: '{ url, method }',
    outputSummary: '{ rows: any[] }',
  },
  {
    id: 'appservice::publisher.push',
    namespace: 'appservice',
    description: 'Publish to an app-service endpoint.',
    inputSummary: '{ payload }',
    outputSummary: '{ ok: boolean }',
  },
]

export class WorkflowMockStore {
  definitions: WorkflowDefinition[] = seedDefinitions
  apps: WorkflowApp[] = seedApps
  runs: WorkflowRunSummary[] = seedRuns
  amendments: AmendmentSummary[] = seedAmendments
  executors: ExecutorEntry[] = seedExecutors
  schemaVersion = SCHEMA_VERSION

  listDefinitions(): WorkflowDefinition[] {
    return this.definitions
  }

  getDefinition(id: string): WorkflowDefinition | undefined {
    return this.definitions.find((d) => d.id === id)
  }

  listApps(): WorkflowApp[] {
    return this.apps
  }

  findMountPoint(
    appId: string,
    mountPointId: string,
  ): { app: WorkflowApp; mp: AppWorkflowMountPoint } | undefined {
    const app = this.apps.find((a) => a.id === appId)
    if (!app) return undefined
    const mp = app.mountPoints.find((m) => m.id === mountPointId)
    if (!mp) return undefined
    return { app, mp }
  }

  listRunsForMountPoint(
    appId: string,
    mountPointId: string,
    limit = 20,
  ): WorkflowRunSummary[] {
    return this.runs
      .filter((r) => r.appId === appId && r.mountPointId === mountPointId)
      .slice(0, limit)
  }

  listAmendments(runId: string): AmendmentSummary[] {
    return this.amendments.filter((a) => a.runId === runId)
  }

  listMountPointsUsing(definitionId: string): Array<{
    app: WorkflowApp
    mp: AppWorkflowMountPoint
  }> {
    const out: Array<{ app: WorkflowApp; mp: AppWorkflowMountPoint }> = []
    for (const app of this.apps) {
      for (const mp of app.mountPoints) {
        if (mp.currentBinding?.definitionId === definitionId) {
          out.push({ app, mp })
        }
      }
    }
    return out
  }

  addDefinition(def: WorkflowDefinition): void {
    this.definitions.unshift(def)
  }

  bindMountPoint(
    appId: string,
    mountPointId: string,
    definitionId: string,
    definitionVersion: number,
  ): boolean {
    const found = this.findMountPoint(appId, mountPointId)
    if (!found) return false
    found.mp.currentBinding = {
      definitionId,
      definitionVersion,
      boundAt: new Date().toISOString(),
      boundBy: 'user',
    }
    return true
  }

  restoreDefaultBinding(appId: string, mountPointId: string): boolean {
    const found = this.findMountPoint(appId, mountPointId)
    if (!found) return false
    const def = found.mp.defaultDefinitionId
      ? this.getDefinition(found.mp.defaultDefinitionId)
      : undefined
    if (!def) {
      found.mp.currentBinding = undefined
      return true
    }
    found.mp.currentBinding = {
      definitionId: def.id,
      definitionVersion: def.version,
      boundAt: new Date().toISOString(),
      boundBy: 'user',
    }
    return true
  }

  unbindMountPoint(appId: string, mountPointId: string): boolean {
    const found = this.findMountPoint(appId, mountPointId)
    if (!found || !found.mp.allowEmpty) return false
    found.mp.currentBinding = undefined
    return true
  }

  buildAiPrompt(): string {
    const exec = this.executors
      .map((e) => `- ${e.id} :: ${e.description} (in: ${e.inputSummary ?? '?'}; out: ${e.outputSummary ?? '?'})`)
      .join('\n')
    return [
      `You are generating a BuckyOS Workflow definition (DSL JSON).`,
      `schema_version: ${this.schemaVersion}`,
      ``,
      `Top-level fields: id, name, schema_version, trigger?, defs?, nodes, edges, guards?.`,
      ``,
      `Node kinds:`,
      `- TaskNode (Step) fields: id, name, type ("autonomous" | "human_confirm" | "human_required"), executor, input_schema, output_schema, output_mode ("single" | "finite_seekable" | "finite_sequential"), idempotent, skippable, subject_ref?, prompt?, guards?.`,
      `- ControlNode types:`,
      `  * branch: { on, paths, max_iterations? }  // paths is exhaustive enum -> nodeId map`,
      `  * parallel: { branches[], join: { strategy: "all" | "any" | "n_of_m", n? } }`,
      `  * for_each: { items, steps[], max_items, concurrency }  // upstream finite_sequential forces concurrency=1`,
      ``,
      `Edges: { from, to }. branch.paths and parallel.branches imply edges; do not duplicate them.`,
      ``,
      `References (in input bindings) use:  \${node_id.output[.field[.sub]]}`,
      `No expressions, function calls, or string concatenation in references.`,
      ``,
      `Available executors in this zone:`,
      exec,
      ``,
      `Output requirements: emit ONE valid JSON object (no commentary, no markdown fences). Conform exactly to schema_version ${this.schemaVersion}.`,
    ].join('\n')
  }
}
