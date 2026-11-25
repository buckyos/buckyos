import { z } from 'zod'

/** Launch parameters of Workflow: one definition, one run, or the AI prompt. */
export const workflowLaunchSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('definition'), definitionId: z.string().min(1) }),
  z.object({ kind: z.literal('run'), runId: z.string().min(1) }),
  z.object({ kind: z.literal('ai_prompt') }),
])
