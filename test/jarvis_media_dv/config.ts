export type TomlValue = string | number | boolean | string[];

export type FlatToml = Record<string, TomlValue>;

function stripComment(line: string): string {
  let quote = "";
  let escaped = false;
  for (let index = 0; index < line.length; index += 1) {
    const char = line[index];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (quote === '"' && char === "\\") {
      escaped = true;
      continue;
    }
    if (char === '"' || char === "'") {
      if (!quote) quote = char;
      else if (quote === char) quote = "";
      continue;
    }
    if (char === "#" && !quote) return line.slice(0, index);
  }
  return line;
}

function splitArray(raw: string): string[] {
  const values: string[] = [];
  let current = "";
  let quote = "";
  let escaped = false;
  for (const char of raw) {
    if (escaped) {
      current += char;
      escaped = false;
      continue;
    }
    if (quote === '"' && char === "\\") {
      current += char;
      escaped = true;
      continue;
    }
    if (char === '"' || char === "'") {
      current += char;
      if (!quote) quote = char;
      else if (quote === char) quote = "";
      continue;
    }
    if (char === "," && !quote) {
      values.push(current.trim());
      current = "";
      continue;
    }
    current += char;
  }
  if (current.trim()) values.push(current.trim());
  return values;
}

function parseString(raw: string, lineNumber: number): string {
  if (raw.startsWith('"')) {
    try {
      return JSON.parse(raw) as string;
    } catch {
      throw new Error(`invalid TOML string at line ${lineNumber}`);
    }
  }
  if (raw.startsWith("'") && raw.endsWith("'")) return raw.slice(1, -1);
  throw new Error(`expected quoted TOML string at line ${lineNumber}`);
}

function parseValue(raw: string, lineNumber: number): TomlValue {
  const value = raw.trim();
  if (!value) throw new Error(`missing TOML value at line ${lineNumber}`);
  if (value.startsWith('"') || value.startsWith("'")) {
    return parseString(value, lineNumber);
  }
  if (value === "true") return true;
  if (value === "false") return false;
  if (/^[+-]?\d+$/.test(value)) return Number(value);
  if (value.startsWith("[") && value.endsWith("]")) {
    const body = value.slice(1, -1).trim();
    if (!body) return [];
    return splitArray(body).map((item) => parseString(item, lineNumber));
  }
  throw new Error(`unsupported TOML value at line ${lineNumber}`);
}

export function parseToml(input: string): FlatToml {
  const result: FlatToml = {};
  let section = "";
  const lines = input.replace(/^\uFEFF/, "").split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    const lineNumber = index + 1;
    const line = stripComment(lines[index]).trim();
    if (!line) continue;
    const sectionMatch = /^\[([A-Za-z0-9_.-]+)\]$/.exec(line);
    if (sectionMatch) {
      section = sectionMatch[1];
      continue;
    }
    const assignment = /^([A-Za-z0-9_-]+)\s*=\s*(.+)$/.exec(line);
    if (!assignment) throw new Error(`invalid TOML syntax at line ${lineNumber}`);
    const key = section ? `${section}.${assignment[1]}` : assignment[1];
    result[key] = parseValue(assignment[2], lineNumber);
  }
  return result;
}

export function tomlString(config: FlatToml, key: string): string | undefined {
  const value = config[key];
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

export function tomlNumber(config: FlatToml, key: string): number | undefined {
  const value = config[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

export function tomlBoolean(config: FlatToml, key: string): boolean | undefined {
  const value = config[key];
  return typeof value === "boolean" ? value : undefined;
}

export function tomlStrings(config: FlatToml, key: string): string[] | undefined {
  const value = config[key];
  return Array.isArray(value) ? value : undefined;
}
