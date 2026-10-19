import { z } from 'zod'

/** Launch parameters of My Content: open one share's details, or a tab. */
export const myContentLaunchSchema = z.object({
  shareId: z.string().min(1).max(64).optional(),
  tab: z.enum(['all', 'links', 'apps', 'public', 'sites']).optional(),
})

export type MyContentLaunch = z.infer<typeof myContentLaunchSchema>
