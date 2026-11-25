import { compactReading, recentReading } from "./reading.ts";
import { SiteBucket } from "./store.ts";
import { assertEquals, fixture, memoryStore, tempDir } from "./testing.ts";
import { articleKey, READING_KEY } from "./types.ts";
import type { Reading } from "./types.ts";

const DAY = 86_400_000;
const NOW = 1764003600000;
const A = "7eec8758d56a244a";
const B = "81c5e462be246971";
const C = "8064031d77568dc9";
const D = "01eabe90c23c8fe3";

const record = (article: string, opened_at: number, dwell_ms: number, max_scroll: number, source = "news-yahoo-co-jp") => ({
  article,
  source,
  opened_at,
  dwell_ms,
  max_scroll,
});

Deno.test("the reports of the pages are folded into reading.json, each batch once and each opening once", async () => {
  const batch = await fixture("site/read_batch.json") as unknown[];
  const memory = memoryStore({
    "_read/20261124/01JDA00000000000000000000A.json": JSON.stringify(batch),
    "_read/20261124/01JDA00000000000000000000B.json": JSON.stringify([
      record(A, NOW - 3000, 42000, 60),
      record(B, NOW - 2000, 9000, 35),
      { article: "nope", source: "x", opened_at: 1, dwell_ms: 1, max_scroll: 1 },
      record(B, NOW, 1, 101),
    ]),
    "_read/20261124/01JDA00000000000000000000C.json": "not json",
    [articleKey(A)]: JSON.stringify({ tags: ["科技", "经济"] }),
    [articleKey(B)]: JSON.stringify({ tags: ["社会"] }),
  });
  await memory.seed();
  const bucket = new SiteBucket(memory.store, "b");
  const data = await tempDir();
  try {
    const first = await compactReading(bucket, data, NOW);
    assertEquals(first, {
      batches: 3,
      articles: 2,
      metrics: {
        records: 3,
        opens_7d: 3,
        dwell_ms_7d: 93000,
        articles_7d: 2,
        profile: "tags:科技,经济,社会;sources:news-yahoo-co-jp",
      },
    });
    assertEquals(memory.json(READING_KEY), {
      schema: "buckyos.site.reading/1",
      updated_at: NOW,
      since: NOW - 90 * DAY,
      articles: {
        [A]: { source: "news-yahoo-co-jp", opens: 2, dwell_ms: 84000, max_scroll: 100, last_opened_at: NOW - 3000 },
        [B]: { source: "news-yahoo-co-jp", opens: 1, dwell_ms: 9000, max_scroll: 35, last_opened_at: NOW - 2000 },
      },
      sources: { "news-yahoo-co-jp": { opens: 3, dwell_ms: 93000, last_opened_at: NOW - 2000 } },
      tags: { "科技": { opens: 2, dwell_ms: 84000 }, "经济": { opens: 2, dwell_ms: 84000 }, "社会": { opens: 1, dwell_ms: 9000 } },
    });

    // An hour later the page reports more of an opening it reported before, and a new one. The old batches are
    // not counted twice, the tags are not read again.
    await memory.write(
      "_read/20261125/01JDB00000000000000000000A.json",
      JSON.stringify([record(B, NOW - 2000, 3000, 80), record(B, NOW + 1000, 1000, 20)]),
    );
    memory.objects.delete(articleKey(B));
    const second = await compactReading(bucket, data, NOW + 3_600_000);
    assertEquals([second.batches, second.metrics.records, second.metrics.opens_7d, second.metrics.dwell_ms_7d], [1, 2, 4, 97000]);
    const next = memory.json(READING_KEY) as Reading;
    assertEquals(next.articles[B], { source: "news-yahoo-co-jp", opens: 2, dwell_ms: 13000, max_scroll: 80, last_opened_at: NOW + 1000 });
    assertEquals([next.articles[A].opens, next.tags["社会"]], [2, { opens: 2, dwell_ms: 13000 }]);

    // Nothing new: the same numbers, the same profile.
    const idle = await compactReading(bucket, data, NOW + 7_200_000);
    assertEquals(idle.metrics, { ...second.metrics, records: 0 });

    // A job that starts over (no state) folds everything anew and arrives at the same summary.
    const fresh = await tempDir();
    try {
      const again = await compactReading(bucket, fresh, NOW + 7_200_000);
      assertEquals([again.batches, again.metrics.records, again.metrics.profile], [4, 5, "tags:科技,经济;sources:news-yahoo-co-jp"]);
      const rebuilt = memory.json(READING_KEY) as Reading;
      assertEquals([rebuilt.articles, rebuilt.sources], [next.articles, next.sources]);
    } finally {
      await Deno.remove(fresh, { recursive: true });
    }

    // After 7 days the openings leave the profile, after 90 the summary.
    const week = await compactReading(bucket, data, NOW + 8 * DAY);
    assertEquals(week.metrics, { records: 0, opens_7d: 0, dwell_ms_7d: 0, articles_7d: 0, profile: "tags:;sources:" });
    assertEquals(Object.keys((memory.json(READING_KEY) as Reading).articles), [A, B]);
    const later = await compactReading(bucket, data, NOW + 90 * DAY + 500);
    assertEquals([later.articles, Object.keys((memory.json(READING_KEY) as Reading).articles)], [1, [B]]);

    const recent = recentReading(next, 7, NOW + 3_600_000);
    assertEquals(recent.articles.map((entry) => entry.id), [B, A]);
    assertEquals(recentReading(next, 7, NOW + 30 * DAY).articles, []);
  } finally {
    await Deno.remove(data, { recursive: true });
  }
});

Deno.test("the profile names the three most read tags and sources, the same way every time", async () => {
  const memory = memoryStore({
    "_read/20261124/01JDA00000000000000000000A.json": JSON.stringify([
      record(A, NOW - 1000, 5000, 50, "source-b"),
      record(B, NOW - 2000, 5000, 50, "source-a"),
      record(C, NOW - 3000, 9000, 50, "source-c"),
      record(D, NOW - 4000, 100, 50, "source-d"),
      record(D, NOW - 5000, 0, 0, "source-e"),
    ]),
    [articleKey(A)]: JSON.stringify({ tags: ["b", "shared"] }),
    [articleKey(B)]: JSON.stringify({ tags: ["a", "shared"] }),
    [articleKey(C)]: JSON.stringify({ tags: ["c"] }),
    [articleKey(D)]: JSON.stringify({ tags: ["d"] }),
  });
  await memory.seed();
  const data = await tempDir();
  try {
    const report = await compactReading(new SiteBucket(memory.store, "b"), data, NOW);
    assertEquals(report.metrics, {
      records: 5,
      opens_7d: 5,
      dwell_ms_7d: 19100,
      articles_7d: 4,
      // shared 10000, c 9000, then a and b at 5000 each: the name decides.
      profile: "tags:shared,c,a;sources:source-c,source-a,source-b",
    });
  } finally {
    await Deno.remove(data, { recursive: true });
  }
});
