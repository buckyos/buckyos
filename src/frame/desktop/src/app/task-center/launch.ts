import { z } from 'zod'

/** Launch parameters of Task Center: the task it opens on. */
export const taskCenterLaunchSchema = z.object({ taskId: z.string().min(1) })
