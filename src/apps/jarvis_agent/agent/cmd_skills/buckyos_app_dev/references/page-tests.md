# Page tests with Playwright

Extended runtimes get a full Chromium from the ext-core online item. In a fresh session after download, `PLAYWRIGHT_BROWSERS_PATH` selects the installed browser. Run `agent-browser version` and set `PLAYWRIGHT_VERSION` to its reported playwright-core version.

```sh
npm i -D "@playwright/test@${PLAYWRIGHT_VERSION:?set the reported playwright-core version}"
npx playwright test
```

Set `use: { channel: 'chromium' }` globally or in each Chromium project. A missing chromium_headless_shell means the config requested the headless shell; use the included full Chromium.

If `chromium --version` exits 127, PLAYWRIGHT_BROWSERS_PATH is unset and the expected system revision is missing, the owner must download the online item in Desktop Settings → Extension tools or via `buckyos ext-tools download ext-core chromium`. Start a fresh session after download.

If the project deliberately pins a different Playwright revision, give its browser the package libraries/fonts in a subshell:

```sh
( eval "$(agent-browser env)"; npx playwright install chromium && npx playwright test )
```

Historical checks: ext-core 1.3.0 in ubuntu:24.04 with Playwright 1.65.1/1.63.0 (2026-11-15); online layout tested with playwright-core 1.65.1, full browser placed in tools/online/chromium and pre-download command. Live node download and Agent session were unverified.
