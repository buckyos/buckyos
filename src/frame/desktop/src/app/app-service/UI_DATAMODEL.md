# App Service UI DataModel

## Overview

This document records the UI-facing model proven by the mock-first App Service prototype.

- Product sources: `product/app_service/BuckyOS_App_Service_PRD.md` v0.3 and `product/BuckyOS Sys Dlg.md`
- Prototype entries: `AppServiceAppPanel.tsx` and `/sysdlg/app_installer`
- Views: runtime home, application detail, Stage 1 source entry, shared/direct App Installer Verify/Plan/Progress/Result views
- Data source: `mock/store.ts`; the prototype has no backend dependency
- Input schemas: `schemas.ts`

The UI model deliberately does not mirror a backend DTO. Installer source normalization, trust evidence, the stable install plan, task progress, installation result, and application runtime status remain separate concepts.

## DataModel definitions

### Runtime objects

```ts
type AppServiceStatus =
  | 'running'
  | 'starting'
  | 'stopped'
  | 'error'
  | 'installing'
  | 'activation_failed'

type ServiceLayer = 'app' | 'system' | 'kernel'

interface AppServiceItem {
  id: string
  name: string
  description: string
  iconKey: string
  version: string
  layer: ServiceLayer
  status: AppServiceStatus
  docker: DockerDependency | null
  diagnostics: string[]
  spec: Record<string, string>
  settings: Record<string, string>
  serviceInfo: Record<string, string>
  logs: string[]
  installProgress?: number
  installTaskId?: string
}

interface DockerDependency {
  engine: 'running' | 'not_running'
  image: 'present' | 'missing' | 'pulling'
  imageName: string
  container: 'running' | 'stopped' | 'error' | 'not_created'
}
```

`activation_failed` means installation succeeded but automatic startup or the health check failed. It must not be collapsed into installation failure.

### Source normalization

```ts
type InstallSourceKind =
  | 'url-app-meta'
  | 'url-pikg'
  | 'app-did'
  | 'app-name'
  | 'app-document-object'
  | 'pikg-object'
  | 'share-object'
  | 'signed-jwt'
  | 'unsigned-json'
  | 'local-pikg'
  | 'personal-server-pikg'

interface InstallSourceResolution {
  kind: InstallSourceKind
  originalInput: string
  displaySource: string
  normalizedType: 'identifier' | 'staging_handle'
  normalizedValue: string
  fileName?: string
  warningCode?: 'UNSIGNED_CANDIDATE'
}
```

The Stage 1 page only accepts raw input or a lightweight selected-file reference. Local and Personal Server files become a controlled `staging_handle`; server paths and file bytes are not exposed to the page model.

### Verify and trust

```ts
type TrustCheckStatus = 'verified' | 'warning' | 'pending' | 'failed' | 'unknown'

interface TrustCheck {
  code: 'document' | 'signature' | 'owner' | 'authority'
  status: TrustCheckStatus
  detail: string
}

interface InstallAppInfo {
  id: string
  name: string
  version: string
  releaseVersion: string
  description: string
  iconKey: string
  appDid: string
  documentObjectId: string
  publisher: string
  referrer: string
  source: InstallSourceResolution
  trustChecks: TrustCheck[]
  platformSupported: boolean
  content: ContentReadiness
  permissions: InstallPermission[]
  availableComponents: string[]
  defaultSettings: Record<string, string>
  installReady: boolean
  blockingReason?:
    | 'TRUST_RESOLUTION_REQUIRED'
    | 'IDENTITY_REVOKED'
    | 'TARGET_UNSUPPORTED'
    | 'OFFLINE_CONTENT_UNAVAILABLE'
}
```

Document format, signature, owner constraint, and authoritative publication are independent checks. A valid parse never implies trust. `TRUST_RESOLUTION_REQUIRED` is a trust-readiness state, not content download. `OFFLINE_CONTENT_UNAVAILABLE` prevents Acquisition when a caller explicitly forbids network access.

### Install plan

```ts
interface InstallOptions {
  targetNode: 'ood-primary' | 'ood-backup'
  components: string[]
  dataDir: string
  networkMode: 'private' | 'zone'
  autoStart: boolean
}

interface InstallPlan {
  options: InstallOptions
  permissions: InstallPermission[]
  impacts: Array<'container' | 'persistent-data' | 'network-route'>
  content: ContentReadiness
  ready: boolean
}

interface ContentReadiness {
  offlineReady: boolean
  missingBytes: number
  availableSource: string
}
```

The plan is recalculated whenever the target node or installation options change. The administrator password is intentionally absent from `InstallPlan` and `InstallTask`.

### Install task

```ts
type InstallTaskStage =
  | 'resolve'
  | 'inspect'
  | 'acquire'
  | 'verify'
  | 'prepare'
  | 'deploy'
  | 'activate'
  | 'completed'

type InstallTaskStatus =
  | 'waiting_for_approval'
  | 'running'
  | 'completed'
  | 'failed'

interface InstallTask {
  taskId: string
  app: InstallAppInfo
  plan: InstallPlan
  stage: InstallTaskStage
  status: InstallTaskStatus
  progress: number | null
  summary: string
  currentResource?: string
  history: InstallTaskHistoryItem[]
  launchRequest?: InstallLaunchRequest
  failure?: InstallFailure
  result?: InstallResult
}

interface InstallLaunchRequest {
  identifier: string
  referrer?: string
  targetNode?: 'ood-primary' | 'ood-backup'
  offline: boolean
  installParams?: Record<string, unknown>
}

interface InstallResult {
  installed: boolean
  installedVersion: string
  targetNode: 'ood-primary' | 'ood-backup'
  autoStart: 'running' | 'failed' | 'skipped'
}
```

One positive i64 decimal `taskId` identifies Resolve through Activate. Closing the Installer preserves the active task, and reopening from the home banner or direct URL restores it. Retry keeps the same task ID. `launchRequest` is the normalized, reviewable snapshot of caller suggestions; it is not user approval.

## Input models and validation

The schemas in `schemas.ts` are the single source of truth for forms.

```ts
const manualInstallSourceSchema = z.object({
  sourceText: z.string().trim().min(1).max(32768),
})

const settingsInputSchema = z.record(
  z.string(),
  z.string().trim().min(1).max(256),
)

const installerApprovalSchema = z.object({
  targetNode: z.enum(['ood-primary', 'ood-backup']),
  components: z.array(z.string()).min(1),
  dataDir: z.string().trim().min(1).max(256).regex(/^\//),
  networkMode: z.enum(['private', 'zone']),
  autoStart: z.boolean(),
  password: z.string().min(1).max(128),
})
```

The public dialog launch union is validated in `sysdlg/AppInstaller.tsx` before task lookup or creation:

```ts
type AppInstallerLaunchParams =
  | { task_id: string }
  | {
      identifier: string
      ref?: string
      options?: {
        target?: { node_did?: string; node_id?: string }
        install_params?: Record<string, unknown>
        offline?: boolean
      }
    }
```

The two forms are exclusive. URL parameters reject unknown keys and duplicates; `options` is parsed as once-decoded strict JSON with a 16 KiB UI limit. `identifier` is limited to 32 KiB and `ref` to 2 KiB. Target identifiers must resolve to the same known node.

Valid examples:

- Source: `https://apps.buckyos.ai/nextcloud/app-meta.jwt`
- Source: `did:cyfs:app-nextcloud-7w4k2n`
- Data directory: `/data/nextcloud`
- Approval: at least one component and a non-empty password

Invalid examples:

- Empty or unsupported identifier text: `source_unrecognized`
- Repeated, unknown, or conflicting URL parameters: visible launch error, no task created
- Invalid or out-of-range task ID: visible launch error, no lookup performed
- JSON without a DID or supported `doc_type`: `INVALID_APP_META`
- File without a `.pikg` suffix: `INVALID_PIKG`
- Relative data directory: visible `dataDirectoryError`
- Empty component list or password: visible localized form errors

## State definitions

### Home

| State | UI treatment |
|---|---|
| Loading | Application and service skeletons |
| Normal | Applications, System Services, and Kernel layers |
| Empty | Teaching empty state plus Add app action; system/kernel remain visible |
| Error | Human-readable load failure and Retry |
| Progress | Recoverable active-task banner and installing app card |

Mock URL scenarios are `?appServiceScenario=empty` and `?appServiceScenario=error`. The normal scenario briefly renders Loading before Ready.

### Detail

| State | UI treatment |
|---|---|
| Running | Start disabled, Stop enabled, healthy dependency chain |
| Stopped | Start enabled, Stop disabled |
| Starting | Immediate feedback with progress indicator |
| Error | Diagnosis, failed Docker dependency, and runtime log entry |
| Installed/start failed | Installation preserved; Start and diagnosis available |

### Stage 1 source entry

| State | UI treatment |
|---|---|
| Idle | Next disabled |
| Analyzing | Source identification status |
| Valid | Identified source kind and normalized input type; Next enabled |
| Warning | Unsigned JSON candidate warning; Installer performs trust checks |
| Invalid | Specific parse category and correction guidance; Next disabled |

### App Installer

| State | UI treatment |
|---|---|
| Launch resolving | Source-normalization spinner before task creation |
| Invalid launch | Localized parameter/source error and Close action |
| Task missing | Recoverable task-not-found state; no replacement task is created |
| Inspect/waiting | App identity, source, trust, platform, and content readiness |
| Blocked | Named blocking reason and disabled plan action |
| Approval | Recomputed plan, permissions, impact, and administrator authorization |
| Running | Stage history, progress, current resource, and background action |
| Failure | Human-readable category, safe details, retry, modify, and source actions |
| Completed | Installed result and independent automatic-start result |

## Pagination and aggregation

The current PRD requires a complete installed-app runtime view, so the prototype does not paginate. Objects are grouped by `layer`, with applications first, system services second, and kernel last. A future backend integration may add cursor pagination only if a realistic Zone size requires it; the three-layer order and active-task banner must remain stable.

Derived fields:

- Home status badge derives from `AppServiceItem.status`.
- Docker diagnosis derives from engine, image, and container state without overwriting application status.
- `InstallPlan.content` derives from source readiness, target node, and options.
- Result presentation derives from both `InstallResult.installed` and `InstallResult.autoStart`.

## Field stability

| Field | Stability | Notes |
|---|---|---|
| `AppServiceItem.id` | Frozen | Detail navigation and task-to-app correlation |
| `AppServiceItem.layer` | Frozen | Three-level home information architecture |
| `AppServiceItem.status` | Frozen | Shared product status vocabulary |
| `InstallSourceResolution.normalizedType` | Frozen | Installer entry contract |
| `InstallSourceResolution.originalInput` | Frozen | Source explanation and retry |
| `InstallAppInfo.appDid` | Frozen | Application identity |
| `InstallAppInfo.documentObjectId` | Frozen | Candidate/authority comparison |
| `TrustCheck.code` | Frozen | Trust must remain decomposed |
| `InstallTask.taskId` | Frozen | Recoverable transaction identity |
| `InstallTask.launchRequest` | Frozen | Normalized public-launch suggestion snapshot |
| `InstallTask.stage` | Frozen | Resolve-to-Activate progress mapping |
| `InstallPlan.impacts` | Extensible | New technical-impact categories may be added |
| `AppServiceItem.serviceInfo` | Extensible | Layer-specific runtime metadata |
| Diagnostic text | Volatile | Backend error mapping and UX copy will evolve |
| Mock progress timings | Volatile | Prototype-only behavior |

## Mock data contract

The mock store provides:

- Normal runtime apps: running, stopped, error, installing, and installed/start-failed
- Docker boundaries: present, pulling, container error, and not-created
- Valid inputs: App Meta URL, PIKG URL, App DID, app name, App Document/PIKG Object ID, share object, signed JWT, unsigned JSON, local package, Personal Server package
- Invalid inputs: unsupported URL content, invalid JSON, invalid PIKG, and unrecognized text
- Trust boundary: include `trust-pending` in a recognized URL to produce `TRUST_RESOLUTION_REQUIRED`
- Revocation boundary: include `revoked` to produce `IDENTITY_REVOKED`
- Platform boundary: include `unsupported` to produce `TARGET_UNSUPPORTED`
- Offline boundary: direct-launch a network source with `options.offline=true` to produce `OFFLINE_CONTENT_UNAVAILABLE`
- Direct launch: `identifier` creates and persists a task, then normalizes browser history to `?task_id=<positive-i64>`
- Download failure: include `fail-download`; Retry resumes the same task ID and succeeds
- Activation failure: include `activation-fail`; installation completes with `autoStart: 'failed'`
- Empty/error home states: `appServiceScenario=empty|error`

## KRPC mapping notes

| UI field | Expected source | Transform |
|---|---|---|
| App list and service layers | `system-config` service/app records plus runtime `service_info` | Group by UI layer and map runtime status |
| Docker dependency | Node/app runtime inspection | Normalize engine/image/container states |
| Source resolution | App Service Stage 1 parser | Raw input to `identifier` or `staging_handle` |
| Public launch request | `sysdlg/app_installer` URL or Dialog SDK | Strict validation to task lookup or normalized `InstallLaunchRequest` |
| App identity and trust | Installer Resolve/Inspect | Preserve independent evidence checks |
| Install plan | Installer Inspect plus user options | Recompute after every option change |
| Task stage/progress | Task Manager by `task_id` | Protocol stage to localized product state |
| Failure | Structured Installer/task error | Category plus user explanation and safe details |
| Install result | Completed task result | Keep installed and auto-start outcomes separate |

Backend integration must not send the administrator password through task storage, task logs, diagnostics, or error details.
