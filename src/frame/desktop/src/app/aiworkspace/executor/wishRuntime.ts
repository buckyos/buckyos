/* The runtime of a wish stage's model loop (AI Native 余下任务 A1): the stage's tools, as the service sent them in the
 * brief, become the page tools of the xllm site runtime `aiws.wish`. Every call goes back to the service
 * (`proc.tool`), except `run_program`, which runs the macro in this page's sandbox (programHost) and hands it in with
 * `proc.program`; the program never runs in the loop. An answer the service marks as an error is a failed call the
 * model sees. A call that could not reach the service at all (revoked, cancelled, offline, snapshot too large) is no
 * answer to show the model: `fatal` stops the run with it. */

import { createSiteRuntime, type JsonValue, type SiteRuntime, type SiteTool } from 'buckyos/xllm'
import type { Json, WishStageBrief, WishToolAnswer } from '../api/types'

export const WISH_SCENE = 'aiws.wish'

export type WishToolCall = (name: string, args: Record<string, Json>) => Promise<WishToolAnswer>

export function wishRuntime(tools: NonNullable<WishStageBrief['tools']>, call: WishToolCall, fatal: (error: unknown) => void): SiteRuntime {
  const site: Record<string, SiteTool> = {}
  for (const t of tools) {
    site[t.name] = {
      spec: { description: t.description, args_schema: t.parameters as JsonValue },
      call: async (args) => {
        let answer: WishToolAnswer
        try {
          answer = await call(t.name, args as Record<string, Json>)
        } catch (error) {
          fatal(error)
          throw error
        }
        if (answer.status === 'error') throw new Error(answer.text ?? '')
        return answer.text ?? ''
      },
    }
  }
  return createSiteRuntime({ scene: WISH_SCENE, origin: window.location.origin, hostname: window.location.hostname, tools: site })
}
