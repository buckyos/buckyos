// site — the personal reading site of the user this Agent works for.
// 配方骨架同 reflow.ts：站点在用户的桶 `user/control-panel/<user>/reflow` 里，
// 规则见 doc/opendan/protocol/Personal Site Protocol.md，读写和流水线在 lib/site/。

import { agentServiceAccess, callAgentService, serviceError } from "../lib/agent_service.ts";
import { bailArgError, consumeCommon, flagInt, parseArgvOrExit, requireString } from "../lib/cli.ts";
import { aiccModel, MeteredModel } from "../lib/model.ts";
import { contentTypeOf, errorCode, libraryBucketId } from "../lib/reflow.ts";
import { initRuntime } from "../lib/runtime.ts";
import type { AiccRuntime } from "../lib/runtime.ts";
import {
  emit,
  emitAndExit,
  errorResult,
  EXIT_AICC_FAILED,
  EXIT_ARG_ERROR,
  EXIT_IO_FAILED,
  EXIT_SUCCESS,
  successResult,
} from "../lib/result.ts";
import type { JsonValue } from "../lib/types.ts";
import { Net } from "../lib/site/extract.ts";
import { findResize } from "../lib/site/images.ts";
import { dirStore } from "../lib/site/local.ts";
import {
  getSource,
  getTemplate,
  listTemplates,
  putSource,
  putTemplate,
  readSourcePackage,
  removeSource,
  sourceEntry,
  useTemplate,
} from "../lib/site/packages.ts";
import { putSupplement, syncSource } from "../lib/site/pipeline.ts";
import type { SyncReport } from "../lib/site/pipeline.ts";
import { compactReading, recentReading } from "../lib/site/reading.ts";
import {
  articleAddress,
  displayState,
  emptyEvents,
  isSourceId,
  MAX_UPGRADES,
  SiteBucket,
  sourceOfUrl,
} from "../lib/site/store.ts";
import type { SiteStore } from "../lib/site/store.ts";
import { createTranslator } from "../lib/site/translate.ts";
import { describeVisits } from "../lib/site/visits.ts";
import type { VisitsAnswer } from "../lib/site/visits.ts";
import { SITE_KEY, SITE_SCHEMA, SiteError } from "../lib/site/types.ts";
import type { RawArticle, Site, SourceEntry, SourceState, Upgrade } from "../lib/site/types.ts";

const TOOL = "site";
const DEFAULT_TIME_LIMIT_S = 480;
const TRY_LIMIT = 3;
const FIXTURES_DIR = "fixtures";
const FIXTURES_INDEX = "index.json";
/** The default template, beside the commands. */
const TEMPLATE_DIR = decodeURIComponent(new URL("../site-template", import.meta.url).pathname);

export const HELP = `Usage: site <subcommand> [args]

The personal reading site of the user this Agent works for: the websites the
user reads, fetched by a program per source, translated into the site's
language and shown on https://reflow.<zone>/ whether or not anything is
running. Everything is in the user's bucket; the contract is
doc/opendan/protocol/Personal Site Protocol.md.

  site init [--title ..] [--language ..] [--topics a,b,c] [--owner-did <did>]
                                    make the site (site.json) and install the default template;
                                    a site that exists is left as it is
  site status [--json]              the site, every source with its state and last sync, open
                                    upgrade requests
  site match <url>                  an article of the site (and its route), the source the URL
                                    belongs to, or neither
  site config get [<path>]          read site.json or one field of it
  site config set <path> <json>     change one field: title, language, topics, columns,
                                    template.params.<name>, sources.<id>.<field>, reading.enabled,
                                    limits.<name>, maintainer
  site source put <dir> [--state preparing|active|failed] [--note ..]
                                    check the source package in <dir>, store it as the next
                                    version and update its entry in site.json
  site source list                  the sources and their package versions
  site source get <id> <dir>        copy the current package of a source into <dir> (new or empty)
  site source rm <id>               take a source out of the site and remove its package and feed
  site try <dir> [--limit 3] [--no-translate] [--fixtures] [--record] [--out <dir>]
                                    run the package without the bucket: extract, check, images
                                    and translation into a local directory, and say what the
                                    check found
  site sync <source> [--dir <package>] [--time-limit <seconds>] [--no-translate]
                                    one full sync of a source; without --dir the package comes
                                    from the bucket
  site template put <dir>           install the template in <dir> as the next version and use it
                                    (index.html is the entry all versions share; test/ is left out)
  site template list                the installed template versions
  site template use <version>       switch to an installed version (this is how to go back)
  site template get <version> <dir>
                                    copy an installed version and index.html into <dir> (new or
                                    empty): what site template put takes again
  site preview <dir> [--data <dir>] [--port <n>]
                                    serve the template in <dir> on this machine, with the data of
                                    --data (what site try wrote) or of the bucket
  site upgrade open <id> --title .. --summary .. --file <path> [--source <id>] [--goal <id>]
  site upgrade close <id>           the upgrade was made
  site upgrade dismiss <id>         the user said no to it
  site upgrade list                 upgrade requests: what a Goal asks the user to have rebuilt
  site events get [--out <file>]    events.json; with --out written to <file>, as site events put
                                    takes it (a site without events gives the empty document)
  site events put <file>            replace events.json (checked first; the title, source and time
                                    of every entry are filled in from its article, which has to
                                    be in the bucket)
  site supplement put <file> --event <id> --adds .. [--no-translate]
                                    store one article found for an event (<file>: the article as
                                    extract returns one, JSON) and enter it in the event
  site reading [--days 7] [--json]  what the user read lately
  site reading --compact            fold the reports of the pages into reading.json; several
                                    reports of one opening (same article, same opened_at) count
                                    as one
  site visits [--days 7] [--json]   what the user looked at in this Agent's browser: per host and
                                    per page (address without query, title), visits, time spent,
                                    last time; says when the user switched the recording off

A source package is a directory:
  source.json   { "schema": "buckyos.site.source/1", "id": "news-example-com", "title": "..",
                  "home_url": "https://news.example.com/", "match": [ { "host": "news.example.com" } ],
                  "lang": "ja", "kind": "news" | "blog" | "forum", "needs": "http" | "browser" | "login",
                  "every_secs": 1800, "max_new": 20, "keep_days": 30, "comments": 10,
                  "check": { "min_chars": 200, "median_ratio": 0.3, "max_replacement": 0.01 } }
                the four numbers and check are optional
  extract.ts    export default async function extract(ctx) -> { list, items, state? }
                ctx: source, state, limit, known(url), log(text),
                     fetch(url, { conditional?, headers? }) -> { status, url, text, notModified, headers },
                     html(text), xml(text), blocks(element, baseUrl) -> { blocks, links },
                     feed(text) -> [ { url, title, published_at?, summary?, image?, content_html?,
                                       comments_url? } ]   RSS 1.0 / 2.0 and Atom,
                     browser(url | null, "script.mjs", args?)   only when needs is browser or login
                list: every entry the source lists now, { url, title, published_at?, summary?, image? }
                items: the articles fetched now, at most ctx.limit, none that ctx.known(url) says
                       the site has: { url, title, lang?, published_at?, byline?, summary?,
                       blocks: [ { type: "p"|"h2"|"h3"|"quote"|"li"|"code", text } | { type: "img", url, alt? }
                                 | { type: "table", rows } ], comments?: [ { author?, at?, text } ],
                       links?: [ { url, text } ] }
                to fail: throw new SiteError("auth" | "transient" | "program", message)
                it imports nothing but files of the package
  fixtures/     real pages for site try --fixtures; index.json maps each URL to its file
                (site try --record writes both)
  NOTES.md      how the source is read, how to check it, what goes wrong

A sync publishes an article only when it passed the check (a title, a body that is long enough
and not garbled, not the same as two others), its images are stored and it is translated. When
more than half of a run fails the check, or a list that had entries is empty, nothing is
published and the source shows as failed. What was fetched and could not be published yet
(translation budget used up, no model, a translation that went wrong, an image that did not come,
time) waits in the working directory and is published by a later sync.

As a Goal job (\$OPENDAN_JOB_RUN_ID is set), site sync and site reading --compact print the job's
result line { "summary", "kind"?, "metrics" } as the last line of stdout, keep their state in the
working directory, count the model tokens in \$OPENDAN_USAGE_LOG and stay within
\$OPENDAN_MODEL_BUDGET and the site's limits.translate_tokens_per_day. Elsewhere site sync keeps
its state in .site-job/<source>/ of the working directory.

  site sync               metrics { listed, new, published, rejected, images, tokens, ms,
                          pending, pending_reason }: pending is how many fetched articles are
                          not published yet, pending_reason why (null, translate_budget,
                          translate_config, translate_failed, images or time);
                          kind is auth, program or transient when the run failed, budget when
                          it passed and translation was refused
  site reading --compact  metrics { records, opens_7d, dwell_ms_7d, articles_7d, profile };
                          profile is "tags:<a>,<b>,<c>;sources:<x>,<y>,<z>", the three most read
                          tags and sources of the last 7 days by time spent: the same line for
                          the same reading

  agent-session goal job put src-<source> --goal <goal_id> --dir <package> \\
      --command "site sync <source> --dir {dir}" --every <every_secs> --timeout 600
  agent-session goal job put site-reading --goal <goal_id> --dir <dir> \\
      --command "site reading --compact" --every 3600
      (<dir>: a directory with one file in it, say a README telling what the job does; a job
      is not installed from an empty directory)

An Agent in read-only mode cannot write the user's bucket and so cannot keep a site: writes fail
with permission_denied. Keep what you made in the working directory and report it.

Exit codes: 0 done (site sync: the run passed the check, also when translation was refused),
1 bad arguments or an invalid package, 2 the service refused or failed, or the sync failed,
5 local file error.`;

const BOOLEAN_FLAGS = ["no-translate", "fixtures", "record", "compact"];
const VALUE_FLAGS = [
  "title", "language", "topics", "owner-did", "state", "note", "limit", "out", "dir", "time-limit", "data", "port",
  "summary", "file", "source", "goal", "days", "event", "adds",
];
const LOCAL_ERRORS = ["NotFound", "PermissionDenied", "AlreadyExists", "IsADirectory", "NotADirectory", "NotCapable"];

const isJob = () => !!Deno.env.get("OPENDAN_JOB_RUN_ID");
const log = (message: string) => console.error(`site: ${message}`);

function usage(message: string): never {
  bailArgError(TOOL, `${message} (site --help lists the subcommands)`);
}

// --json: the summary is the detail, for a caller that reads only the summary.
let summaryAsJson = false;

function done(title: string, summary: string, detail: unknown): never {
  emitAndExit(
    successResult(TOOL, `${TOOL} ${title} => done`, summaryAsJson ? JSON.stringify(detail) : summary, detail as JsonValue),
    EXIT_SUCCESS,
  );
}

// The last stdout line of a command that runs as a Goal job (Agent State Protocol §9.9).
function jobDone(result: { summary: string; kind?: string; metrics?: unknown }, exitCode: number): never {
  console.log(JSON.stringify(result));
  Deno.exit(exitCode);
}

// Every failure ends here. `job`: the command is `site sync` or `site reading --compact` run as a Goal job.
function bail(err: unknown, context: { write?: boolean; job?: boolean } = {}): never {
  const message = err instanceof Error ? err.message : String(err);
  // A Deno file error has a `code` of its own (ENOENT); the name is what tells it from a service's.
  const local = err instanceof Error && LOCAL_ERRORS.includes(err.name);
  const code = local ? undefined : errorCode(err);
  const bad = err instanceof SiteError && err.kind === "invalid";
  if (context.job) {
    const refused = bad || local || code === "permission_denied" || code === "bucket_not_found";
    jobDone({ summary: message, kind: refused ? "program" : "transient" }, bad ? EXIT_ARG_ERROR : EXIT_AICC_FAILED);
  }
  if ((err as { runtimeInit?: boolean } | null)?.runtimeInit) {
    emitAndExit(errorResult(TOOL, `${TOOL} => runtime_init_failed`, message, { error: message }), EXIT_AICC_FAILED);
  }
  if (bad) {
    const problems = (err as { problems?: string[] }).problems;
    emitAndExit(
      errorResult(TOOL, `${TOOL} => invalid`, message, { error: message, ...(problems ? { problems } : {}) }),
      EXIT_ARG_ERROR,
    );
  }
  if (code === "permission_denied" && context.write) {
    const summary =
      "this Agent may not write the user's bucket (its permission mode is read-only), so it cannot keep the personal site. Keep what you made in the working directory and report it.";
    emitAndExit(
      errorResult(TOOL, `${TOOL} => permission_denied`, summary, { error: message, error_code: code }),
      EXIT_AICC_FAILED,
    );
  }
  if (code === "bucket_not_found") {
    const summary =
      "this Zone has no bucket for the personal site: its control-panel service does not declare the reflow bucket";
    emitAndExit(errorResult(TOOL, `${TOOL} => no_bucket`, summary, { error: message, error_code: code }), EXIT_AICC_FAILED);
  }
  if (local) emitAndExit(errorResult(TOOL, `${TOOL} => io_failed`, message, { error: message }), EXIT_IO_FAILED);
  emitAndExit(
    errorResult(TOOL, `${TOOL} => failed`, message, { error: message, ...(code ? { error_code: code } : {}) }),
    EXIT_AICC_FAILED,
  );
}

interface Opened {
  runtime: AiccRuntime;
  bucket: SiteBucket;
  /** Who a change of this run is by: the Agent, or the user outside an Agent runtime. */
  by: string;
}

async function open(): Promise<Opened> {
  let runtime: AiccRuntime;
  try {
    runtime = await initRuntime();
  } catch (err) {
    throw Object.assign(err instanceof Error ? err : new Error(String(err)), { runtimeInit: true });
  }
  const store = runtime.buckyos.getBucketClient() as unknown as SiteStore;
  return {
    runtime,
    bucket: new SiteBucket(store, libraryBucketId(runtime.ownerUserId)),
    by: Deno.env.get("OPENDAN_AGENT_DID") || runtime.userId,
  };
}

// One operation on the site in the bucket: runtime, operation, one AgentToolResult.
async function inSite(
  title: string,
  context: { write?: boolean },
  operation: (opened: Opened) => Promise<{ summary: string; detail: unknown }>,
): Promise<never> {
  let out;
  try {
    out = await operation(await open());
  } catch (err) {
    bail(err, context);
  }
  done(title, out.summary, out.detail);
}

const when = (ms: number | null) => ms ? new Date(ms).toISOString() : "never";

// The DID of the user the site is published as. control-panel tells an Agent's App token (and the
// user's own session) who the owner is with `owner.context`; `user.get` is the older way.
async function ownerDid(runtime: AiccRuntime): Promise<string> {
  // deno-lint-ignore no-explicit-any
  const panel = (runtime.buckyos as any).getServiceRpcClient("control-panel");
  const failures: string[] = [];
  const isDid = (value: unknown): value is string => typeof value === "string" && value.startsWith("did:");
  try {
    const context = await panel.call("owner.context", {});
    if (isDid(context?.owner_did)) return context.owner_did;
    failures.push("owner.context names no owner_did");
  } catch (err) {
    failures.push(`owner.context: ${(err as Error).message}`);
  }
  try {
    const user = await panel.call("user.get", { user_id: runtime.ownerUserId });
    const did = user?.local_profile?.did ?? user?.profile?.did ?? user?.did_document?.id;
    if (isDid(did)) return did;
    failures.push("user.get has no DID for the user");
  } catch (err) {
    failures.push(`user.get: ${(err as Error).message}`);
  }
  throw new SiteError(
    "invalid",
    `control-panel does not tell the DID of ${runtime.ownerUserId} (${failures.join("; ")}): pass --owner-did <did>`,
  );
}

export async function siteStatus(bucket: SiteBucket, site: Site) {
  const sources = [];
  for (const source of site.sources) {
    const feed = await bucket.feed(source.id).catch(() => null);
    sources.push({
      id: source.id,
      title: source.title,
      state: displayState(site, source, feed),
      version: source.version,
      every_secs: source.every_secs,
      note: source.note,
      checked_at: feed?.status.checked_at ?? null,
      published_at: feed?.status.published_at ?? null,
      items: feed?.items.length ?? 0,
      error: feed?.status.error ?? null,
      pending: feed?.status.pending ?? null,
      login_url: feed?.status.login_url ?? null,
    });
  }
  const detail = {
    title: site.title,
    language: site.language,
    owner_did: site.owner_did,
    topics: site.topics,
    template: site.template.version,
    maintainer: site.maintainer,
    sources,
    upgrades: site.upgrades.filter((upgrade) => upgrade.state === "open"),
    updated_at: site.updated_at,
  };
  const lines = [
    `${site.title} (${site.language}), template ${site.template.version}, ${
      site.maintainer ? `kept by ${site.maintainer.agent} (goal ${site.maintainer.goal_id})` : "no maintainer"
    }`,
    ...sources.map((source) =>
      `${source.id}  ${source.state}  package ${source.version ?? "none"}  ${source.items} article(s)  checked ${
        when(source.checked_at)
      }` +
      (source.error ? `  last error (${source.error.kind}): ${source.error.message}` : "") +
      (source.pending ? `  waiting: ${source.pending.count} (${source.pending.reason})` : "") +
      (source.note ? `  note: ${source.note}` : "")
    ),
    ...(sources.length ? [] : ["no sources yet"]),
    ...detail.upgrades.map((upgrade) =>
      `upgrade ${upgrade.id}${upgrade.source ? ` (${upgrade.source})` : ""}: ${upgrade.title} — ${upgrade.file}`
    ),
  ];
  return { summary: lines.join("\n"), detail };
}

// `site visits` asks this Agent's own Agent Service, which only a session on the Agent's runtime reaches.
function noBrowser(why: string): never {
  emitAndExit(
    errorResult(
      TOOL,
      `${TOOL} visits => no_browser`,
      `what the user looked at is kept by this Agent's own browser service, which is not reachable from this runtime: ${why}`,
      { error: why },
    ),
    EXIT_AICC_FAILED,
  );
}

/** Settles an upgrade request: `closed` once the upgrade was made, `dismissed` when the user declined it. */
export function decideUpgrade(site: Site, id: string, state: "closed" | "dismissed", now: number = Date.now()): Upgrade {
  const found = site.upgrades.find((upgrade) => upgrade.id === id);
  if (!found) throw new SiteError("invalid", `there is no upgrade request ${id}`);
  found.state = state;
  found.decided_at = now;
  return found;
}

const SOURCE_FIELDS = ["title", "every_secs", "max_new", "keep_days", "comments", "state", "note"];

// The object a config path's last segment is a field of.
export function configParent(site: Site, path: string): { parent: Record<string, unknown>; field: string } {
  const [head, ...rest] = path.split(".");
  const root = site as unknown as Record<string, unknown>;
  const at = (parent: unknown, field: string) => ({ parent: parent as Record<string, unknown>, field });
  if (["title", "language", "topics", "columns", "maintainer"].includes(head) && !rest.length) return at(root, head);
  if (head === "template" && rest[0] === "params" && rest.length === 2) return at(site.template.params, rest[1]);
  if (head === "reading" && rest.length === 1 && rest[0] === "enabled") return at(site.reading, rest[0]);
  if (head === "limits" && rest.length === 1) return at(site.limits, rest[0]);
  if (head === "sources" && rest.length === 2 && SOURCE_FIELDS.includes(rest[1])) {
    const source = site.sources.find((entry) => entry.id === rest[0]);
    if (!source) throw new SiteError("invalid", `the site has no source ${rest[0]}`);
    return at(source, rest[1]);
  }
  throw new SiteError(
    "invalid",
    `${path} is not a field site config changes: title, language, topics, columns, template.params.<name>, sources.<id>.<${
      SOURCE_FIELDS.join("|")
    }>, reading.enabled, limits.<name>, maintainer`,
  );
}

export function configValue(site: Site, path: string): unknown {
  let value: unknown = site;
  for (const segment of path.split(".")) {
    if (Array.isArray(value)) value = value.find((entry) => entry?.id === segment);
    else value = (value as Record<string, unknown> | null | undefined)?.[segment];
    if (value === undefined) throw new SiteError("invalid", `site.json has no ${path}`);
  }
  return value;
}

// ── site try: fixtures ─────────────────────────────────────────────────────

type FixtureIndex = Record<string, string>;

async function fixtureFetcher(dir: string): Promise<typeof fetch> {
  let index: FixtureIndex;
  try {
    index = JSON.parse(await Deno.readTextFile(`${dir}/${FIXTURES_DIR}/${FIXTURES_INDEX}`));
  } catch (err) {
    throw new SiteError(
      "invalid",
      `--fixtures needs ${FIXTURES_DIR}/${FIXTURES_INDEX} in the package, a map of URL to file (site try --record writes it): ${
        (err as Error).message
      }`,
    );
  }
  return async (input) => {
    const file = index[String(input)];
    if (!file) return new Response("not among the fixtures", { status: 404 });
    return new Response(await Deno.readFile(`${dir}/${FIXTURES_DIR}/${file}`) as BodyInit, {
      headers: { "content-type": contentTypeOf(file) },
    });
  };
}

// Keeps every page the extractor reads as a fixture of the package.
function recordingFetcher(dir: string, index: FixtureIndex): typeof fetch {
  const kinds: [RegExp, string][] = [[/xml|rss|atom/, "xml"], [/html/, "html"], [/json/, "json"], [/^text\//, "txt"]];
  return async (input, init) => {
    const response = await fetch(input, init);
    const type = response.headers.get("content-type") ?? "";
    const ext = kinds.find(([pattern]) => pattern.test(type))?.[1];
    if (response.status !== 200 || !ext) return response;
    const url = new URL(String(input));
    const base = `${url.host}${url.pathname}${url.search}`.replace(/[^a-zA-Z0-9.-]+/g, "_").replace(/^_+|_+$/g, "")
      .replace(new RegExp(`\\.${ext}$`), "").slice(0, 80);
    let name = `${base}.${ext}`;
    for (let n = 2; Object.entries(index).some(([known, file]) => file === name && known !== url.href); n += 1) {
      name = `${base}-${n}.${ext}`;
    }
    await Deno.mkdir(`${dir}/${FIXTURES_DIR}`, { recursive: true });
    await Deno.writeFile(`${dir}/${FIXTURES_DIR}/${name}`, new Uint8Array(await response.clone().arrayBuffer()));
    index[String(input)] = name;
    return response;
  };
}

export function describeSync(report: SyncReport): string {
  return [
    report.summary,
    ...report.published.map((article) =>
      `  published ${article.id} rev ${article.rev}: ${article.title} — ${article.chars} chars, ${article.blocks} block(s), ${article.images} image(s)${
        article.missing_images ? ` (${article.missing_images} missing)` : ""
      }, ${article.comments} comment(s)${article.tags.length ? `, tags ${article.tags.join(" ")}` : ""}`
    ),
    ...report.rejected.map((entry) => `  rejected ${entry.url}: ${entry.reason}`),
    `  ${report.requests} request(s)${report.not_modified ? `, ${report.not_modified} not modified` : ""}, ${
      report.metrics.tokens
    } model token(s), ${Math.round(report.metrics.ms / 100) / 10}s`,
  ].join("\n");
}

function translatorFor(runtime: AiccRuntime, site: Site, dataDir: string) {
  const model = new MeteredModel(aiccModel(runtime.buckyos.getAiccClient()));
  return {
    model,
    translator: createTranslator({
      model,
      cacheDir: `${dataDir}/tr`,
      dailyCap: site.limits.translate_tokens_per_day,
    }),
  };
}

async function trySource(
  dir: string,
  options: { limit: number; translate: boolean; fixtures: boolean; record: boolean; out?: string },
): Promise<never> {
  let report: SyncReport;
  let out: string;
  let model: MeteredModel | undefined;
  try {
    const { def } = await readSourcePackage(dir);
    out = options.out ?? await Deno.makeTempDir({ prefix: `site-try-${def.id}-` });
    const dataDir = `${out}/_job`;
    for (const file of ["state.json", "pending.json", "index.json"]) await Deno.remove(`${dataDir}/${file}`).catch(() => {});
    await Deno.remove(`${out}/_data`, { recursive: true }).catch(() => {});

    const now = Date.now();
    const source: SourceEntry = { ...sourceEntry(def, null, "site try", now), state: "active" };
    let site: Site = {
      schema: SITE_SCHEMA,
      title: def.title,
      language: def.lang,
      owner_did: "did:web:site-try.invalid",
      topics: [],
      template: { version: "t1", params: { home_order: "time", per_column: 20, hide_topics: [], collapsed: [] } },
      columns: [],
      sources: [source],
      upgrades: [],
      reading: { enabled: false },
      limits: { translate_tokens_per_day: 1_000_000 },
      maintainer: null,
      updated_at: now,
      updated_by: "site try",
    };
    let translator = null;
    if (options.translate) {
      const opened = await open();
      const real = (await opened.bucket.site())?.site;
      if (real) {
        site = { ...site, title: real.title, language: real.language, owner_did: real.owner_did, topics: real.topics };
      }
      ({ model, translator } = translatorFor(opened.runtime, site, dataDir));
    }
    const bucket = new SiteBucket(dirStore(out), "local");
    await bucket.write(SITE_KEY, JSON.stringify(site, null, 2));

    const recorded: FixtureIndex = {};
    const net = options.fixtures
      ? new Net({ fetcher: await fixtureFetcher(dir), sleep: () => Promise.resolve() })
      : new Net(options.record ? { fetcher: recordingFetcher(dir, recorded) } : {});
    report = await syncSource({
      bucket,
      site,
      source,
      def,
      dir,
      dataDir,
      net,
      translator,
      resize: await findResize(),
      tokens: () => model?.tokens ?? 0,
      timeLimitMs: DEFAULT_TIME_LIMIT_S * 1000,
      limit: options.limit,
      imageTries: 1,
      cleanup: false,
      log,
    });
    if (options.record && Object.keys(recorded).length) {
      await Deno.writeTextFile(`${dir}/${FIXTURES_DIR}/${FIXTURES_INDEX}`, `${JSON.stringify(recorded, null, 2)}\n`);
    }
  } catch (err) {
    bail(err);
  }
  const detail = { ...report, out, model: model?.model ?? null } as unknown as JsonValue;
  const summary = `${describeSync(report)}\n  written to ${out} (site preview <template dir> --data ${out} shows it)`;
  if (!report.ok) {
    emitAndExit(errorResult(TOOL, `${TOOL} try => ${report.kind ?? "failed"}`, summary, detail), EXIT_AICC_FAILED);
  }
  done("try", summary, detail);
}

async function syncCommand(
  sourceId: string,
  options: { dir?: string; timeLimitS: number; translate: boolean },
): Promise<never> {
  const job = isJob();
  let report: SyncReport | undefined;
  let failure: unknown;
  let model: MeteredModel | undefined;
  let fetched: string | undefined;
  try {
    const { runtime, bucket } = await open();
    const site = await bucket.requireSite();
    const source = site.sources.find((entry) => entry.id === sourceId);
    if (!source) throw new SiteError("invalid", `the site has no source ${sourceId}: site source put <dir> adds it`);
    let dir = options.dir;
    if (!dir) {
      dir = fetched = await Deno.makeTempDir({ prefix: `site-src-${sourceId}-` });
      await getSource(bucket, site, sourceId, dir);
    }
    const { def } = await readSourcePackage(dir);
    if (def.id !== sourceId) {
      throw new SiteError("invalid", `the package in ${dir} is the source ${def.id}, not ${sourceId}`);
    }
    const dataDir = job ? Deno.cwd() : `${Deno.cwd()}/.site-job/${sourceId}`;
    let translator = null;
    if (options.translate) ({ model, translator } = translatorFor(runtime, site, dataDir));
    report = await syncSource({
      bucket,
      site,
      source,
      def,
      dir,
      dataDir,
      net: new Net(),
      translator,
      resize: await findResize(),
      tokens: () => model?.tokens ?? 0,
      timeLimitMs: options.timeLimitS * 1000,
      log,
    });
  } catch (err) {
    failure = err;
  }
  if (fetched) await Deno.remove(fetched, { recursive: true }).catch(() => {});
  if (!report) bail(failure, { write: true, job });
  const { result, exitCode } = syncOutcome(report, job, model?.model ?? null);
  console.log(JSON.stringify(result));
  Deno.exit(exitCode);
}

/**
 * What `site sync` prints and exits with. As a Goal job: the job's result
 * line. Exit code 0 when the run passed the check — also when translation was
 * refused (`kind: "budget"`).
 */
export function syncOutcome(report: SyncReport, job: boolean, model: string | null): { result: unknown; exitCode: number } {
  const exitCode = report.ok ? EXIT_SUCCESS : EXIT_AICC_FAILED;
  if (job) {
    return {
      result: { summary: report.summary, ...(report.kind ? { kind: report.kind } : {}), metrics: report.metrics },
      exitCode,
    };
  }
  const detail = { ...report, model } as unknown as JsonValue;
  return {
    result: report.ok
      ? successResult(TOOL, `${TOOL} sync => done`, describeSync(report), detail)
      : errorResult(TOOL, `${TOOL} sync => ${report.kind}`, describeSync(report), detail),
    exitCode,
  };
}

// ── site preview ───────────────────────────────────────────────────────────

async function preview(dir: string, options: { data?: string; port: number }): Promise<never> {
  let bucket: SiteBucket | undefined;
  try {
    await Deno.stat(`${dir}/index.html`);
    if (options.data) await Deno.stat(`${options.data}/${SITE_KEY}`);
    else bucket = (await open()).bucket;
  } catch (err) {
    bail(err);
  }
  const file = async (path: string): Promise<Response> => {
    try {
      return new Response(await Deno.readFile(path) as BodyInit, {
        headers: { "content-type": contentTypeOf(path), "cache-control": "no-store" },
      });
    } catch {
      return new Response("not found", { status: 404 });
    }
  };
  const server = Deno.serve({ hostname: "127.0.0.1", port: options.port, onListen: () => {} }, async (request) => {
    const path = decodeURIComponent(new URL(request.url).pathname);
    if (path.split("/").includes("..")) return new Response("not found", { status: 404 });
    if (request.method === "POST" && path === "/_read") return new Response(null, { status: 204 });
    if (request.method !== "GET" && request.method !== "HEAD") return new Response("no", { status: 405 });
    if (path === "/" || path === "/index.html") return await file(`${dir}/index.html`);
    const app = /^\/_app\/[^/]+\/(.+)$/.exec(path);
    if (app) return await file(`${dir}/${app[1]}`);
    if (!path.startsWith("/_data/")) return new Response("not found", { status: 404 });
    if (options.data) return await file(`${options.data}${path}`);
    const found = await bucket!.read(path.slice(1)).catch(() => null);
    return found
      ? new Response(found.bytes as BodyInit, {
        headers: { "content-type": contentTypeOf(path), "cache-control": "no-store" },
      })
      : new Response("not found", { status: 404 });
  });
  const url = `http://127.0.0.1:${server.addr.port}/`;
  emit(successResult(
    TOOL,
    `${TOOL} preview => serving`,
    `the template in ${dir} is at ${url} with the data of ${options.data ?? "the bucket"}; it serves until this command is stopped`,
    { url, dir, data: options.data ?? null },
  ));
  await server.finished;
  Deno.exit(EXIT_SUCCESS);
}

// ── the subcommands ────────────────────────────────────────────────────────

export async function run(argv: string[]): Promise<never> {
  // ── 1. 解析参数 ────────────────────────────────────────────────────────
  const parsed = parseArgvOrExit(TOOL, HELP, argv, BOOLEAN_FLAGS);
  const [sub, ...args] = parsed.positional;
  const flags = parsed.flags;
  summaryAsJson = parsed.common.json;
  const unknown = [...consumeCommon(flags).keys()].find((name) =>
    !BOOLEAN_FLAGS.includes(name) && !VALUE_FLAGS.includes(name)
  );
  if (unknown) usage(`unsupported option: --${unknown}`);
  const expect = (count: number, form: string) => {
    if (args.length !== count) usage(`expected: site ${form}`);
  };
  const text = (name: string): string | undefined => {
    try {
      return requireString(flags, name);
    } catch (err) {
      bailArgError(TOOL, err as Error);
    }
  };
  const int = (name: string, min: number): number | undefined => {
    let value;
    try {
      value = flagInt(flags, name);
    } catch (err) {
      bailArgError(TOOL, err as Error);
    }
    if (value !== undefined && value < min) usage(`--${name} must be at least ${min}`);
    return value;
  };
  const need = (name: string): string => text(name) ?? usage(`--${name} is required`);

  // ── 2–6. 每个子命令一次站点操作 ────────────────────────────────────────
  switch (sub) {
    case "init": {
      expect(0, "init [--title ..] [--language ..] [--topics a,b,c]");
      const topics = (text("topics") ?? "").split(",").map((topic) => topic.trim()).filter(Boolean);
      return await inSite("init", { write: true }, async ({ runtime, bucket, by }) => {
        const existing = await bucket.site();
        if (existing) {
          const status = await siteStatus(bucket, existing.site);
          return { summary: `the site exists already; nothing was changed\n${status.summary}`, detail: status.detail };
        }
        const did = text("owner-did") ?? await ownerDid(runtime);
        const template = await putTemplate(bucket, TEMPLATE_DIR, by);
        const now = Date.now();
        const site: Site = {
          schema: SITE_SCHEMA,
          title: text("title") ?? "Reading",
          language: text("language") ?? "en",
          owner_did: did,
          topics,
          template: {
            version: template.version,
            params: { home_order: "time", per_column: 20, hide_topics: [], collapsed: [] },
          },
          columns: [],
          sources: [],
          upgrades: [],
          reading: { enabled: true },
          limits: { translate_tokens_per_day: 1_000_000 },
          maintainer: null,
          updated_at: now,
          updated_by: by,
        };
        if (!await bucket.createSite(site)) {
          throw new Error("somebody else made the site in the meantime: site status shows it");
        }
        return {
          summary:
            `made the site "${site.title}" (${site.language}) of ${did} with the default template as ${template.version}; add sources with site source put`,
          detail: { site, template },
        };
      });
    }
    case "status":
      expect(0, "status");
      return await inSite("status", {}, async ({ bucket }) => siteStatus(bucket, await bucket.requireSite()));
    case "match":
      expect(1, "match <url>");
      return await inSite("match", {}, async ({ bucket }) => {
        const site = await bucket.requireSite();
        const { url, id } = await articleAddress(args[0]);
        const article = await bucket.article(id);
        const source = sourceOfUrl(site, url);
        const detail = {
          url,
          article: article ? { id, route: `a=${id}`, source: article.source, title: article.title.t, rev: article.rev } : null,
          source: source ? { id: source.id, route: `s=${source.id}`, state: source.state } : null,
        };
        const summary = article
          ? `${url} is the article ${id} of ${article.source}: /?a=${id}`
          : source
          ? `${url} belongs to the source ${source.id} (/?s=${source.id}) and is not among its articles`
          : `${url} is neither an article nor a page of a source of the site`;
        return { summary, detail };
      });
    case "config": {
      const [action, path, value] = args;
      if (action === "get" && args.length <= 2) {
        return await inSite("config get", {}, async ({ bucket }) => {
          const site = await bucket.requireSite();
          const found = path ? configValue(site, path) : site;
          return { summary: JSON.stringify(found, null, 2), detail: { path: path ?? null, value: found } };
        });
      }
      if (action !== "set" || args.length !== 3) usage("expected: site config get [<path>] | site config set <path> <json>");
      let next: unknown;
      try {
        next = JSON.parse(value);
      } catch {
        usage(`the value must be JSON (text in double quotes): ${value}`);
      }
      return await inSite("config set", { write: true }, async ({ bucket, by }) => {
        const site = await bucket.updateSite((draft) => {
          const { parent, field } = configParent(draft, path);
          parent[field] = next;
        }, by);
        return { summary: `${path} is now ${JSON.stringify(configValue(site, path))}`, detail: { path, value: next } };
      });
    }
    case "source": {
      const [action, ...rest] = args;
      if (action === "put" && rest.length === 1) {
        const state = text("state");
        if (state && !["preparing", "active", "failed"].includes(state)) usage("--state must be preparing, active or failed");
        const note = text("note");
        return await inSite("source put", { write: true }, async ({ bucket, by }) => {
          const session = Deno.env.get("OPENDAN_SESSION_ID");
          const put = await putSource(bucket, rest[0], {
            ...(state ? { state: state as SourceState } : {}),
            ...(note === undefined ? {} : { note }),
            by,
            addedBy: session ? `${session.startsWith("ws-") ? "work" : "session"}:${session}` : by,
          });
          return {
            summary:
              `${put.id}: the package (${put.files.length} file(s)) is ${put.version}; the source is ${put.entry.state}, every ${put.entry.every_secs}s, job ${put.entry.job}`,
            detail: put,
          };
        });
      }
      if (action === "list" && rest.length === 0) {
        return await inSite("source list", {}, async ({ bucket }) => {
          const site = await bucket.requireSite();
          const lines = site.sources.map((source) =>
            `${source.id}  ${source.title}  ${source.state}  package ${source.version ?? "none"}  ${source.needs}  every ${source.every_secs}s  ${
              source.match.map((rule) => `${rule.host}${rule.path ?? "/*"}`).join(" ")
            }`
          );
          return { summary: lines.length ? lines.join("\n") : "the site has no sources", detail: { sources: site.sources } };
        });
      }
      if (action === "get" && rest.length === 2) {
        return await inSite("source get", {}, async ({ bucket }) => {
          const got = await getSource(bucket, await bucket.requireSite(), rest[0], rest[1]);
          const dir = await Deno.realPath(rest[1]);
          return {
            summary: `wrote ${rest[0]} ${got.version} (${got.files.length} file(s)) to ${dir}`,
            detail: { id: rest[0], dir, ...got },
          };
        });
      }
      if (action === "rm" && rest.length === 1) {
        if (!isSourceId(rest[0])) usage(`not a source id: ${rest[0]}`);
        return await inSite("source rm", { write: true }, async ({ bucket, by }) => {
          const removed = await removeSource(bucket, rest[0], by);
          return {
            summary:
              `${rest[0]} is no longer a source of the site (${removed} object(s) removed); its articles stay until they expire, and its Goal job is yours to remove`,
            detail: { id: rest[0], removed },
          };
        });
      }
      usage("expected: site source put <dir> | list | get <id> <dir> | rm <id>");
      break;
    }
    case "try":
      expect(1, "try <dir> [--limit 3] [--no-translate] [--fixtures] [--record] [--out <dir>]");
      if (flags.get("fixtures") === true && flags.get("record") === true) usage("--fixtures and --record exclude each other");
      return await trySource(args[0], {
        limit: int("limit", 1) ?? TRY_LIMIT,
        translate: flags.get("no-translate") !== true,
        fixtures: flags.get("fixtures") === true,
        record: flags.get("record") === true,
        out: text("out"),
      });
    case "sync":
      expect(1, "sync <source> [--dir <package>] [--time-limit <seconds>] [--no-translate]");
      return await syncCommand(args[0], {
        dir: text("dir"),
        timeLimitS: int("time-limit", 1) ?? DEFAULT_TIME_LIMIT_S,
        translate: flags.get("no-translate") !== true,
      });
    case "template": {
      const [action, ...rest] = args;
      if (action === "put" && rest.length === 1) {
        return await inSite("template put", { write: true }, async ({ bucket, by }) => {
          await bucket.requireSite();
          const put = await putTemplate(bucket, rest[0], by);
          await useTemplate(bucket, put.version, by);
          return {
            summary: `the template (${put.files.length} file(s)) is ${put.version} and the site uses it`,
            detail: put,
          };
        });
      }
      if (action === "list" && rest.length === 0) {
        return await inSite("template list", {}, async ({ bucket }) => {
          const current = (await bucket.requireSite()).template.version;
          const templates = (await listTemplates(bucket)).map((entry) => ({ ...entry, current: entry.version === current }));
          return {
            summary: templates.map((entry) =>
              `${entry.version}${entry.current ? "  in use" : ""}  installed ${when(entry.updated_at)} by ${entry.updated_by ?? "?"}`
            ).join("\n") || "no template is installed",
            detail: { current, templates },
          };
        });
      }
      if (action === "use" && rest.length === 1) {
        return await inSite("template use", { write: true }, async ({ bucket, by }) => {
          await useTemplate(bucket, rest[0], by);
          return { summary: `the site uses the template ${rest[0]}`, detail: { version: rest[0] } };
        });
      }
      if (action === "get" && rest.length === 2) {
        return await inSite("template get", {}, async ({ bucket }) => {
          const got = await getTemplate(bucket, rest[0], rest[1]);
          const dir = await Deno.realPath(rest[1]);
          return {
            summary: `wrote the template ${got.version} (${got.files.length} file(s), index.html among them) to ${dir}`,
            detail: { dir, ...got },
          };
        });
      }
      usage("expected: site template put <dir> | list | use <version> | get <version> <dir>");
      break;
    }
    case "preview":
      expect(1, "preview <dir> [--data <dir>] [--port <n>]");
      return await preview(args[0], { data: text("data"), port: int("port", 0) ?? 0 });
    case "upgrade": {
      const [action, id] = args;
      if (action === "list" && args.length === 1) {
        return await inSite("upgrade list", {}, async ({ bucket }) => {
          const upgrades = (await bucket.requireSite()).upgrades;
          return {
            summary: upgrades.map((upgrade) =>
              `${upgrade.id}  ${upgrade.state}${upgrade.source ? `  ${upgrade.source}` : ""}  ${upgrade.title}  ${upgrade.file}`
            ).join("\n") || "no upgrade requests",
            detail: { upgrades },
          };
        });
      }
      if (action === "open" && args.length === 2) {
        const request = { title: need("title"), summary: need("summary"), file: need("file"), source: text("source") };
        const goal = text("goal") ?? Deno.env.get("OPENDAN_GOAL_ID");
        return await inSite("upgrade open", { write: true }, async ({ bucket, by }) => {
          let opened!: Upgrade;
          await bucket.updateSite((site) => {
            if (site.upgrades.some((upgrade) => upgrade.id === id)) {
              throw new SiteError("invalid", `there is an upgrade request ${id} already: a new problem takes a new short name`);
            }
            if (request.source && !site.sources.some((source) => source.id === request.source)) {
              throw new SiteError("invalid", `the site has no source ${request.source}`);
            }
            const goalId = goal ?? site.maintainer?.goal_id;
            if (!goalId) throw new SiteError("invalid", "which Goal asks? pass --goal <goal_id> (the site has no maintainer)");
            opened = {
              id,
              ...(request.source ? { source: request.source } : {}),
              title: request.title,
              summary: request.summary,
              file: request.file,
              goal_id: goalId,
              opened_at: Date.now(),
              state: "open",
            };
            site.upgrades.push(opened);
            const settled = site.upgrades.filter((upgrade) => upgrade.state === "closed" || upgrade.state === "dismissed");
            for (const old of settled.slice(0, Math.max(0, site.upgrades.length - MAX_UPGRADES))) {
              site.upgrades.splice(site.upgrades.indexOf(old), 1);
            }
          }, by);
          return { summary: `opened the upgrade request ${id}: ${opened.title}`, detail: opened };
        });
      }
      if ((action === "close" || action === "dismiss") && args.length === 2) {
        return await inSite(`upgrade ${action}`, { write: true }, async ({ bucket, by }) => {
          let decided!: Upgrade;
          await bucket.updateSite((site) => {
            decided = decideUpgrade(site, id, action === "close" ? "closed" : "dismissed");
          }, by);
          return { summary: `the upgrade request ${id} is ${decided.state}`, detail: decided };
        });
      }
      usage(
        "expected: site upgrade open <id> --title .. --summary .. --file <path> [--source <id>] | close <id> | dismiss <id> | list",
      );
      break;
    }
    case "events": {
      const [action, file] = args;
      if (action === "get" && args.length === 1) {
        const out = text("out");
        return await inSite("events get", {}, async ({ bucket }) => {
          const events = await bucket.events() ?? emptyEvents();
          if (!out) return { summary: JSON.stringify(events, null, 2), detail: events };
          await Deno.writeTextFile(out, `${JSON.stringify(events, null, 2)}\n`);
          const path = await Deno.realPath(out);
          return {
            summary: `wrote events.json (${events.events.length} event(s)) to ${path}; site events put ${out} writes it back`,
            detail: { file: path, events: events.events.length },
          };
        });
      }
      if (action === "put" && args.length === 2) {
        return await inSite("events put", { write: true }, async ({ bucket }) => {
          let events: unknown;
          try {
            events = JSON.parse(await Deno.readTextFile(file));
          } catch (err) {
            if (err instanceof SyntaxError) throw new SiteError("invalid", `${file} is not JSON: ${err.message}`);
            throw err;
          }
          const complete = await bucket.completeEvents(events);
          await bucket.writeEvents(complete);
          return { summary: `events.json has ${complete.events.length} event(s)`, detail: { events: complete.events.length } };
        });
      }
      usage("expected: site events get [--out <file>] | put <file>");
      break;
    }
    case "supplement": {
      if (args[0] !== "put" || args.length !== 2) usage("expected: site supplement put <file> --event <id> --adds ..");
      const event = need("event");
      const adds = need("adds");
      return await inSite("supplement put", { write: true }, async ({ runtime, bucket }) => {
        let raw: RawArticle;
        try {
          raw = JSON.parse(await Deno.readTextFile(args[1]));
        } catch (err) {
          if (err instanceof SyntaxError) throw new SiteError("invalid", `${args[1]} is not JSON: ${err.message}`);
          throw err;
        }
        const site = await bucket.requireSite();
        const cache = await Deno.makeTempDir({ prefix: "site-supplement-" });
        try {
          const translation = flags.get("no-translate") === true ? null : translatorFor(runtime, site, cache);
          const put = await putSupplement({
            bucket,
            site,
            raw,
            event,
            adds,
            net: new Net(),
            translator: translation?.translator ?? null,
            resize: await findResize(),
            log,
          });
          return {
            summary:
              `stored the supplement ${put.article.id} (/?a=${put.article.id}) for ${event}: ${put.article.title.t} — ${put.article.blocks.length} block(s), ${put.images} image(s), ${
                translation?.model.tokens ?? 0
              } model token(s)`,
            detail: {
              id: put.article.id,
              route: `a=${put.article.id}`,
              event,
              rev: put.article.rev,
              images: put.images,
              tokens: translation?.model.tokens ?? 0,
              model: translation?.model.model ?? null,
            },
          };
        } finally {
          await Deno.remove(cache, { recursive: true }).catch(() => {});
        }
      });
    }
    case "reading": {
      expect(0, "reading [--days 7] | reading --compact");
      if (flags.get("compact") === true) {
        const job = isJob();
        let report;
        try {
          const { bucket } = await open();
          report = await compactReading(bucket, job ? Deno.cwd() : `${Deno.cwd()}/.site-job/_reading`);
        } catch (err) {
          bail(err, { write: true, job });
        }
        const { metrics } = report;
        const summary =
          `reading: ${metrics.records} record(s) of ${report.batches} report(s) folded in; ${report.articles} article(s) read in the last 90 days, ${metrics.articles_7d} in the last 7 (${metrics.opens_7d} open(s), ${
            Math.round(metrics.dwell_ms_7d / 1000)
          }s; ${metrics.profile})`;
        if (job) jobDone({ summary, metrics }, EXIT_SUCCESS);
        done("reading", summary, report);
      }
      const days = int("days", 1) ?? 7;
      return await inSite("reading", {}, async ({ bucket }) => {
        const reading = await bucket.reading();
        if (!reading) return { summary: "nothing was read yet, or the reports were not folded in yet", detail: null };
        const recent = recentReading(reading, days);
        const lines = [
          `${recent.articles.length} article(s) opened in the last ${days} day(s); summary of ${when(reading.updated_at)}`,
          ...recent.articles.map((entry) =>
            `${entry.id}  ${entry.source}  opened ${entry.opens}x, ${Math.round(entry.dwell_ms / 1000)}s, read to ${entry.max_scroll}%, last ${
              when(entry.last_opened_at)
            }`
          ),
          ...Object.entries(recent.sources).map(([id, entry]) =>
            `source ${id}: ${entry.opens} open(s), ${Math.round(entry.dwell_ms / 1000)}s`
          ),
          ...Object.entries(recent.tags).map(([tag, entry]) =>
            `tag ${tag}: ${entry.opens} open(s), ${Math.round(entry.dwell_ms / 1000)}s`
          ),
        ];
        return { summary: lines.join("\n"), detail: recent };
      });
    }
    case "visits": {
      expect(0, "visits [--days 7]");
      const days = int("days", 1) ?? 7;
      const access = agentServiceAccess();
      if (!access) noBrowser("$OPENDAN_BROWSER_RPC and $BUCKYOS_APPCLIENT_SESSION_TOKEN are not both set here");
      let answer: VisitsAnswer;
      try {
        answer = await callAgentService<VisitsAnswer>(access.endpoint, access.token, "browser.visits", {
          since_ms: Date.now() - days * 86_400_000,
        });
      } catch (err) {
        const { code, message } = serviceError(err);
        if (!code) noBrowser(message);
        emitAndExit(
          errorResult(TOOL, `${TOOL} visits => ${code}`, message, { error: message, error_code: code }),
          EXIT_AICC_FAILED,
        );
      }
      const shown = describeVisits({ enabled: answer.enabled !== false, visits: answer.visits ?? [] }, days);
      done("visits", shown.summary, shown.detail);
      break;
    }
    default:
      usage(sub ? `unknown subcommand: ${sub}` : "a subcommand is required");
  }
}

if (import.meta.main) {
  await run(Deno.args);
}
