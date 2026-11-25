// Translating and tagging one article: Personal Site Protocol §5.2. One model
// request per article; what a block was translated to is kept by the SHA-256
// of its original, so a corrected article only has its changed blocks done.

import { MeteredModel, ModelError } from "../model.ts";
import { SiteError } from "./types.ts";

export const TRANSLATE_MODEL = "llm.translate";
export const MAX_TAGS = 3;
const CACHE_SCHEMA = "buckyos.site.tr/1";
const EXCERPT_CHARS = 4000;

export interface TranslateJob {
  /** The article id: names the cache file and the trace. */
  id: string;
  from: string;
  to: string;
  topics: string[];
  /** The article's original texts: `texts[0]` is its title. */
  texts: string[];
  /** The article's own summary; "" when it has none and one is to be written. */
  summary: string;
}

export interface Translation {
  /** `texts[i]` in the site's language. */
  texts: string[];
  /** At most 3 of `topics`. */
  tags: string[];
  /** The summary written for an article that has none; "" otherwise. */
  summary: string;
}

export interface Translator {
  translate(job: TranslateJob): Promise<Translation>;
}

interface Cache {
  schema: string;
  language: string;
  t: Record<string, string>;
  tags?: string[];
  summary?: string;
}

const SYSTEM = `You prepare one web article for a personal reading site. Answer with JSON only.

translations: translations[i] is texts[i] in the target language. Give exactly as many entries as texts has, in the same order: never merge, split, drop or reorder entries. Translate faithfully, in the register of the original, and add nothing of your own. Keep names of people and organizations recognizable, and keep numbers, URLs and code as they are. An entry that is already in the target language, or has nothing to translate, comes back unchanged. [] when texts is empty.

tags: at most ${MAX_TAGS} entries of topics that say what the article is about, spelled exactly as in topics. [] when none fits or topics is empty.

summary: when write_summary is true, one or two sentences in the target language that say what the article reports; otherwise "".`;

const SCHEMA = {
  name: "site_translation",
  schema: {
    type: "object",
    additionalProperties: false,
    required: ["translations", "tags", "summary"],
    properties: {
      translations: { type: "array", items: { type: "string" } },
      tags: { type: "array", items: { type: "string" } },
      summary: { type: "string" },
    },
  },
};

/** `zh-CN` and `zh` are one language to translate between; `en` and `ja` are not. */
export function sameLanguage(a: string, b: string): boolean {
  const primary = (tag: string) => tag.trim().toLowerCase().split(/[-_]/)[0];
  return primary(a) === primary(b);
}

async function sha256(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

function utcDayStart(ms: number): number {
  return Math.floor(ms / 86_400_000) * 86_400_000;
}

function answerOf(text: string): { translations: string[]; tags: string[]; summary: string } | null {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  const v = value as { translations?: unknown; tags?: unknown; summary?: unknown } | null;
  const strings = (list: unknown): list is string[] =>
    Array.isArray(list) && list.every((entry) => typeof entry === "string");
  if (!v || !strings(v.translations) || !strings(v.tags) || typeof v.summary !== "string") return null;
  return { translations: v.translations, tags: v.tags, summary: v.summary };
}

/**
 * `cacheDir` is the job's `data/tr`. `dailyCap` is the site's
 * `limits.translate_tokens_per_day`: the tokens the usage log has for today
 * (UTC) count against it. Failures are SiteErrors: `config` (no model, quota
 * or permission), `budget` (the run's or the day's tokens are used up) and
 * `transient` (this article is left for the next sync).
 */
export function createTranslator(options: {
  model: MeteredModel;
  cacheDir: string;
  dailyCap: number;
  now?: () => number;
}): Translator {
  const now = options.now ?? Date.now;

  const ask = async (job: TranslateJob, texts: string[], writeSummary: boolean) => {
    const spent = options.model.spentSince(utcDayStart(now()));
    if (spent >= options.dailyCap) {
      throw new SiteError("budget", `today's translation budget is used up (${spent} of ${options.dailyCap} tokens)`);
    }
    const request = {
      source_language: job.from,
      target_language: job.to,
      topics: job.topics,
      write_summary: writeSummary,
      texts,
      ...(texts.length ? {} : { article: job.texts.join("\n").slice(0, EXCERPT_CHARS) }),
    };
    let answer;
    try {
      answer = await options.model.ask({
        model: TRANSLATE_MODEL,
        system: SYSTEM,
        user: JSON.stringify(request),
        schema: SCHEMA,
        traceId: `site:${job.id}`,
      });
    } catch (err) {
      if (err instanceof ModelError && (err.kind === "config" || err.kind === "budget")) {
        throw new SiteError(err.kind, err.message);
      }
      throw new SiteError("transient", (err as Error).message);
    }
    return answerOf(answer.text);
  };

  return {
    async translate(job) {
      const file = `${options.cacheDir}/${job.id}.json`;
      let cache: Cache = { schema: CACHE_SCHEMA, language: job.to, t: {} };
      try {
        const kept = JSON.parse(await Deno.readTextFile(file)) as Cache;
        if (kept.schema === CACHE_SCHEMA && kept.language === job.to && kept.t) cache = kept;
      } catch {
        // nothing kept for this article
      }

      const translate = !sameLanguage(job.from, job.to);
      const hashes = await Promise.all(job.texts.map(sha256));
      const missing: string[] = [];
      const missingHashes: string[] = [];
      if (translate) {
        job.texts.forEach((text, index) => {
          if (text.trim() && cache.t[hashes[index]] === undefined && !missingHashes.includes(hashes[index])) {
            missing.push(text);
            missingHashes.push(hashes[index]);
          }
        });
      }
      const wantTags = job.topics.length > 0 && cache.tags === undefined;
      const wantSummary = !job.summary.trim() && cache.summary === undefined;

      if (missing.length || wantTags || wantSummary) {
        let answer = await ask(job, missing, wantSummary);
        if (!answer || answer.translations.length !== missing.length) answer = await ask(job, missing, wantSummary);
        if (!answer || answer.translations.length !== missing.length) {
          throw new SiteError(
            "transient",
            answer
              ? `the model returned ${answer.translations.length} translations for ${missing.length} texts, twice`
              : "the model did not answer with the JSON asked for, twice",
          );
        }
        missingHashes.forEach((hash, index) => cache.t[hash] = answer.translations[index]);
        if (wantTags) cache.tags = answer.tags;
        if (wantSummary) cache.summary = answer.summary.trim();
        await Deno.mkdir(options.cacheDir, { recursive: true });
        await Deno.writeTextFile(file, JSON.stringify(cache));
      }

      return {
        texts: job.texts.map((text, index) => translate && text.trim() ? cache.t[hashes[index]] : text),
        tags: [...new Set(cache.tags ?? [])].filter((tag) => job.topics.includes(tag)).slice(0, MAX_TAGS),
        summary: job.summary.trim() ? "" : cache.summary ?? "",
      };
    },
  };
}
