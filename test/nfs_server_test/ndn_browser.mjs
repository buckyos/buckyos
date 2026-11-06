/**
 * What a real browser does with user content served on the Desktop origin (bucket design
 * §21 N2, N9): active content does not run on `/bucket/v1`, `/home/.../content`,
 * `/nfs/v1/read` or `/ndn/`; `<video>` seeks over `/ndn/`. Playwright's Chromium, from Desktop's
 * `node_modules`, the full browser: Playwright's default headless shell has no PDF viewer and
 * downloads every PDF, whatever the headers say (`chromium.executablePath()` names the full
 * browser either way; `DEBUG=pw:browser` shows what really started).
 *
 * Usage: node ndn_browser.mjs <urls.json>; entries {name, url, expect}, expect one of `no_run`
 * (the page must not execute), `runs` (a control that it would notice), `renders`, `seek` (with
 * `origin`, a plain page to host the `<video>`), `pdf` (shown: red pixels of the fixture on
 * screen, because a PDF's text is in neither the DOM nor the accessibility tree; with `origin`,
 * inside an `<iframe>` as Desktop's renderer does) and `no_pdf` (a control for that measure).
 * Exits 1 on a failure.
 */
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const desktop = new URL("../../src/frame/desktop/", import.meta.url).pathname;
const { chromium } = createRequire(`${desktop}package.json`)("@playwright/test");
const cases = JSON.parse(readFileSync(process.argv[2], "utf8"));
const browser = await chromium.launch({ channel: "chromium", headless: true });
const context = await browser.newContext({ ignoreHTTPSErrors: true, acceptDownloads: true });
let failed = 0;
const report = (ok, name, detail) => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? `: ${detail}` : ""}`);
  if (!ok) failed++;
};

async function redShare(png) {
  const page = await context.newPage();
  await page.goto("data:text/html,<body>");
  const share = await page.evaluate(async (b64) => {
    const bitmap = await createImageBitmap(await (await fetch(`data:image/png;base64,${b64}`)).blob());
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const context2d = canvas.getContext("2d");
    context2d.drawImage(bitmap, 0, 0);
    const data = context2d.getImageData(0, 0, bitmap.width, bitmap.height).data;
    let red = 0;
    for (let i = 0; i < data.length; i += 4) if (data[i] > 200 && data[i + 1] < 60 && data[i + 2] < 60) red++;
    return red / (bitmap.width * bitmap.height);
  }, png.toString("base64"));
  await page.close();
  return share;
}

async function runCase(c) {
  const page = await context.newPage();
  let dialog = false;
  page.on("dialog", (d) => { dialog = true; void d.dismiss(); });
  let downloaded = false;
  page.on("download", () => { downloaded = true; });
  if (c.expect === "seek") {
    // A page of its own: `about:blank` cannot load http media, a sandboxed one cannot run this.
    await page.goto(c.origin, { waitUntil: "domcontentloaded" });
    const outcome = await page.evaluate(async (src) => {
      const video = document.createElement("video");
      video.muted = true;
      video.src = src;
      document.body.append(video);
      const event = (name) => new Promise((resolve, reject) => {
        video.addEventListener(name, resolve, { once: true });
        video.addEventListener("error", () => reject(new Error(`video error ${video.error?.code} ${video.error?.message}`)), { once: true });
        setTimeout(() => reject(new Error(`no ${name}`)), 30000);
      });
      await event("loadedmetadata");
      const duration = video.duration;
      video.currentTime = duration - 1;
      await event("seeked");
      return { duration, at: video.currentTime };
    }, c.url).catch((e) => ({ error: String(e) }));
    report(!outcome.error && outcome.at > 1, c.name, JSON.stringify(outcome));
    await page.close();
    return;
  }
  if (c.expect === "pdf" || c.expect === "no_pdf") {
    if (c.origin) {
      await page.goto(c.origin, { waitUntil: "domcontentloaded" });
      await page.evaluate((src) => {
        const frame = document.createElement("iframe");
        frame.src = src;
        frame.referrerPolicy = "no-referrer";
        frame.style.cssText = "position:fixed;inset:0;width:100vw;height:100vh;border:0;background:white;z-index:99999";
        document.body.append(frame);
      }, c.url);
    } else {
      await page.goto(c.url, { waitUntil: "load", timeout: 15000 }).catch(() => {});
    }
    await page.waitForTimeout(3000);
    const share = await redShare(await page.screenshot());
    report(c.expect === "pdf" ? share > 0.1 && !downloaded : share < 0.01, c.name, `red=${share.toFixed(3)} downloaded=${downloaded}`);
    await page.close();
    return;
  }
  const response = await page.goto(c.url, { waitUntil: "load", timeout: 15000 }).catch((e) => ({ aborted: String(e) }));
  await page.waitForTimeout(500);
  const ran = await page.evaluate(() => ({ title: document.title, flagged: Boolean(window.__ran) })).catch(() => ({ title: "", flagged: false }));
  const executed = dialog || ran.flagged || ran.title === "RAN";
  if (c.expect === "runs") {
    report(executed, c.name);
  } else if (c.expect === "no_run") {
    report(!executed, c.name, `downloaded=${downloaded} status=${response?.status?.() ?? response?.aborted?.slice(0, 40)}`);
  } else if (c.expect === "renders") {
    report(response?.status?.() === 200 && !executed && !downloaded, c.name, `content-type=${response?.headers?.()["content-type"]}`);
  }
  await page.close();
}

for (const c of cases) {
  await runCase(c).catch((e) => report(false, c.name, e.message.split("\n")[0]));
}
await browser.close();
process.exit(failed ? 1 : 0);
