// Check of the goal that keeps the owner's reading site (goal skill reading_site_ops). The hand-over copies this
// file into the workspace as check.ts, fills in the two constants below and installs it; the goal may rewrite it.
//
// The host runs it every 5 minutes, before any model call. It wakes the goal only for
// - a source whose job failed PROGRAM_FAILURES times in a row with kind `program` (the original site changed or
//   the package is broken): once per installed version of the job, so a repair that does not work wakes it again;
// - a source without a successful sync for STALE_PERIODS of its periods for any other reason (unreachable, timing
//   out, not run at all): once, until it syncs again;
// - a changed reading profile (which topics and sources the owner spent most time on in the last 7 days): once a
//   day at most, and only when at least MIN_OPENS articles were opened;
// - another job of the goal (the reading summary) that failed OTHER_FAILURES times in a row: once per version.
// Two things wake nobody; the check tells the owner itself, once per episode:
// - a source that needs the owner's sign-in (kind `auth`), until it syncs again;
// - a Zone without a model for translation (a sync's `metrics.pending_reason` is `translate_config`: articles are
//   fetched and none can be published), until a sync publishes again.
// Articles waiting for another reason (`translate_budget`: today's limit is used up; `time`; `images`) are normal.
// A job that is switched off (a source the owner paused, or one stopped for an upgrade request) is left alone.

// Filled in when the goal is set up.
const AGENT = ""; // the Agent's DID ($OPENDAN_AGENT_DID): whose Agent Browser the notification opens
const TEXT = { // in the owner's language, short enough for a lock screen
  login_title: (source: string) => `${source}: sign-in needed`,
  login_message: "This source can only be read while you are signed in. Sign in on its website; it updates again by itself.",
  model_title: "Reading site: no model for translation",
  model_message: "New articles are fetched but cannot be translated, so none are published. The Zone needs a model for translation (llm.translate): ask its administrator.",
};

const PROGRAM_FAILURES = 2;
const STALE_PERIODS = 3;
const OTHER_FAILURES = 3;
const MIN_OPENS = 5;
const DAY_MS = 24 * 3600 * 1000;
const UNKNOWN_PERIOD_SECS = 8 * 3600; // a job on a cron timetable: stale after a day

type Run = { status: string; kind?: string; summary?: string; metrics?: Record<string, unknown> | null; ended_at_ms: number };
type Job = { job_id: string; version: number; enabled: boolean; failures: number; last_run?: Run | null };
type Ctx = { goal_id: string; agent_root?: string | null; slot_ms: number; jobs?: Job[] };
type Source = { seen_at: number; ok_at?: number; told?: string };
type State = {
  sources?: Record<string, Source>;
  jobs?: Record<string, number>;
  reading?: { profile: string; at: number };
  no_model_told?: boolean;
};
type Notify = { title: string; message: string; url: string; level: string };

// deno-lint-ignore no-explicit-any
async function readJson(path: string): Promise<any> {
  try {
    return JSON.parse(await Deno.readTextFile(path));
  } catch {
    return null;
  }
}

// `tags:b,a;sources:y,x` and `tags:a,b;sources:x,y` are the same reading: only who is among the first three counts.
const sameReading = (profile: string) =>
  profile.split(";").map((part) => {
    const [name, list = ""] = part.split(":");
    return `${name}:${list.split(",").filter(Boolean).sort().join(",")}`;
  }).join(";");

const cut = (text: string | undefined) => (text ?? "").replace(/\s+/g, " ").trim().slice(0, 300);

export default async function check(ctx: Ctx, state: State | null) {
  const slot = ctx.slot_ms;
  const next: State = { sources: {}, jobs: {}, reading: state?.reading, no_model_told: state?.no_model_told };
  let noModel = false; // a sync fetched articles that no model could translate
  let published = false; // a sync published something: translation works
  const wake: { reason: string; mark: () => void }[] = [];
  let notify: { notify: Notify; mark: () => void } | null = null;
  // Where the host keeps a job: its timetable (job.json) and the installed package (versions/<n>/source.json).
  const jobDir = (job: Job) => `${ctx.agent_root}/state/goals/${ctx.goal_id}/jobs/${job.job_id}`;

  for (const job of ctx.jobs ?? []) {
    const last = job.last_run ?? null;
    if (!job.job_id.startsWith("src-")) {
      const told = state?.jobs?.[job.job_id];
      if (told !== undefined) next.jobs![job.job_id] = told;
      if (job.enabled && job.failures >= OTHER_FAILURES && told !== job.version) {
        wake.push({
          reason: `job ${job.job_id} v${job.version} failed ${job.failures} times in a row: ${last?.kind ?? last?.status} ${cut(last?.summary)}`,
          mark: () => next.jobs![job.job_id] = job.version,
        });
      }
      continue;
    }
    const id = job.job_id.slice("src-".length);
    if (!job.enabled) {
      next.sources![id] = { seen_at: slot };
      continue;
    }
    const before = state?.sources?.[id];
    const source: Source = { seen_at: before?.seen_at ?? slot, ok_at: before?.ok_at, told: before?.told };
    next.sources![id] = source;
    if (last?.status === "ok") {
      source.ok_at = last.ended_at_ms;
      source.told = undefined;
      if (last.metrics?.pending_reason === "translate_config") noModel = true;
      if (Number(last.metrics?.published) > 0) published = true;
      continue;
    }
    if (last?.kind === "auth") {
      if (source.told !== "auth" && !notify) {
        const def = await readJson(`${jobDir(job)}/versions/${job.version}/source.json`);
        const url = typeof def?.home_url === "string" ? `&mode=page&url=${encodeURIComponent(def.home_url)}` : "&mode=home";
        notify = {
          notify: {
            title: TEXT.login_title(typeof def?.title === "string" ? def.title : id),
            message: TEXT.login_message,
            url: `/?agent_browser=${encodeURIComponent(AGENT)}${url}`,
            level: "normal",
          },
          mark: () => source.told = "auth",
        };
      }
      continue;
    }
    const program = `program:${job.version}`;
    if (last?.kind === "program" && job.failures >= PROGRAM_FAILURES) {
      if (source.told !== program) {
        wake.push({
          reason: `source ${id}: job ${job.job_id} v${job.version} failed ${job.failures} times in a row (program): ${cut(last.summary)}`,
          mark: () => source.told = program,
        });
      }
      continue;
    }
    if (source.told) continue;
    const every = (await readJson(`${jobDir(job)}/job.json`))?.every_secs;
    const period = (typeof every === "number" && every > 0 ? every : UNKNOWN_PERIOD_SECS) * 1000;
    const since = source.ok_at ?? source.seen_at;
    if (slot - since > STALE_PERIODS * period) {
      wake.push({
        reason: `source ${id}: no successful sync for ${Math.round((slot - since) / 60000)} min (its period is ${Math.round(period / 60000)} min); last run ${last ? `${last.status}${last.kind ? ` (${last.kind})` : ""}: ${cut(last.summary)}` : "none yet"}`,
        mark: () => source.told = "stale",
      });
    }
  }

  const reading = ctx.jobs?.find((job) => job.job_id === "site-reading")?.last_run;
  const metrics = reading?.status === "ok" ? reading.metrics : null;
  if (metrics && typeof metrics.profile === "string" && Number(metrics.opens_7d) >= MIN_OPENS) {
    const profile = sameReading(metrics.profile);
    const before = state?.reading;
    if (profile !== before?.profile && slot - (before?.at ?? 0) >= DAY_MS) {
      wake.push({
        reason: `what the owner reads changed: ${profile} (it was ${before?.profile ?? "not looked at yet"}), ${metrics.opens_7d} article(s) opened in 7 days`,
        mark: () => next.reading = { profile, at: slot },
      });
    }
  }

  if (!noModel && published) next.no_model_told = undefined;
  if (noModel && !next.no_model_told && !notify) {
    notify = {
      notify: { title: TEXT.model_title, message: TEXT.model_message, url: `/?agent_browser=${encodeURIComponent(AGENT)}&mode=home`, level: "normal" },
      mark: () => next.no_model_told = true,
    };
  }

  // A notification goes out at once and alone: a `run: true` result may be set aside while the goal is held.
  if (notify) {
    notify.mark();
    return { run: false, notify: notify.notify, state: next };
  }
  if (!wake.length) return { run: false, state: next };
  for (const entry of wake) entry.mark();
  return { run: true, reason: wake.map((entry) => entry.reason).join("; "), state: next };
}
