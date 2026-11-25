import { bodyOf, checkBatch, nextLengths } from "./check.ts";
import { assertEquals } from "./testing.ts";
import type { RawArticle } from "./types.ts";

const text = (seed: string, chars: number) => seed.repeat(Math.ceil(chars / seed.length)).slice(0, chars);

function article(n: number, patch: Partial<RawArticle> = {}, chars = 600): RawArticle {
  return {
    url: `https://news.example.com/a/${n}`,
    title: `Title ${n}`,
    blocks: [
      { type: "img", url: "https://img.example.com/a.jpg", alt: "not body text ".repeat(100) },
      { type: "h2", text: `Heading ${n}` },
      { type: "p", text: text(`Paragraph ${n} of the body. `, chars) },
      { type: "table", rows: [["not", "counted"]] },
    ],
    ...patch,
  };
}

const LIST = [{ url: "https://news.example.com/a/1", title: "Title 1" }];
const base = { list: LIST, listedBefore: 8, notModified: false, lengths: [] as number[] };

Deno.test("a batch of good articles passes", () => {
  const items = [article(1), article(2), article(3)];
  const report = checkBatch({ ...base, items });
  assertEquals([report.accepted.length, report.rejected, report.failure, report.min_chars], [3, [], null, 200]);
  assertEquals(bodyOf(items[0]).startsWith("Heading 1\nParagraph 1"), true, "the body is the text blocks, nothing else");
});

Deno.test("rule: a list that had entries and is empty now fails the whole batch, unless a 304 said nothing changed", () => {
  const empty = checkBatch({ ...base, list: [], items: [] });
  assertEquals(empty.failure, "the list had 8 entries last time and is empty now");
  assertEquals(checkBatch({ ...base, list: [], items: [], notModified: true }).failure, null);
  assertEquals(checkBatch({ ...base, list: [], items: [], listedBefore: 0 }).failure, null, "a first, empty run is not a failure");
});

Deno.test("rule: an article without a title is not published", () => {
  const report = checkBatch({ ...base, items: [article(1), article(2, { title: "  " }), article(3)] });
  assertEquals(report.accepted.map((item) => item.url), ["https://news.example.com/a/1", "https://news.example.com/a/3"]);
  assertEquals(report.rejected, [{ url: "https://news.example.com/a/2", title: "  ", reason: "the title is empty" }]);
  assertEquals(report.failure, null);
});

Deno.test("rule: a body shorter than max(min_chars, median of the last 30 x median_ratio) is not published", () => {
  const short = checkBatch({ ...base, items: [article(1), article(2, {}, 150), article(3)] });
  assertEquals(short.rejected.map((entry) => entry.reason), ["the body has 159 characters, fewer than 200"]);

  // The source usually runs to 3000 characters: 600 is suspicious there (3000 x 0.3 = 900).
  const lengths = Array(30).fill(3000);
  const byMedian = checkBatch({ ...base, lengths, items: [article(1, {}, 3000), article(2), article(3, {}, 3000)] });
  assertEquals([byMedian.min_chars, byMedian.rejected.length], [900, 1]);

  // A source that states its own thresholds: a list of leads.
  const leads = checkBatch({ ...base, check: { min_chars: 40 }, items: [article(1, {}, 60), article(2, {}, 60)] });
  assertEquals(leads.rejected, []);
  const strict = checkBatch({ ...base, lengths, check: { median_ratio: 0.1 }, items: [article(1), article(2)] });
  assertEquals([strict.min_chars, strict.rejected.length], [300, 0]);
});

Deno.test("rule: a body with more than max_replacement of U+FFFD is not published", () => {
  const garbled = article(2);
  garbled.blocks = [{ type: "p", text: text("��� text ", 600) }];
  const few = article(3);
  few.blocks = [{ type: "p", text: `${text("fine text ", 600)}��` }];
  const report = checkBatch({ ...base, items: [article(1), garbled, few] });
  assertEquals(report.accepted.map((item) => item.url), ["https://news.example.com/a/1", "https://news.example.com/a/3"]);
  assertEquals(report.rejected[0].reason.includes("U+FFFD"), true);
  const tolerant = checkBatch({ ...base, check: { max_replacement: 0.5 }, items: [article(1), garbled] });
  assertEquals(tolerant.rejected, []);
});

Deno.test("rule: three or more articles with the very same body are all left out", () => {
  const same = (n: number) => article(n, { blocks: [{ type: "p", text: text("Please enable JavaScript. ", 400) }] });
  const report = checkBatch({ ...base, items: [article(1), article(2), article(3), article(4), same(5), same(6), same(7)] });
  assertEquals(report.accepted.length, 4);
  assertEquals(report.rejected.map((entry) => entry.reason), Array(3).fill("3 articles of this run have this same body"));
  assertEquals(report.failure, null, "3 of 7 is not more than half");
  const pair = checkBatch({ ...base, items: [article(1), same(5), same(6)] });
  assertEquals(pair.rejected, [], "two alike are a coincidence");
});

Deno.test("rule: more than half of a run rejected (at least 2) fails the whole batch", () => {
  const bad = (n: number) => article(n, {}, 50);
  const failed = checkBatch({ ...base, items: [article(1), bad(2), bad(3)] });
  assertEquals(failed.failure?.startsWith("2 of 3 articles did not pass the check"), true, failed.failure ?? "");
  assertEquals(checkBatch({ ...base, items: [bad(1)] }).failure, null, "one bad article alone is not a broken extractor");
  assertEquals(checkBatch({ ...base, items: [article(1), article(2), bad(3), bad(4)] }).failure, null, "half is not more than half");
});

Deno.test("the length history keeps the last 30 accepted bodies", () => {
  const lengths = nextLengths(Array(29).fill(100), [article(1), article(2)]);
  assertEquals([lengths.length, lengths[0], lengths.at(-1)], [30, 100, bodyOf(article(2)).length]);
});
