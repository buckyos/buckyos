import { z } from 'zod'
import type { AICenterPage } from './components/layout/nav'

const pages = ['home', 'providers', 'providers/add', 'models', 'routing', 'playground', 'usage', 'quota', 'visibility'] as const satisfies readonly AICenterPage[]

/** Launch parameters of AI Center: the page it opens on. */
export const aiCenterLaunchSchema = z.object({ page: z.enum(pages) })
