// Source packages and templates, on disk and in the bucket: Personal Site
// Protocol §4 (`_src/<source>/<version>/`) and §6.1 (`_app/<version>/`,
// `index.html`).

import { EXTRACT_FILE } from "./extract.ts";
import { invalid, SiteBucket, sourceDefaults, sourceDefProblems } from "./store.ts";
import { feedKey, INDEX_KEY, SiteError, sourcePrefix, TEMPLATE_SCHEMA, templatePrefix } from "./types.ts";
import type { Site, SourceDef, SourceEntry, SourceState } from "./types.ts";

export const SOURCE_FILE = "source.json";
export const TEMPLATE_FILE = "template.json";
export const TEMPLATE_ENTRY = "app.js";
/** A template's own tests live beside it and are not installed. */
const TEMPLATE_TESTS = "test/";
const MAX_PACKAGE_FILES = 200;
const MAX_PACKAGE_BYTES = 20 * 1024 * 1024;
const KEPT_SOURCE_VERSIONS = 3;

// The files of a package directory, relative, `/`-separated, sorted; hidden entries are left out.
async function filesOf(dir: string): Promise<{ files: string[]; bytes: number }> {
  const files: string[] = [];
  const walk = async (path: string, prefix: string) => {
    for await (const entry of Deno.readDir(path)) {
      if (entry.name.startsWith(".")) continue;
      const relative = `${prefix}${entry.name}`;
      if (entry.isSymlink) {
        throw new SiteError("invalid", `${relative} is a symbolic link; a package holds plain files`);
      }
      if (entry.isDirectory) await walk(`${path}/${entry.name}`, `${relative}/`);
      else files.push(relative);
    }
  };
  await walk(dir, "");
  files.sort();
  let bytes = 0;
  for (const file of files) bytes += (await Deno.stat(`${dir}/${file}`)).size;
  return { files, bytes };
}

/** The checked source package in `dir`: its declaration and its files (hidden ones left out). */
export async function readSourcePackage(dir: string): Promise<{ def: SourceDef; files: string[] }> {
  const { files, bytes } = await filesOf(dir);
  const problems: string[] = [];
  let def: unknown;
  if (!files.includes(SOURCE_FILE)) {
    problems.push(`${SOURCE_FILE} is missing`);
  } else {
    try {
      def = JSON.parse(await Deno.readTextFile(`${dir}/${SOURCE_FILE}`));
      problems.push(...sourceDefProblems(def));
    } catch (err) {
      problems.push(`${SOURCE_FILE} is not JSON: ${(err as Error).message}`);
    }
  }
  if (!files.includes(EXTRACT_FILE)) problems.push(`${EXTRACT_FILE} is missing`);
  if (files.length > MAX_PACKAGE_FILES) {
    problems.push(`a source package has at most ${MAX_PACKAGE_FILES} files, this one has ${files.length}`);
  }
  if (bytes > MAX_PACKAGE_BYTES) {
    problems.push(`a source package is at most ${MAX_PACKAGE_BYTES} bytes, this one is ${bytes}`);
  }
  if (problems.length) throw invalid(`the source package in ${dir}`, problems);
  return { def: def as SourceDef, files };
}

function nextVersion(letter: string, existing: string[]): string {
  const numbers = existing.flatMap((name) => new RegExp(`^${letter}([1-9]\\d*)$`).exec(name)?.[1] ?? [])
    .map(Number);
  return `${letter}${Math.max(0, ...numbers) + 1}`;
}

const byNumber = (a: string, b: string) => Number(a.slice(1)) - Number(b.slice(1));

/** The entry of a source in site.json as its package declares it. */
export function sourceEntry(def: SourceDef, version: string | null, addedBy: string, now: number): SourceEntry {
  const defaults = sourceDefaults(def.kind, def.needs);
  return {
    id: def.id,
    title: def.title,
    home_url: def.home_url,
    match: def.match,
    lang: def.lang,
    kind: def.kind,
    needs: def.needs,
    job: `src-${def.id}`,
    version,
    every_secs: def.every_secs ?? defaults.every_secs,
    max_new: def.max_new ?? defaults.max_new,
    keep_days: def.keep_days ?? defaults.keep_days,
    comments: def.comments ?? defaults.comments,
    state: "preparing",
    note: null,
    added_at: now,
    added_by: addedBy,
  };
}

/**
 * Checks the package in `dir`, writes it as the next version of
 * `_src/<source>/` and updates the source's entry in site.json: what the
 * package declares replaces the entry's, except `title` and the four sync
 * parameters of a source that is there already — those are the owner's.
 */
export async function putSource(
  bucket: SiteBucket,
  dir: string,
  options: { state?: SourceState; note?: string; by: string; addedBy: string; now?: number },
): Promise<{ id: string; version: string; files: string[]; entry: SourceEntry }> {
  const { def, files } = await readSourcePackage(dir);
  await bucket.requireSite();
  const now = options.now ?? Date.now();
  const versions = (await bucket.folders(sourcePrefix(def.id))).filter((name) => /^s[1-9]\d*$/.test(name));
  const version = nextVersion("s", versions);
  for (const file of files) {
    await bucket.write(`${sourcePrefix(def.id, version)}${file}`, await Deno.readFile(`${dir}/${file}`));
  }
  let entry!: SourceEntry;
  await bucket.updateSite((site) => {
    const fresh = sourceEntry(def, version, options.addedBy, now);
    const at = site.sources.findIndex((source) => source.id === def.id);
    const old = at >= 0 ? site.sources[at] : undefined;
    entry = old
      ? {
        ...fresh,
        title: old.title,
        every_secs: old.every_secs,
        max_new: old.max_new,
        keep_days: old.keep_days,
        comments: old.comments,
        state: old.state,
        note: old.note,
        added_at: old.added_at,
        added_by: old.added_by,
      }
      : fresh;
    if (options.state) entry.state = options.state;
    if (options.note !== undefined) entry.note = options.note;
    else if (options.state && options.state !== "failed") entry.note = null;
    if (old) site.sources[at] = entry;
    else site.sources.push(entry);
  }, options.by, now);
  for (const old of versions.sort(byNumber).slice(0, -(KEPT_SOURCE_VERSIONS - 1))) {
    for (const key of await bucket.keys(sourcePrefix(def.id, old))) await bucket.remove(key);
  }
  return { id: def.id, version, files, entry };
}

async function isEmptyDir(dir: string): Promise<boolean> {
  try {
    for await (const _ of Deno.readDir(dir)) return false;
    return true;
  } catch (err) {
    if (err instanceof Deno.errors.NotFound) return true;
    throw err;
  }
}

/** Writes the current version of a source's package into `dir` (new or empty). */
export async function getSource(
  bucket: SiteBucket,
  site: Site,
  id: string,
  dir: string,
): Promise<{ version: string; files: string[] }> {
  const entry = site.sources.find((source) => source.id === id);
  if (!entry) throw new SiteError("invalid", `the site has no source ${id}`);
  if (!entry.version) throw new SiteError("invalid", `the source ${id} has no package in the bucket yet`);
  const prefix = sourcePrefix(id, entry.version);
  const keys = await bucket.keys(prefix);
  if (!keys.length) throw new SiteError("invalid", `${prefix} is empty: the package of ${id} is gone from the bucket`);
  if (!await isEmptyDir(dir)) throw new SiteError("invalid", `${dir} is not empty`);
  const files: string[] = [];
  for (const key of keys) {
    const file = key.slice(prefix.length);
    if (file.split("/").some((segment) => !segment || segment === "." || segment === "..")) {
      throw new SiteError("invalid", `the key ${key} is not a file path`);
    }
    const found = await bucket.read(key);
    if (!found) continue;
    const target = `${dir}/${file}`;
    await Deno.mkdir(target.slice(0, target.lastIndexOf("/")), { recursive: true });
    await Deno.writeFile(target, found.bytes);
    files.push(file);
  }
  return { version: entry.version, files };
}

/** Takes a source out of site.json and removes its package and its feed; its articles stay until they expire. */
export async function removeSource(bucket: SiteBucket, id: string, by: string): Promise<number> {
  let found = false;
  await bucket.updateSite((site) => {
    found = site.sources.some((source) => source.id === id);
    site.sources = site.sources.filter((source) => source.id !== id);
    for (const column of site.columns) column.sources = column.sources.filter((source) => source !== id);
  }, by);
  if (!found) throw new SiteError("invalid", `the site has no source ${id}`);
  const keys = [...await bucket.keys(sourcePrefix(id)), feedKey(id)];
  for (const key of keys) await bucket.remove(key);
  return keys.length;
}

/**
 * Installs the template in `dir` as the next `_app/t<n>/` (its files never
 * change afterwards) and, when the directory has one, the entry `index.html`
 * all versions share. site.json is the caller's to switch.
 */
export async function putTemplate(
  bucket: SiteBucket,
  dir: string,
  by: string,
  now: number = Date.now(),
): Promise<{ version: string; files: string[] }> {
  const files = (await filesOf(dir)).files.filter((file) => !file.startsWith(TEMPLATE_TESTS));
  const problems: string[] = [];
  if (!files.includes(TEMPLATE_ENTRY)) problems.push(`${TEMPLATE_ENTRY} is missing`);
  if (files.length > MAX_PACKAGE_FILES) {
    problems.push(`a template has at most ${MAX_PACKAGE_FILES} files, this one has ${files.length}`);
  }
  if (!files.includes(INDEX_KEY) && !await bucket.read(INDEX_KEY)) {
    problems.push(`${INDEX_KEY} is missing, and the site has no entry page yet`);
  }
  if (problems.length) throw invalid(`the template in ${dir}`, problems);
  const version = nextVersion("t", await bucket.folders("_app/"));
  const own = files.filter((file) => file !== INDEX_KEY && file !== TEMPLATE_FILE);
  for (const file of own) {
    await bucket.writeImmutable(`${templatePrefix(version)}${file}`, await Deno.readFile(`${dir}/${file}`));
  }
  await bucket.writeImmutable(
    `${templatePrefix(version)}${TEMPLATE_FILE}`,
    JSON.stringify({ schema: TEMPLATE_SCHEMA, version, updated_at: now, updated_by: by }, null, 2),
  );
  if (files.includes(INDEX_KEY)) await bucket.write(INDEX_KEY, await Deno.readFile(`${dir}/${INDEX_KEY}`));
  return { version, files: own };
}

export async function listTemplates(
  bucket: SiteBucket,
): Promise<{ version: string; updated_at: number | null; updated_by: string | null }[]> {
  const versions = (await bucket.folders("_app/")).filter((name) => /^t[1-9]\d*$/.test(name)).sort(byNumber);
  const out = [];
  for (const version of versions) {
    const found = await bucket.readJson(`${templatePrefix(version)}${TEMPLATE_FILE}`).catch(() => null);
    const info = (found?.value ?? {}) as { updated_at?: unknown; updated_by?: unknown };
    out.push({
      version,
      updated_at: typeof info.updated_at === "number" ? info.updated_at : null,
      updated_by: typeof info.updated_by === "string" ? info.updated_by : null,
    });
  }
  return out;
}

/**
 * Writes an installed template version into `dir` (new or empty), with the
 * entry `index.html` beside it: a directory `putTemplate` takes again.
 */
export async function getTemplate(
  bucket: SiteBucket,
  version: string,
  dir: string,
): Promise<{ version: string; files: string[] }> {
  const prefix = templatePrefix(version);
  const keys = /^t[1-9]\d*$/.test(version) ? await bucket.keys(prefix) : [];
  if (!keys.includes(`${prefix}${TEMPLATE_ENTRY}`)) {
    throw new SiteError("invalid", `no template version ${version} is installed (site template list shows them)`);
  }
  if (!await isEmptyDir(dir)) throw new SiteError("invalid", `${dir} is not empty`);
  const files: string[] = [];
  for (const key of [...keys.filter((key) => key !== `${prefix}${TEMPLATE_FILE}`), INDEX_KEY]) {
    const file = key === INDEX_KEY ? INDEX_KEY : key.slice(prefix.length);
    if (file.split("/").some((segment) => !segment || segment === "." || segment === "..")) {
      throw new SiteError("invalid", `the key ${key} is not a file path`);
    }
    const found = await bucket.read(key);
    if (!found) continue;
    const target = `${dir}/${file}`;
    await Deno.mkdir(target.slice(0, target.lastIndexOf("/")), { recursive: true });
    await Deno.writeFile(target, found.bytes);
    files.push(file);
  }
  return { version, files: files.sort() };
}

/** Switches the site to an installed template version. */
export async function useTemplate(bucket: SiteBucket, version: string, by: string): Promise<Site> {
  if (!/^t[1-9]\d*$/.test(version) || !await bucket.read(`${templatePrefix(version)}${TEMPLATE_ENTRY}`)) {
    throw new SiteError("invalid", `no template version ${version} is installed (site template list shows them)`);
  }
  return await bucket.updateSite((site) => {
    site.template.version = version;
  }, by);
}
