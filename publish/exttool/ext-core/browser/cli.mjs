import { parseArgs } from 'node:util';

// A failure with the exit code it stands for. Its message is the last line on stderr.
export class CliError extends Error {
  constructor(message, exitCode = 1) {
    super(message);
    this.exitCode = exitCode;
  }
}

export const EXIT_USAGE = 2;

export function usage(message) {
  return new CliError(message, EXIT_USAGE);
}

// `options` as for util.parseArgs. Returns { values, positionals }.
export function parse(args, options) {
  try {
    return parseArgs({ args, options: { help: { type: 'boolean', short: 'h' }, ...options }, allowPositionals: true, strict: true });
  } catch (e) {
    throw usage(e.message);
  }
}

export function number(text, name, { min = 0, integer = false } = {}) {
  const value = Number(text);
  if (text === '' || !Number.isFinite(value) || value < min || (integer && !Number.isInteger(value))) {
    throw usage(`${name}: not a valid number: ${text}`);
  }
  return value;
}

function flush(stream) {
  return new Promise((resolve) => stream.write('', resolve));
}

// Runs a command to its exit code. Pipes are written asynchronously, so both streams are drained before exiting.
export async function main(name, run) {
  let code = 0;
  try {
    code = (await run()) ?? 0;
  } catch (e) {
    const expected = e instanceof CliError;
    code = expected ? e.exitCode : 1;
    if (!expected && e?.stack) process.stderr.write(`${e.stack}\n`);
    const hint = code === EXIT_USAGE ? ` (see ${name} --help)` : '';
    process.stderr.write(`${name}: ${expected ? e.message : String(e?.message ?? e).split('\n')[0]}${hint}\n`);
  }
  await flush(process.stdout);
  await flush(process.stderr);
  process.exit(code);
}
