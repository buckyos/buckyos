import { createContext } from 'react'
import { z } from 'zod'

/** Launch parameters of HomeStation: a new post pre-filled by the system share dialog (`sysdlg/share`). */
export const homeStationLaunchSchema = z.object({
  compose: z.object({
    shareId: z.string().min(1),
    attachments: z.array(z.object({ kind: z.enum(['image', 'video', 'audio']), name: z.string().min(1).max(200), object: z.string().min(1), status: z.literal('uploaded') })).max(9).optional(),
    link: z.object({ url: z.string().url(), title: z.string().min(1).max(120), summary: z.string().max(280).optional() }).optional(),
    text: z.string().max(2000).optional(),
  }),
})

export type HomeStationCompose = z.infer<typeof homeStationLaunchSchema>['compose']

/** The pending pre-filled post: full composers start from it until a post is published. */
export const ComposePrefillContext = createContext<{ peek: () => HomeStationCompose | null; clear: () => void }>({ peek: () => null, clear: () => {} })
