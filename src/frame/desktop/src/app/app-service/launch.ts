import { z } from 'zod'

/** Launch parameters of App Service: one service's details, or the install wizard. */
export const appServiceLaunchSchema = z.union([
  z.object({ page: z.literal('detail'), serviceId: z.string().min(1) }),
  z.object({ page: z.literal('install') }),
])
