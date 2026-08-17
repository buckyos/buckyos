#!/usr/bin/env -S deno run

import { BuckyOSToolApplication } from './core/app.ts'

if (import.meta.main) {
  Deno.exitCode = await new BuckyOSToolApplication().run(Deno.args)
}
