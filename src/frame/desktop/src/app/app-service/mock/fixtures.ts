import type {
  AppDocumentView,
  AppServiceItem,
  InstallTarget,
  MockScenario,
  RuntimeType,
  RuntimeView,
} from '../types'
import type { InstallInput } from '../schemas'

export const targets: InstallTarget[] = [
  {
    node_id: 'home-server',
    node_did: 'did:bns:home-server.alice',
    label: 'Personal Server',
    os: 'linux',
    arch: 'aarch64',
    online: true,
  },
  {
    node_id: 'studio-pc',
    node_did: 'did:bns:studio-pc.alice',
    label: 'Studio PC',
    os: 'linux',
    arch: 'x86_64',
    online: true,
  },
  {
    node_id: 'travel-node',
    node_did: 'did:bns:travel-node.alice',
    label: 'Travel Node',
    os: 'windows',
    arch: 'x86_64',
    online: false,
  },
]
export const scenarios: MockScenario[] = [
  'normal',
  'developer',
  'revoked',
  'migrated',
  'tombstoned',
  'trust-pending',
  'signature-fail',
  'owner-fail',
  'document-fail',
  'content-missing',
  'unsupported',
  'config-invalid',
  'fail-download',
  'fail-committed',
  'paused',
  'stale',
  'activation-fail',
  'offline-node',
  'runtime-timeout',
  'runtime-unknown',
  'stale-evidence',
  'expired-reference',
  'import-fail',
  'upgrade',
  'downgrade',
]
const products: Record<
  string,
  { name: string; type: RuntimeType; version: string; icon: string }
> = {
  nextcloud: {
    name: 'Nextcloud',
    type: 'docker',
    version: '28.0.2',
    icon: 'files',
  },
  paperless: {
    name: 'Paperless',
    type: 'docker',
    version: '2.9.0',
    icon: 'files',
  },
  'home-dashboard': {
    name: 'Home Dashboard',
    type: 'web',
    version: '0.8.4',
    icon: 'homestation',
  },
  'backup-script': {
    name: 'Backup Script',
    type: 'script',
    version: '1.0.0',
    icon: 'task-center',
  },
  opendan: {
    name: 'OpenDAN',
    type: 'agent',
    version: '2.2.0',
    icon: 'ai-center',
  },
  'nostr-relay': {
    name: 'Nostr Relay',
    type: 'docker',
    version: '1.2.0',
    icon: 'messagehub',
  },
  'home-assistant': {
    name: 'Home Assistant',
    type: 'docker',
    version: '2024.3.1',
    icon: 'homestation',
  },
}
export function fixtureFor(value: string): string | null {
  return (
    Object.keys(products).find(
      (key) =>
        value === key ||
        value.startsWith(`${key}.`) ||
        value.includes(`:${key}.`) ||
        value.startsWith(`${key}-`),
    ) ?? null
  )
}
export function appDocument(
  fixture: string,
  scenario: MockScenario = 'normal',
): AppDocumentView {
  const product = products[fixture] ?? products.nextcloud
  const version =
    scenario === 'upgrade'
      ? '29.0.0'
      : scenario === 'downgrade'
        ? '27.0.0'
        : product.version
  const objectByte =
    scenario === 'upgrade' ? 'b2' : scenario === 'downgrade' ? 'c3' : 'a1'
  return {
    schema_version: 1,
    doc_type: 'app',
    did: `did:bns:${fixture}.buckyos`,
    app_id: `${fixture}.buckyos.bns.did`,
    object_id: `appdoc:${Array.from(fixture, (c) =>
      c.charCodeAt(0).toString(16),
    )
      .join('')
      .padEnd(62, '0')
      .slice(0, 62)}${objectByte}`,
    show_name: product.name,
    version,
    description_key: `app22.description.${fixture}`,
    app_type:
      product.type === 'web'
        ? 'web'
        : product.type === 'agent'
          ? 'agent'
          : 'dapp',
    runtime_type: product.type,
    owner: 'did:bns:buckyos',
    controller: 'did:bns:buckyos',
    author: 'did:bns:buckyos',
    publisher: 'BuckyOS Community',
    iconKey: product.icon,
    components: [
      { name: 'main', required: true },
      ...(product.type === 'docker'
        ? [{ name: 'worker', required: false }]
        : []),
    ],
    permissions: [
      {
        scope_path: `user/home/${fixture}`,
        actions: ['read', 'write'],
        required: true,
        exp: null,
      },
      { scope_path: 'wan', actions: ['connect'], required: false, exp: null },
    ],
    endpoints:
      product.type === 'script'
        ? []
        : [
            {
              name: 'www',
              protocol: 'http',
              inner_port: 80,
              required: true,
              route: 'web',
            },
            {
              name: 'metrics',
              protocol: 'tcp',
              inner_port: 9090,
              required: false,
              route: 'port',
            },
          ],
    mounts: [
      {
        kind: 'data',
        path: '/app/data',
        target_path: `/data/${fixture}`,
        access: 'read_write',
        required: true,
      },
      {
        kind: 'local_cache',
        path: '/app/cache',
        target_path: `/cache/${fixture}`,
        access: 'read_write',
        required: true,
      },
      {
        kind: 'external',
        path: '/app/import',
        target_path: '/home/Documents',
        access: 'read_only',
        required: false,
      },
    ],
    environment: [
      {
        name: 'LANG',
        default: 'en_US.UTF-8',
        required: true,
        sensitive: false,
        system: false,
      },
      {
        name: 'API_KEY',
        default: '',
        required: false,
        sensitive: true,
        system: false,
      },
      {
        name: 'BUCKYOS_APP_INSTANCE_ID',
        default: '',
        required: true,
        sensitive: false,
        system: true,
      },
    ],
    start_param: product.type === 'script' ? 'backup.sh' : undefined,
    container_param:
      product.type === 'docker' || product.type === 'agent'
        ? '--init'
        : undefined,
  }
}
export function defaultInput(app: AppDocumentView): InstallInput {
  const mounts = (kind: 'data' | 'local_cache' | 'external') =>
    Object.fromEntries(
      app.mounts
        .filter((m) => m.kind === kind && m.required)
        .map((m) => [m.path, { target_path: m.target_path, access: m.access }]),
    )
  return {
    target_node_id: targets[0].node_id,
    policy: 'NORMAL',
    offline: false,
    install_params: {
      selected_components: app.components
        .filter((c) => c.required)
        .map((c) => c.name),
      permissions: app.permissions.filter((p) => p.required),
      data_mount_points: mounts('data'),
      local_cache_mount_points: mounts('local_cache'),
      external_mount_points: mounts('external'),
      service_settings: {
        services: Object.fromEntries(
          app.endpoints.map((e) => [
            e.name,
            {
              enabled: e.required,
              expose: {
                route:
                  e.route === 'web'
                    ? { type: 'web', sub_hostname: [] }
                    : { type: 'port', expose_port: e.inner_port },
                scope: 'zone',
                allow_guest: false,
              },
            },
          ]),
        ),
      },
      bash_envs: Object.fromEntries(
        app.environment
          .filter((e) => !e.system)
          .map((e) => [e.name, e.default]),
      ),
      auto_start: true,
      expected_instance_count: 1,
    },
  }
}
export function emptyRuntime(
  type: RuntimeType,
  status: RuntimeView['status'] = 'unknown',
): RuntimeView {
  return {
    type,
    status,
    expected_deployment: null,
    evidence: null,
    docker: null,
    process_health: 'UNKNOWN',
    web_accessible: 'UNKNOWN',
    environment_ready: 'UNKNOWN',
    agent_bindings: type === 'agent' ? 0 : null,
    reason: null,
  }
}
export function seedServices(): AppServiceItem[] {
  const apps = [
    'nostr-relay',
    'home-assistant',
    'home-dashboard',
    'backup-script',
    'opendan',
  ].map((name): AppServiceItem => {
    const app = appDocument(name)
    const id = `${app.app_id}@alice`
    const runtime = emptyRuntime(
      app.runtime_type,
      name === 'home-assistant' ? 'stopped' : 'running',
    )
    runtime.expected_deployment = {
      app_instance_id: id,
      task_id: 't-seed',
      app_doc_object_id: app.object_id,
      spec_generation: 1,
    }
    runtime.evidence = {
      deployment: runtime.expected_deployment,
      observed_at: Date.now(),
      valid_until: Date.now() + 3_600_000,
    }
    if (['docker', 'agent'].includes(runtime.type))
      runtime.docker = {
        engine: 'running',
        image: 'present',
        imageName: `${name}:${app.version}`,
        container: runtime.status === 'running' ? 'running' : 'stopped',
      }
    runtime.process_health = runtime.type === 'script' ? 'READY' : 'UNKNOWN'
    runtime.web_accessible = runtime.type === 'web' ? 'READY' : 'UNKNOWN'
    runtime.environment_ready = runtime.type === 'agent' ? 'READY' : 'UNKNOWN'
    const record = {
      app_instance_id: id,
      app_did: app.did,
      owner_user_id: 'alice',
      app_doc_object_id: app.object_id,
      task_id: null,
      version: app.version,
      state: 'installed' as const,
      deployment: runtime.expected_deployment,
      app_name: name,
      app_host_name: name,
      app_index: 1,
      address: `https://${name}.alice.buckyos.io`,
    }
    return {
      id,
      app_instance_id: id,
      system_service_id: null,
      owner_user_id: 'alice',
      app_did: app.did,
      name: app.show_name,
      description: app.description_key,
      iconKey: app.iconKey,
      version: app.version,
      layer: 'app',
      status: runtime.status,
      runtime,
      record,
      docker: runtime.docker,
      diagnostics: [],
      spec: { auto_start: 'true', expected_instance_count: '1' },
      settings: { language: 'en' },
      serviceInfo: { node: 'home-server', endpoint: record.address },
      logs: [],
      available_actions: runtime.status === 'running' ? ['stop'] : ['start'],
      operation: null,
      operation_error: null,
    }
  })
  const secondOwner = structuredClone(apps[0])
  secondOwner.id =
    secondOwner.app_instance_id = `${appDocument('nostr-relay').app_id}@bob`
  secondOwner.owner_user_id = 'bob'
  secondOwner.record = {
    ...secondOwner.record!,
    app_instance_id: secondOwner.id,
    owner_user_id: 'bob',
    address: 'https://nostr-relay-bob.alice.buckyos.io',
    deployment: null,
  }
  secondOwner.runtime = emptyRuntime('docker')
  secondOwner.status = 'unknown'
  secondOwner.docker = null
  secondOwner.available_actions = []
  secondOwner.serviceInfo = { node: 'studio-pc' }
  apps.push(secondOwner)
  for (const [name, label, layer] of [
    ['gateway', 'Zone Gateway', 'system'],
    ['scheduler', 'Scheduler', 'system'],
    ['verify-hub', 'Verify Hub', 'system'],
    ['node-daemon', 'Node Daemon', 'kernel'],
    ['kmsg', 'KMsg', 'kernel'],
  ] as const) {
    apps.push({
      id: name,
      app_instance_id: null,
      system_service_id: name,
      owner_user_id: null,
      app_did: null,
      name: label,
      description: `app22.description.${name}`,
      iconKey: 'settings',
      version: '2.2.0',
      layer,
      status: 'unknown',
      runtime: emptyRuntime('unknown'),
      record: null,
      docker: null,
      diagnostics: [],
      spec: {},
      settings: {},
      serviceInfo: { node: 'home-server' },
      logs: [],
      available_actions: [],
      operation: null,
      operation_error: null,
    })
  }
  return apps
}
