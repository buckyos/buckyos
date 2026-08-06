import { z } from 'zod'

export const manualInstallSourceSchema = z.object({
  sourceText: z.string().trim().min(1).max(32768),
})

export type ManualInstallSourceInput = z.infer<typeof manualInstallSourceSchema>

export const settingsInputSchema = z.record(
  z.string(),
  z.string().trim().min(1).max(256),
)

export type SettingsInput = z.infer<typeof settingsInputSchema>

const absolutePathSchema = z.string().trim().min(1).max(256).regex(/^\//)

const serviceExposeSettingSchema = z.object({
  route: z.discriminatedUnion('type', [
    z.object({
      type: z.literal('web'),
      subHostname: z.array(z.string().trim().min(1).max(63)),
      exposeUri: z.string().trim().max(256).optional(),
    }),
    z.object({
      type: z.literal('port'),
      exposePort: z.number().int().min(1).max(65535),
    }),
  ]),
  scope: z.string().trim().max(256),
  allowGuest: z.boolean(),
})

export const installerApprovalSchema = z.object({
  targetNode: z.enum(['ood-primary', 'ood-backup']),
  components: z.array(z.string()).min(1),
  shortcutDomain: z.string().trim().max(63),
  serviceSettings: z.array(z.object({
    serviceName: z.string().trim().min(1).max(128),
    label: z.string().trim().min(1).max(128),
    protocol: z.enum(['http', 'https', 'tcp', 'udp']),
    innerPort: z.number().int().min(1).max(65535),
    enabled: z.boolean(),
    expose: serviceExposeSettingSchema,
  })),
  mounts: z.array(z.object({
    name: z.string().trim().min(1).max(128),
    containerPath: absolutePathSchema,
    targetPath: absolutePathSchema,
    access: z.enum(['read_only', 'read_write', 'read_write_append']),
    enabled: z.boolean(),
    declared: z.boolean(),
  })),
  envVars: z.array(z.object({
    name: z.string().trim().min(1).max(128).regex(/^[A-Za-z_][A-Za-z0-9_]*$/),
    value: z.string().max(4096),
    description: z.string().max(512),
    required: z.boolean(),
    declared: z.boolean(),
  })),
  permissionGrants: z.array(z.object({
    scope: z.string().trim().min(1).max(512),
    grant: z.enum(['default', 'allow', 'deny', 'read-only', 'read-write', 'zone-only', 'full-network']),
  })),
  autoStart: z.boolean(),
})

export type InstallerApprovalInput = z.infer<typeof installerApprovalSchema>

export const installerSudoSchema = z.object({
  password: z.string().min(1).max(128),
})

export type InstallerSudoInput = z.infer<typeof installerSudoSchema>
