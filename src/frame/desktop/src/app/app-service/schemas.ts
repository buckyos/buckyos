import { z } from 'zod'
import type { AppDocumentView, InstallTarget } from './types'

export const hostnameDidSchema = z
  .string()
  .max(512)
  .regex(
    /^did:[a-z0-9]+:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)*$/,
  )
export const taskIdSchema = z
  .string()
  .min(1)
  .max(256)
  .regex(/^\S+$/)
  .refine((value) =>
    Array.from(value).every(
      (char) => char.charCodeAt(0) >= 32 && char.charCodeAt(0) !== 127,
    ),
  )
export const manualInstallSourceSchema = z
  .object({ sourceText: z.string().trim().min(1).max(32768) })
  .strict()
export type ManualInstallSourceInput = z.infer<typeof manualInstallSourceSchema>
const pathSchema = z
  .string()
  .min(1)
  .max(256)
  .regex(/^\//)
  .refine(
    (v) =>
      !v.split('/').includes('..') && !v.includes('\0') && !v.includes('\\'),
  )
const mountSchema = z
  .object({
    target_path: pathSchema,
    access: z.enum(['read_only', 'read_write', 'read_write_append']),
  })
  .strict()
const permissionSchema = z
  .object({
    scope_path: z.string().min(1).max(512),
    actions: z.array(z.string().min(1)).max(16),
    required: z.boolean(),
    exp: z.number().int().nonnegative().nullable(),
  })
  .strict()
const serviceSchema = z
  .object({
    enabled: z.boolean(),
    expose: z
      .object({
        route: z.discriminatedUnion('type', [
          z
            .object({
              type: z.literal('web'),
              sub_hostname: z.array(z.string()).max(0),
            })
            .strict(),
          z
            .object({
              type: z.literal('port'),
              expose_port: z.number().int().min(1).max(65535),
            })
            .strict(),
        ]),
        scope: z.enum(['', 'zone']),
        allow_guest: z.boolean(),
      })
      .strict(),
  })
  .strict()
export const installParamsSchema = z
  .object({
    selected_components: z.array(z.string().min(1)).min(1).max(32),
    permissions: z.array(permissionSchema).max(64),
    data_mount_points: z.record(pathSchema, mountSchema),
    local_cache_mount_points: z.record(pathSchema, mountSchema),
    external_mount_points: z.record(pathSchema, mountSchema),
    service_settings: z
      .object({ services: z.record(z.string().min(1), serviceSchema) })
      .strict(),
    bash_envs: z.record(
      z.string().regex(/^[A-Za-z_][A-Za-z0-9_]*$/),
      z.string().max(4096),
    ),
    auto_start: z.boolean(),
    expected_instance_count: z.literal(1),
  })
  .strict()
export const installerApprovalSchema = z
  .object({
    target_node_id: z.string().min(1).max(256),
    policy: z.enum(['NORMAL', 'LOCAL_DEVELOPER']),
    offline: z.boolean(),
    install_params: installParamsSchema,
  })
  .strict()
export type InstallInput = z.infer<typeof installerApprovalSchema>
export type InstallerApprovalInput = InstallInput

export function createInstallInputSchema(
  app: AppDocumentView,
  targets: InstallTarget[],
  localPikg: boolean,
) {
  return installerApprovalSchema.superRefine((input, ctx) => {
    const issue = (path: Array<string | number>, message: string) =>
      ctx.addIssue({ code: 'custom', path, message })
    const p = input.install_params
    if (!targets.some((node) => node.node_id === input.target_node_id))
      issue(['target_node_id'], 'UNKNOWN_TARGET')
    if (input.policy === 'LOCAL_DEVELOPER' && !localPikg)
      issue(['policy'], 'LOCAL_PIKG_REQUIRED')
    if (
      new Set(p.selected_components).size !== p.selected_components.length ||
      p.selected_components.some(
        (name) => !app.components.some((c) => c.name === name),
      ) ||
      app.components.some(
        (c) => c.required && !p.selected_components.includes(c.name),
      )
    )
      issue(['install_params', 'selected_components'], 'REQUIRED_COMPONENT')
    const permissionKey = (v: z.infer<typeof permissionSchema>) =>
      JSON.stringify([v.scope_path, v.actions, v.required, v.exp])
    if (
      new Set(p.permissions.map(permissionKey)).size !== p.permissions.length ||
      p.permissions.some(
        (v) =>
          !app.permissions.some(
            (declared) => permissionKey(v) === permissionKey(declared),
          ),
      ) ||
      app.permissions.some(
        (v) =>
          v.required &&
          !p.permissions.some(
            (selected) => permissionKey(v) === permissionKey(selected),
          ),
      )
    )
      issue(['install_params', 'permissions'], 'DECLARED_PERMISSION')
    const ports = new Set<number>()
    for (const [name, service] of Object.entries(p.service_settings.services)) {
      const endpoint = app.endpoints.find((e) => e.name === name)
      const path = ['install_params', 'service_settings', 'services', name]
      if (
        !endpoint ||
        (endpoint.required && !service.enabled) ||
        service.expose.route.type !== endpoint.route
      )
        issue(path, 'REQUIRED_ENDPOINT')
      if (service.enabled && service.expose.route.type === 'port') {
        if (ports.has(service.expose.route.expose_port))
          issue([...path, 'expose', 'route', 'expose_port'], 'PORT_CONFLICT')
        ports.add(service.expose.route.expose_port)
      }
      if (service.expose.allow_guest && service.expose.scope === 'zone')
        issue([...path, 'expose', 'allow_guest'], 'GUEST_SCOPE')
    }
    for (const endpoint of app.endpoints)
      if (!p.service_settings.services[endpoint.name])
        issue(['install_params', 'service_settings'], 'REQUIRED_ENDPOINT')
    const mountPaths = new Set<string>()
    for (const kind of ['data', 'local_cache', 'external'] as const) {
      const map = p[`${kind}_mount_points`]
      for (const [path, mount] of Object.entries(map)) {
        const declaration = app.mounts.find(
          (m) => m.kind === kind && m.path === path,
        )
        if (
          !declaration ||
          mountPaths.has(path) ||
          mount.target_path !== declaration.target_path ||
          mount.access !== declaration.access
        )
          issue(
            ['install_params', `${kind}_mount_points`, path],
            'DECLARED_MOUNT',
          )
        mountPaths.add(path)
      }
      for (const mount of app.mounts.filter(
        (m) => m.kind === kind && m.required,
      ))
        if (!map[mount.path])
          issue(
            ['install_params', `${kind}_mount_points`, mount.path],
            'REQUIRED_MOUNT',
          )
    }
    for (const name of Object.keys(p.bash_envs))
      if (
        name.startsWith('BUCKYOS_') ||
        !app.environment.some((e) => e.name === name && !e.system)
      )
        issue(['install_params', 'bash_envs', name], 'SYSTEM_ENV')
    for (const env of app.environment)
      if (env.required && !env.system && !p.bash_envs[env.name]?.trim())
        issue(['install_params', 'bash_envs', env.name], 'REQUIRED_ENV')
  })
}

const stringMapSchema = z.record(z.string(), z.string())
const subPackageSchema = z
  .object({
    pkg_id: z.string().min(1),
    pkg_objid: z.string().regex(/^pkg:[0-9a-f]{64}$/),
    docker_image_name: z.string().min(1).optional(),
    docker_image_digest: z
      .string()
      .regex(/^sha256:[0-9a-f]{64}$/)
      .optional(),
    source_url: z.string().min(1).optional(),
    selector: z
      .object({
        os: z.string().optional(),
        arch: z.string().optional(),
        min_kernel_version: z.string().optional(),
      })
      .strict()
      .optional(),
    required: z.boolean().optional(),
  })
  .strict()
const endpointTipSchema = z
  .object({
    protocol: z.enum(['http', 'https', 'tcp', 'udp']),
    inner_port: z.number().int().min(1).max(65535),
    required: z.boolean().optional(),
    description: stringMapSchema.optional(),
    expose: z
      .object({
        route: z.discriminatedUnion('type', [
          z.object({ type: z.literal('web') }).strict(),
          z
            .object({
              type: z.literal('port'),
              preferred_port: z.number().int().min(1).max(65535).optional(),
            })
            .strict(),
        ]),
        scope: z.string().optional(),
        allow_guest: z.boolean().optional(),
      })
      .strict()
      .optional(),
  })
  .strict()
const mountTipSchema = z
  .object({
    mount_point_name: z.string(),
    access: z.enum(['read_only', 'read_write', 'read_write_append']),
    reason: stringMapSchema,
  })
  .strict()
  .nullable()
export const appDocCandidateSchema = z
  .object({
    schema_version: z.literal(1),
    doc_type: z.literal('app'),
    did: hostnameDidSchema,
    name: z.string().min(1).optional(),
    copyright: z.string().optional(),
    tags: z.array(z.string()).min(1).optional(),
    categories: z.array(z.string()).min(1).optional(),
    base_on: z
      .string()
      .regex(/^[^:]+:[0-9a-f]+$/)
      .optional(),
    directory: z
      .record(z.string(), z.record(z.string(), z.unknown()))
      .optional(),
    references: z
      .record(z.string(), z.record(z.string(), z.unknown()))
      .optional(),
    version: z.string().min(1),
    version_tag: z.string().optional(),
    app_type: z.enum(['dapp', 'web', 'agent', 'service']),
    owner: hostnameDidSchema,
    controller: hostnameDidSchema,
    author: hostnameDidSchema,
    create_time: z.number().int().nonnegative(),
    last_update_time: z.number().int().nonnegative(),
    exp: z.number().int().positive(),
    pkg_list: z.record(z.string(), subPackageSchema),
    show_name: z.string().min(1),
    selector_type: z.string().min(1).max(128),
    presentation: z
      .object({
        title: stringMapSchema,
        summary: stringMapSchema,
        description: stringMapSchema,
        icons: stringMapSchema,
        links: stringMapSchema,
        license: z.string(),
      })
      .strict()
      .optional(),
    sdk_version: z.string().optional(),
    req_capbilities: z.record(z.string(), z.number().int()).optional(),
    permissions: z
      .array(permissionSchema.partial({ actions: true, exp: true }))
      .optional(),
    service_config_tips: z
      .object({
        service_endpoints: z.record(z.string(), endpointTipSchema).optional(),
        data_mount_points: z.record(z.string(), mountTipSchema).optional(),
        local_cache_mount_points: z
          .record(z.string(), mountTipSchema)
          .optional(),
        external_mount_points: z.record(z.string(), mountTipSchema).optional(),
        rdb_instances: z.record(z.string(), z.unknown()).optional(),
        instance_volume: z
          .object({
            mode: z.enum(['required', 'optional', 'disabled']).optional(),
            quota_mib: z.number().int().nonnegative().optional(),
            ephemeral_contents: z.array(z.string()).optional(),
          })
          .strict()
          .optional(),
        bash_envs: z
          .record(
            z.string(),
            z
              .object({
                required: z.boolean(),
                description: stringMapSchema.optional(),
              })
              .strict(),
          )
          .optional(),
        runtime_caps: stringMapSchema.optional(),
        container_param: z.string().optional(),
        start_param: z.string().optional(),
      })
      .catchall(z.unknown()),
  })
  .strict()
  .superRefine((doc, ctx) => {
    if (doc.app_type !== 'service' && Object.keys(doc.pkg_list).length === 0)
      ctx.addIssue({
        code: 'custom',
        path: ['pkg_list'],
        message: 'INVALID_APPDOC',
      })
  })
