# Browser scripts and diagnostics

Each command takes control and releases it when finished. Between commands the user can change the page. Use a snapshot to choose locators by role, label or text. Keep the script in the session working directory. Playwright comes with the command and actions wait up to 30 seconds.

```js
export default async ({ page, args }) => {
  await page.goto(args.url);
  await page.getByLabel('Name').fill(args.name);
  await page.getByLabel('City').fill(args.city);
  await page.getByRole('button', { name: 'Submit' }).click();
  const answer = page.getByRole('status');
  await answer.waitFor();
  return { url: page.url(), title: await page.title(), answer: await answer.innerText() };
};
```
```sh
agent-browser run fill.mjs --arg url=https://www.example.com/form --arg name="Li Lei" --arg city=Shanghai
```

`page` is the current tab (created if needed), `pages` contains visible tabs, `args` are strings. Open tabs with `context.newPage()` and close your own when finished to stay within the tab limit. Use `pages` instead of `context.pages()`, which includes internal refresh tabs. Do not attach to the debugging port or start another browser on this profile. Leave the viewport, user agent, device emulation and navigator values as the owner set them.

For a screenshot in a script, `await page.bringToFront()` before `page.screenshot({ scale: 'css' })`. Handle an alert or confirm with `page.on('dialog', …)` if its answer matters. Unhandled dialogs are dismissed. Native permission and passkey prompts require the user on a headed display. Headless macOS/Windows refuses them.

#### Failed startup

Use `agent-browser --help` or `agent-browser <command> --help` for syntax. Exit 0 is success, 1 is failure (last stderr line), 2 is usage, 3 means no browser for this session. Diagnose and report the reason. A browser you launch yourself has separate sign-ins and cannot be watched or taken over in the app.

| Message | Meaning and action |
| --- | --- |
| command not found | Owner enables ext-core in Desktop Settings › Extension tools or with `buckyos ext-tools enable ext-core`. |
| `OPENDAN_BROWSER_RPC is not set` | Only sessions on the Agent's private runtime reach the browser. |
| Chromium not downloaded (`chromium --version` exits 127) | Enable networking and download/retry Chromium in Extension tools or with `buckyos ext-tools download ext-core chromium`. No restart needed. |
| No `chromium` on this node | ext-core is missing or older than the browser. |
| Switched off for this agent | Owner set `[loader.browser] enabled = false`. |
| Cannot reach the Agent Service | Service unavailable. Report this and stop. |

Sources: `$BUCKYOS_ROOT/tools/store/ext-core/browser/agent-browser.mjs`, buckyos-agent's `commands/reflow.ts` and `lib/reflow.ts`, and repository `doc/opendan/protocol/Agent Browser Protocol.md`.

Previous verification (ext-core 1.3.0, 2026-11-15): script against Chromium with a stand-in service, form, missing-field question, password handover and reflow against a Zone. Chromium's pre-download message was checked on the payload. Node download was untested. Shared tabs and browser conversation had Agent Service tests only.
