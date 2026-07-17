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

export const installerApprovalSchema = z.object({
  targetNode: z.enum(['ood-primary', 'ood-backup']),
  components: z.array(z.string()).min(1),
  dataDir: z.string().trim().min(1).max(256).regex(/^\//),
  networkMode: z.enum(['private', 'zone']),
  autoStart: z.boolean(),
  password: z.string().min(1).max(128),
})

export type InstallerApprovalInput = z.infer<typeof installerApprovalSchema>
