// Tiny CLI argument parser. Each command parses its own positionals, then
// hands the leftover flags here for the shared options doc'd in §2.2 of
// aicc_agent_cli_tools.md.

import { Profile } from "./types.ts";

export interface CommonFlags {
  model?: string;
  profile?: Profile;
  maxCostUsd?: number;
  maxLatencyMs?: number;
  noFallback: boolean;
  idempotencyKey?: string;
  traceId?: string;
  json: boolean;
  timeoutMs?: number;
}

export interface ParsedArgs {
  positional: string[];
  flags: Map<string, string | true>;
  common: CommonFlags;
}

const COMMON_FLAGS = new Set([
  "model",
  "profile",
  "max-cost",
  "max-latency-ms",
  "no-fallback",
  "idempotency-key",
  "trace-id",
  "json",
  "timeout",
]);

const BOOLEAN_FLAGS = new Set(["no-fallback", "json"]);

function parseProfile(value: string): Profile {
  const v = value.toLowerCase();
  if (v === "cheap" || v === "fast" || v === "balanced" || v === "quality") return v;
  throw new ArgError(`--profile must be cheap|fast|balanced|quality, got ${value}`);
}

export const COMMON_OPTIONS_HELP = `
Common options:
  --model <alias>                 logical model name or exact model id
  --profile <balanced|cheap|fast|quality>
  --max-cost <usd>                cost ceiling
  --max-latency-ms <ms>           latency ceiling
  --no-fallback                   disable allow_fallback + runtime_failover
  --idempotency-key <key>
  --trace-id <id>
  --timeout <seconds>             max wait for AICC task completion
  --json                          (informational; AgentToolResult JSON is always emitted)
`;

export class ArgError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ArgError";
  }
}

export class HelpRequested extends Error {
  constructor() {
    super("help");
    this.name = "HelpRequested";
  }
}

// Parses argv into positional + flag map. Flags are `--name [value]`. A flag
// that appears in BOOLEAN_FLAGS or has no following non-flag token is treated
// as a boolean switch.
export function parseArgs(argv: string[], booleanFlags: Set<string> = BOOLEAN_FLAGS): ParsedArgs {
  const positional: string[] = [];
  const flags = new Map<string, string | true>();
  let i = 0;
  while (i < argv.length) {
    const tok = argv[i];
    if (tok === "-h" || tok === "--help") {
      throw new HelpRequested();
    }
    if (tok === "--") {
      positional.push(...argv.slice(i + 1));
      break;
    }
    if (tok.startsWith("--")) {
      const name = tok.slice(2);
      if (booleanFlags.has(name)) {
        flags.set(name, true);
        i += 1;
        continue;
      }
      const next = argv[i + 1];
      if (typeof next === "string" && !next.startsWith("--")) {
        flags.set(name, next);
        i += 2;
      } else {
        // No value after — treat as boolean switch.
        flags.set(name, true);
        i += 1;
      }
      continue;
    }
    positional.push(tok);
    i += 1;
  }

  const common: CommonFlags = {
    noFallback: flags.get("no-fallback") === true,
    json: flags.get("json") === true,
  };
  const model = flags.get("model");
  if (typeof model === "string") common.model = model;
  const profile = flags.get("profile");
  if (typeof profile === "string") common.profile = parseProfile(profile);
  const maxCost = flags.get("max-cost");
  if (typeof maxCost === "string") {
    const n = Number(maxCost);
    if (!Number.isFinite(n) || n < 0) throw new ArgError(`--max-cost invalid: ${maxCost}`);
    common.maxCostUsd = n;
  }
  const maxLat = flags.get("max-latency-ms");
  if (typeof maxLat === "string") {
    const n = Number(maxLat);
    if (!Number.isFinite(n) || n < 0) throw new ArgError(`--max-latency-ms invalid: ${maxLat}`);
    common.maxLatencyMs = n;
  }
  const idem = flags.get("idempotency-key");
  if (typeof idem === "string") common.idempotencyKey = idem;
  const trace = flags.get("trace-id");
  if (typeof trace === "string") common.traceId = trace;
  const timeout = flags.get("timeout");
  if (typeof timeout === "string") {
    const n = Number(timeout);
    if (!Number.isFinite(n) || n <= 0) throw new ArgError(`--timeout invalid: ${timeout}`);
    common.timeoutMs = n * 1000;
  }

  return { positional, flags, common };
}

export function requireString(flags: Map<string, string | true>, name: string): string | undefined {
  const v = flags.get(name);
  if (typeof v === "string") return v;
  if (v === true) throw new ArgError(`--${name} requires a value`);
  return undefined;
}

export function flagInt(flags: Map<string, string | true>, name: string): number | undefined {
  const v = requireString(flags, name);
  if (v === undefined) return undefined;
  const n = Number(v);
  if (!Number.isFinite(n) || Math.floor(n) !== n) throw new ArgError(`--${name} must be integer, got ${v}`);
  return n;
}

export function flagFloat(flags: Map<string, string | true>, name: string): number | undefined {
  const v = requireString(flags, name);
  if (v === undefined) return undefined;
  const n = Number(v);
  if (!Number.isFinite(n)) throw new ArgError(`--${name} must be a number, got ${v}`);
  return n;
}

export function flagBool(flags: Map<string, string | true>, name: string): boolean {
  return flags.get(name) === true;
}

// Drop the known-common flags from the map so commands can detect leftovers.
export function consumeCommon(flags: Map<string, string | true>): Map<string, string | true> {
  const out = new Map(flags);
  for (const k of COMMON_FLAGS) out.delete(k);
  return out;
}

// Parse argv or bail with the right exit code. Handles `--help` (prints HELP
// to stderr, exits 0) and ArgError (emits an AgentToolResult error, exits 1).
// Most commands start with this one-liner instead of an explicit try/catch.
//
// Importing this from a new tool you write: it gives you the same shape of
// CLI surface as the built-in commands.
import {
  emitAndExit,
  errorResult,
  EXIT_ARG_ERROR,
  EXIT_SUCCESS,
} from "./result.ts";

export function parseArgvOrExit(tool: string, help: string, argv: string[]): ParsedArgs {
  try {
    return parseArgs(argv);
  } catch (err) {
    if (err instanceof HelpRequested) {
      console.error(help);
      Deno.exit(EXIT_SUCCESS);
    }
    const msg = err instanceof Error ? err.message : String(err);
    emitAndExit(
      errorResult(tool, `${tool} => arg_error`, msg, { error: msg }),
      EXIT_ARG_ERROR,
    );
  }
}

// Re-throw helper for the per-flag validation phase. Build your input_json
// inside a try {} block, catch ArgError, hand it here.
export function bailArgError(tool: string, err: Error | string): never {
  const msg = typeof err === "string" ? err : err.message;
  emitAndExit(errorResult(tool, `${tool} => arg_error`, msg, { error: msg }), EXIT_ARG_ERROR);
}
