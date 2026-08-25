import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const launcherDir = dirname(fileURLToPath(import.meta.url))

export const DEFAULT_TOOL_PATH = join(launcherDir, 'buckyos')

/**
 * Resolve how to invoke `buckyos pikg ...` on the current platform.
 *
 * The `buckyos` wrapper is a POSIX shell script, so Windows cannot spawn it. There the Deno
 * bootstrap that backs `buckyos.cmd` is used instead; both reach the same main.ts with the same
 * permission set. Node cannot spawn a `.cmd` without a shell, so the bootstrap is invoked directly.
 */
export function resolvePikgCommand(args, toolPath = DEFAULT_TOOL_PATH) {
  if (process.platform !== 'win32') {
    return { command: toolPath, args }
  }
  return {
    command: 'deno',
    args: [
      'run',
      '--quiet',
      '--allow-env',
      '--allow-run=deno',
      join(dirname(toolPath), 'win_launcher.ts'),
      ...args,
    ],
  }
}
