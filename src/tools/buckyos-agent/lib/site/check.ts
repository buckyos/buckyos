// Checking what an extractor returned before any of it is published:
// Personal Site Protocol §5.3.

import type { ListEntry, RawArticle, SourceDef } from "./types.ts";

export const CHECK_DEFAULTS = { min_chars: 200, median_ratio: 0.3, max_replacement: 0.01 };
/** How many body lengths the median is taken of. */
export const LENGTH_HISTORY = 30;
const SAME_BODY = 3;

/** The body of an article: the original text of its text blocks. */
export function bodyOf(article: RawArticle): string {
  return article.blocks.flatMap((block) => "text" in block ? [block.text.trim()] : []).filter(Boolean).join("\n");
}

function median(values: number[]): number {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = sorted.length >> 1;
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

export interface CheckInput {
  list: ListEntry[];
  items: RawArticle[];
  /** How many entries the list had the last time it passed. */
  listedBefore: number;
  /** A conditional request of this run was answered 304. */
  notModified: boolean;
  /** Body lengths of the articles accepted lately, oldest first. */
  lengths: number[];
  check?: SourceDef["check"];
}

export interface CheckReport {
  accepted: RawArticle[];
  rejected: { url: string; title: string; reason: string }[];
  /** Why the whole batch fails (`program`); `null` when it passes. */
  failure: string | null;
  /** The shortest body this run accepted. */
  min_chars: number;
}

export function checkBatch(input: CheckInput): CheckReport {
  const limits = { ...CHECK_DEFAULTS, ...input.check };
  const minChars = Math.max(limits.min_chars, Math.round(median(input.lengths) * limits.median_ratio));
  const report: CheckReport = { accepted: [], rejected: [], failure: null, min_chars: minChars };
  if (input.listedBefore > 0 && input.list.length === 0 && !input.notModified) {
    report.failure = `the list had ${input.listedBefore} entries last time and is empty now`;
    return report;
  }

  const bodies = input.items.map(bodyOf);
  const copies = new Map<string, number>();
  for (const body of bodies) copies.set(body, (copies.get(body) ?? 0) + 1);
  input.items.forEach((item, index) => {
    const body = bodies[index];
    const replaced = body.split("�").length - 1;
    let reason: string | null = null;
    if (!item.title.trim()) {
      reason = "the title is empty";
    } else if (body.length < minChars) {
      reason = `the body has ${body.length} characters, fewer than ${minChars}`;
    } else if (replaced > body.length * limits.max_replacement) {
      reason = `${replaced} of the ${body.length} characters of the body are U+FFFD: the page was read in the wrong encoding`;
    } else if ((copies.get(body) ?? 0) >= SAME_BODY) {
      reason = `${copies.get(body)} articles of this run have this same body`;
    }
    if (reason) report.rejected.push({ url: item.url, title: item.title, reason });
    else report.accepted.push(item);
  });
  if (report.rejected.length >= 2 && report.rejected.length * 2 > input.items.length) {
    report.failure = `${report.rejected.length} of ${input.items.length} articles did not pass the check: ${
      report.rejected.slice(0, 3).map((entry) => `${entry.url} (${entry.reason})`).join("; ")
    }`;
  }
  return report;
}

/** The length history after a run that passed. */
export function nextLengths(lengths: number[], accepted: RawArticle[]): number[] {
  return [...lengths, ...accepted.map((item) => bodyOf(item).length)].slice(-LENGTH_HISTORY);
}
