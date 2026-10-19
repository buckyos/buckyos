/* ── My Content – app panel entry point ── */

import type { AppContentLoaderProps } from '../types'
import { MyContentView } from './MyContentView'
import { myContentLaunchSchema } from './launch'

export function MyContentAppPanel({ launch }: AppContentLoaderProps) {
  const parsed = myContentLaunchSchema.safeParse(launch?.payload)
  return <MyContentView key={launch?.requestId ?? 'default'} launch={parsed.success ? parsed.data : undefined} />
}
