import { ndm_proxy } from "buckyos";

import {
  ArgError,
  bailArgError,
  parseArgvOrExit,
  requireString,
} from "../lib/cli.ts";
import { resolveInputResource, saveResourceToPath } from "../lib/io.ts";
import { initRuntime } from "../lib/runtime.ts";
import {
  bailIoError,
  bailRuntimeError,
  emitAndExit,
  errorResult,
  EXIT_ARG_ERROR,
  EXIT_SUCCESS,
  successResult,
} from "../lib/result.ts";

const TOOL = "materialize_resource";

export const HELP =
  `Usage: materialize_resource <resource> <output_path> [options]

Resource forms:
  named_object:<typed_object_id>
  cyfile:<object_id>
  chunk:<object_id>
  http(s)://...
  data:...

Options:
  --mime <mime_type>  MIME hint for URL resources`;

export async function run(argv: string[]): Promise<never> {
  const parsed = parseArgvOrExit(TOOL, HELP, argv);
  if (parsed.positional.length !== 2) {
    emitAndExit(
      errorResult(TOOL, `${TOOL} => arg_error`, HELP, {
        error: "expected <resource> and <output_path>",
      }),
      EXIT_ARG_ERROR,
    );
  }

  const [input, outputPath] = parsed.positional;
  let mimeHint: string | undefined;
  try {
    mimeHint = requireString(parsed.flags, "mime");
    const unsupported = [...parsed.flags.keys()].filter((name) =>
      name !== "mime"
    );
    if (unsupported.length > 0) {
      throw new ArgError(`unsupported option: --${unsupported[0]}`);
    }
    if (!input || !outputPath) {
      throw new ArgError("resource and output path are required");
    }
  } catch (err) {
    if (err instanceof ArgError) bailArgError(TOOL, err);
    throw err;
  }

  try {
    await initRuntime();
  } catch (err) {
    bailRuntimeError(TOOL, err);
  }

  try {
    const resource = await resolveInputResource(input, mimeHint);
    // deno-lint-ignore no-explicit-any
    const ndmProxy = (ndm_proxy as any).createNdmProxyClient();
    const saved = await saveResourceToPath(resource, outputPath, ndmProxy, {
      overwrite: false,
    });
    const absolutePath = await Deno.realPath(saved.path);
    emitAndExit(
      successResult(
        TOOL,
        `${TOOL} => done`,
        `${TOOL} wrote ${absolutePath}`,
        {
          path: absolutePath,
          bytes: saved.bytes,
          mime: saved.mime ?? null,
          source_kind: saved.source_kind,
        },
        absolutePath,
      ),
      EXIT_SUCCESS,
    );
  } catch (err) {
    bailIoError(TOOL, undefined, err);
  }
}

if (import.meta.main) {
  await run(Deno.args);
}
