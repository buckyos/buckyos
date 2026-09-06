# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: pages/desktop.spec.ts >> window modal only blocks its owner window
- Location: tests/e2e/pages/desktop.spec.ts:599:1

# Error details

```
Test timeout of 30000ms exceeded.
```

```
Error: locator.selectOption: Test timeout of 30000ms exceeded.
Call log:
  - waiting for getByRole('combobox', { name: 'Language' })

```

# Page snapshot

```yaml
- main [ref=e3]:
  - generic [ref=e5]:
    - generic:
      - button
      - complementary:
        - generic:
          - generic:
            - paragraph: waterflier
            - paragraph: Browser
            - generic: Relay
          - button:
            - generic: Desktop
          - generic:
            - button:
              - generic: Settings
            - button:
              - generic: Diagnostics
            - button:
              - generic: Users & Agents
            - button:
              - generic: My Network
          - button:
            - generic: Log out
    - generic "Status bar":
      - generic:
        - generic:
          - button "BuckyOS" [ref=e6] [cursor=pointer]: B
          - generic: Relay
        - generic:
          - generic: "2"
          - button "Desktop Tips" [ref=e7] [cursor=pointer]:
            - generic [ref=e12]: "8"
          - generic: 08:38 AM
    - generic [ref=e14]:
      - generic [ref=e15]:
        - generic [ref=e18]:
          - generic [ref=e21]:
            - generic [ref=e22]: Sat
            - generic [ref=e24]:
              - paragraph [ref=e25]: 08:38 AM
              - paragraph [ref=e26]: Sep 5, 2026
          - button "Settings" [ref=e29]
          - button "Files" [ref=e37]
          - button "Studio" [ref=e44]
          - button "Marketplace" [ref=e51]
          - button "Docs" [ref=e60]
          - button "Demos" [ref=e67]
          - button "CodeAssistant" [ref=e73]
          - button "MessageHub" [ref=e81]
          - button "AI Center" [ref=e88]
          - button "HomeStation" [ref=e105]
          - button "Systest" [ref=e113]
          - button "AI Canvas" [ref=e120]
          - button "Preview" [ref=e128]
          - button [ref=e140] [cursor=pointer]:
            - paragraph [ref=e142]: Review drag semantics, dead zone behavior, and window polish.
        - generic [ref=e145]:
          - button "Diagnostics" [ref=e148]
          - button "Users & Agents" [ref=e158]
          - button "Task Center" [ref=e168]
          - button "My Network" [ref=e176]
          - button "App Service" [ref=e186]
          - button "Workflow" [ref=e201]
      - generic [ref=e208]:
        - generic [ref=e209] [cursor=pointer]
        - generic [ref=e210] [cursor=pointer]
    - generic:
      - generic [ref=e211]:
        - generic [ref=e212]:
          - paragraph [ref=e216]: Demos
          - generic [ref=e217]:
            - button "Minimize" [ref=e218] [cursor=pointer]
            - button "Maximize" [ref=e220] [cursor=pointer]
            - button "Close" [ref=e223] [cursor=pointer]
        - generic [ref=e228]:
          - generic [ref=e230]:
            - generic [ref=e231]:
              - paragraph [ref=e232]: Controls
              - paragraph [ref=e233]: Control gallery
              - paragraph [ref=e234]: Use one panel to inspect common controls, interactive states, and shell-friendly spacing.
            - generic [ref=e236]:
              - generic [ref=e240]:
                - paragraph [ref=e241]: Demos
                - paragraph [ref=e242]: A compact playground for buttons, fields, toggles, tabs, and feedback surfaces.
              - button "Window modal" [active] [ref=e243] [cursor=pointer]
          - generic [ref=e244]:
            - generic [ref=e245]:
              - generic [ref=e246]: Readiness
              - generic [ref=e247]: 75%
            - generic [ref=e248]:
              - generic [ref=e249]: Launch mode
              - generic [ref=e250]: Windowed
            - generic [ref=e251]:
              - generic [ref=e252]: Menu state
              - generic [ref=e253]: Closed
          - generic [ref=e254]:
            - generic [ref=e255]:
              - generic [ref=e256]:
                - generic [ref=e257]:
                  - paragraph [ref=e258]: Actions
                  - paragraph [ref=e259]: Button hierarchy, icon buttons, and contextual menus should stay readable in both themes.
                - generic [ref=e260]:
                  - generic [ref=e261]:
                    - button [ref=e262] [cursor=pointer]
                    - button "Secondary action" [ref=e266] [cursor=pointer]
                    - button "Text action" [ref=e267] [cursor=pointer]
                    - button "Disabled" [disabled]
                    - button [ref=e268] [cursor=pointer]
                    - button "Fullscreen modal" [ref=e272] [cursor=pointer]
                  - generic [ref=e273]:
                    - button "Search query" [ref=e274] [cursor=pointer]
                    - button "Launch mode" [ref=e278] [cursor=pointer]
                    - button "Light" [ref=e282] [cursor=pointer]
                    - button "Dark" [ref=e289] [cursor=pointer]
                  - generic [ref=e292]:
                    - generic [ref=e293]: "Theme: Light"
                    - generic [ref=e295]: "Language: English"
                    - generic [ref=e297]: "Release state: In review"
                    - generic [ref=e299]: "Modal state: Idle"
                    - generic [ref=e301]: "Fullscreen permission: Denied"
                    - generic [ref=e303]: "Fullscreen request: Idle"
                  - generic [ref=e305]:
                    - generic [ref=e306]:
                      - generic [ref=e307]:
                        - paragraph [ref=e308]: Sudo permission
                        - paragraph [ref=e309]: Request a short-lived sudo token through Verify Hub and keep the returned token in the current operation.
                      - button [ref=e310] [cursor=pointer]
                    - generic [ref=e315]: "Sudo state: Idle"
              - generic [ref=e318]:
                - generic [ref=e319]:
                  - paragraph [ref=e320]: Inputs
                  - paragraph [ref=e321]: Validate field rhythm, select styling, and multiline editing without leaving the desktop shell.
                - generic [ref=e323]:
                  - generic [ref=e324]:
                    - generic [ref=e325]: Search query
                    - generic [ref=e326]:
                      - textbox "Search query" [ref=e327]: Window controls
                      - group:
                        - generic: Search query
                  - generic [ref=e328]:
                    - generic [ref=e329]: Owner
                    - generic [ref=e330]:
                      - textbox "Owner" [ref=e331]: Prototype team
                      - group:
                        - generic: Owner
                  - generic [ref=e332]:
                    - generic [ref=e333]: Density
                    - generic [ref=e334]:
                      - combobox "Density" [ref=e335] [cursor=pointer]:
                        - option "Compact"
                        - option "Balanced" [selected]
                        - option "Comfortable"
                      - group:
                        - generic: Density
                  - generic [ref=e336]:
                    - generic [ref=e337]: Release state
                    - generic [ref=e338]:
                      - combobox "Release state" [ref=e339] [cursor=pointer]:
                        - option "Draft"
                        - option "In review" [selected]
                        - option "Ready"
                      - group:
                        - generic: Release state
                  - generic [ref=e341]:
                    - generic [ref=e342]: Notes
                    - generic [ref=e343]:
                      - textbox "Notes" [ref=e344]: Validate spacing rhythm, focus rings, disabled states, and mobile fit.
                      - group:
                        - generic: Notes
            - generic [ref=e345]:
              - generic [ref=e346]:
                - generic [ref=e347]:
                  - paragraph [ref=e348]: Selection
                  - paragraph [ref=e349]: Switches, checkboxes, radios, and sliders cover the most common mutable states.
                - generic [ref=e351]:
                  - generic [ref=e352]:
                    - generic [ref=e354] [cursor=pointer]:
                      - switch "Enable notifications" [checked] [ref=e357]
                      - generic [ref=e360]: Enable notifications
                    - generic [ref=e361]:
                      - generic [ref=e362] [cursor=pointer]:
                        - checkbox "Prepare offline cache" [ref=e364]
                        - generic [ref=e367]: Prepare offline cache
                      - generic [ref=e368] [cursor=pointer]:
                        - checkbox "Auto arrange cards" [checked] [ref=e370]
                        - generic [ref=e373]: Auto arrange cards
                    - generic [ref=e374]:
                      - paragraph [ref=e375]: Launch mode
                      - radiogroup [ref=e376]:
                        - generic [ref=e377] [cursor=pointer]:
                          - radio "Windowed" [checked] [ref=e379]
                          - generic [ref=e385]: Windowed
                        - generic [ref=e386] [cursor=pointer]:
                          - radio "Maximized" [ref=e388]
                          - generic [ref=e392]: Maximized
                        - generic [ref=e393] [cursor=pointer]:
                          - radio "Focused preview" [ref=e395]
                          - generic [ref=e399]: Focused preview
                  - generic [ref=e400]:
                    - paragraph [ref=e401]: Scale
                    - slider [ref=e407]: "58"
                    - generic [ref=e408]:
                      - paragraph [ref=e409]: Preview scale
                      - paragraph [ref=e410]: 58%
              - generic [ref=e411]:
                - generic [ref=e412]:
                  - paragraph [ref=e413]: Feedback
                  - paragraph [ref=e414]: Alerts, progress, tabs, and summary surfaces make regressions easy to spot.
                - generic [ref=e415]:
                  - tablist [ref=e418]:
                    - tab "Alerts" [selected] [ref=e419] [cursor=pointer]
                    - tab "Status" [ref=e420] [cursor=pointer]
                    - tab "Preview" [ref=e421] [cursor=pointer]
                  - generic [ref=e424]:
                    - alert [ref=e425]:
                      - generic [ref=e429]: Informational state for neutral guidance and feature context.
                    - alert [ref=e430]:
                      - generic [ref=e434]: Success state for saves, sync, and completion feedback.
                    - alert [ref=e435]:
                      - generic [ref=e439]: Warning state for partial support, risky changes, or layout overflow.
                    - alert [ref=e440]:
                      - generic [ref=e444]: Error state for failed loads, blocked actions, or invalid data.
        - dialog "Scoped window modal" [ref=e453]:
          - button "Close dialog" [ref=e454] [cursor=pointer]
          - generic [ref=e458]:
            - heading "Scoped window modal" [level=2] [ref=e459]
            - paragraph [ref=e460]: This dialog should only block input for the demos window that opened it.
          - generic [ref=e462]:
            - paragraph [ref=e463]: Use this as the default modal contract for confirmation, short forms, and in-window interruption flows.
            - generic [ref=e464]: Desktop keeps other windows interactive. Mobile centers the same dialog on the current screen, and can still opt into sheet or fullscreen when needed.
          - generic [ref=e465]:
            - button "Cancel" [ref=e466] [cursor=pointer]
            - button "Apply change" [ref=e467] [cursor=pointer]
      - generic [ref=e468]:
        - generic [ref=e469]:
          - paragraph [ref=e475]: Settings
          - generic [ref=e476]:
            - button "Minimize" [ref=e477] [cursor=pointer]
            - button "Maximize" [ref=e479] [cursor=pointer]
            - button "Close" [ref=e482] [cursor=pointer]
        - generic [ref=e488]:
          - navigation [ref=e489]:
            - searchbox [ref=e494]
            - generic [ref=e495]:
              - button "General" [ref=e496] [cursor=pointer]
              - button "Appearance" [ref=e502] [cursor=pointer]
              - button "Cluster Manager" [ref=e511] [cursor=pointer]
              - button "Privacy" [ref=e519] [cursor=pointer]
              - button "Developer Mode" [ref=e524] [cursor=pointer]
          - main [ref=e530]:
            - generic [ref=e532]:
              - generic [ref=e535]:
                - paragraph [ref=e536]: System
                - paragraph [ref=e537]: General
                - paragraph [ref=e538]: System information, device details, and support tools.
              - generic [ref=e539]:
                - heading "Software Info" [level=3] [ref=e540]
                - generic [ref=e541]:
                  - generic [ref=e542]:
                    - generic [ref=e543]:
                      - generic [ref=e544]: BuckyOS Version
                      - generic [ref=e545]: 1.0.0-rc.3
                    - generic [ref=e546]:
                      - generic [ref=e547]: Build Version
                      - generic [ref=e548]: build-20260401-a3f8c2d
                    - generic [ref=e549]:
                      - generic [ref=e550]: Release Channel
                      - generic [ref=e551]: Beta
                    - generic [ref=e553]:
                      - generic [ref=e554]: Installed
                      - generic [ref=e555]: 3/1/2026, 1:00:00 AM
                    - generic [ref=e556]:
                      - generic [ref=e557]: Last Update
                      - generic [ref=e558]: 3/28/2026, 7:30:00 AM
                  - generic [ref=e559]:
                    - generic [ref=e560]: "New version available: 1.0.0-rc.4"
                    - button [ref=e561] [cursor=pointer]
              - generic [ref=e568]:
                - heading "Device Info" [level=3] [ref=e569]
                - generic [ref=e571]:
                  - generic [ref=e572]:
                    - generic [ref=e573]: Operating System
                    - generic [ref=e574]: macOS macOS 15.4
                  - generic [ref=e575]:
                    - generic [ref=e576]: CPU
                    - generic [ref=e577]: Apple M3 Pro (12 cores)
                  - generic [ref=e578]:
                    - generic [ref=e579]: Memory
                    - generic [ref=e580]: 36 GB
                  - generic [ref=e581]:
                    - generic [ref=e582]: Storage
                    - generic [ref=e583]: 1 TB
              - generic [ref=e584]:
                - heading "System Snapshot" [level=3] [ref=e585]
                - generic [ref=e586]:
                  - generic [ref=e587]:
                    - generic [ref=e588]:
                      - generic [ref=e589]: Install Mode
                      - generic [ref=e590]: Desktop
                    - generic [ref=e591]:
                      - generic [ref=e592]: Node Count
                      - generic [ref=e593]: "1"
                    - generic [ref=e594]:
                      - generic [ref=e595]: Storage Usage
                      - generic [ref=e596]: 256 GB / 1 TB
                  - generic [ref=e597]:
                    - paragraph [ref=e598]: Enabled Modules
                    - generic [ref=e599]:
                      - generic [ref=e600]: File Manager
                      - generic [ref=e601]: AI Center
                      - generic [ref=e602]: Message Hub
                      - generic [ref=e603]: Code Assistant
              - generic [ref=e604]:
                - heading "Support & Debug" [level=3] [ref=e605]
                - paragraph [ref=e606]: Copy system information for troubleshooting.
                - generic [ref=e608]:
                  - button [ref=e609] [cursor=pointer]
                  - button [ref=e614] [cursor=pointer]
```

# Test source

```ts
  520 |       JSON.stringify({
  521 |         user_name: 'Alice',
  522 |         user_id: 'did:buckyos:user:alice',
  523 |         user_type: 'owner',
  524 |         session_token: 'legacy-session-token',
  525 |       }),
  526 |     )
  527 |     document.cookie = 'control-panel_token=session-token; Path=/; SameSite=Lax'
  528 |   })
  529 | 
  530 |   await page.getByRole('button', { name: 'BuckyOS' }).click()
  531 |   await page.getByRole('button', { name: 'Log out' }).click()
  532 |   await page.waitForURL('**/login?**')
  533 | 
  534 |   await expect
  535 |     .poll(() =>
  536 |       page.evaluate(() => ({
  537 |         browserUserInfo: window.localStorage.getItem('user_info'),
  538 |         desktopAccountInfo: window.localStorage.getItem(
  539 |           'buckyos.account_info.control-panel',
  540 |         ),
  541 |         legacyAccountInfo: window.localStorage.getItem('buckyos.account_info'),
  542 |         appCookie: document.cookie
  543 |           .split(';')
  544 |           .some((part) => part.trim().startsWith('control-panel_token=')),
  545 |       })),
  546 |     )
  547 |     .toEqual({
  548 |       browserUserInfo: null,
  549 |       desktopAccountInfo: null,
  550 |       legacyAccountInfo: null,
  551 |       appCookie: false,
  552 |     })
  553 | })
  554 | 
  555 | test('demos app renders common controls', async ({ page }) => {
  556 |   await page.goto('/?scenario=normal')
  557 | 
  558 |   await page.getByTestId('desktop-app-demos').click()
  559 |   await expect(page.getByTestId('window-demos')).toBeVisible()
  560 |   await expect(page.getByText('Control gallery', { exact: true })).toBeVisible()
  561 | 
  562 |   await page.getByRole('button', { name: 'Quick menu' }).click()
  563 |   await expect(page.getByRole('menuitem', { name: 'Pin to launcher' })).toBeVisible()
  564 |   await page.getByRole('menuitem', { name: 'Pin to launcher' }).click()
  565 | 
  566 |   await page.getByRole('textbox', { name: 'Search query' }).fill('State matrix')
  567 |   await expect(page.getByRole('textbox', { name: 'Search query' })).toHaveValue('State matrix')
  568 |   await page.getByRole('button', { name: 'Fullscreen modal' }).click()
  569 |   await expect(page.getByText('Fullscreen request: Denied')).toBeVisible()
  570 |   await page.getByRole('tab', { name: 'Status' }).click()
  571 |   await expect(page.getByText('Control coverage')).toBeVisible()
  572 | })
  573 | 
  574 | test('status tray tips opens from bell and closes on outside click', async ({ page }) => {
  575 |   await page.goto('/?scenario=normal')
  576 | 
  577 |   const tipsButton = page.getByTestId('status-tray-tips-button')
  578 |   await expect(tipsButton).toBeVisible()
  579 |   await tipsButton.click()
  580 | 
  581 |   const tipsPanel = page.getByTestId('status-tips-panel')
  582 |   await expect(tipsPanel).toBeVisible()
  583 |   await expect(page.getByTestId('status-tip-card-recent-shell-action')).toBeVisible()
  584 |   await expect(page.getByTestId('status-tip-card-mobile-touch-audit')).toBeVisible()
  585 | 
  586 |   const panelBox = await tipsPanel.boundingBox()
  587 |   const viewport = page.viewportSize()
  588 |   expect(panelBox).not.toBeNull()
  589 |   expect(viewport).not.toBeNull()
  590 |   expect((panelBox?.x ?? 0) + (panelBox?.width ?? 0)).toBeLessThanOrEqual(
  591 |     (viewport?.width ?? 0) - 8,
  592 |   )
  593 |   expect(panelBox?.x ?? 0).toBeGreaterThanOrEqual(0)
  594 | 
  595 |   await page.mouse.click(24, (viewport?.height ?? 0) - 24)
  596 |   await expect(tipsPanel).toHaveCount(0)
  597 | })
  598 | 
  599 | test('window modal only blocks its owner window', async ({ page }) => {
  600 |   await page.goto('/?scenario=normal')
  601 | 
  602 |   await page.getByTestId('desktop-app-settings').click()
  603 |   await expect(page.getByTestId('window-settings')).toBeVisible()
  604 | 
  605 |   const settingsBeforeDrag = await page.getByTestId('window-settings').boundingBox()
  606 |   await page.getByTestId('window-drag-settings').hover()
  607 |   await page.mouse.down()
  608 |   await page.mouse.move(
  609 |     1180,
  610 |     (settingsBeforeDrag?.y ?? 0) + 90,
  611 |     { steps: 18 },
  612 |   )
  613 |   await page.mouse.up()
  614 | 
  615 |   await page.getByTestId('desktop-app-demos').click()
  616 |   await expect(page.getByTestId('window-demos')).toBeVisible()
  617 |   await page.getByRole('button', { name: 'Window modal' }).first().click()
  618 |   await expect(page.getByRole('dialog', { name: 'Scoped window modal' })).toBeVisible()
  619 | 
> 620 |   await page.getByRole('combobox', { name: 'Language' }).selectOption('ja')
      |                                                          ^ Error: locator.selectOption: Test timeout of 30000ms exceeded.
  621 |   await expect(page.getByRole('combobox', { name: 'Language' })).toHaveValue('ja')
  622 | 
  623 |   await expect(page.getByRole('dialog', { name: 'Scoped window modal' })).toBeVisible()
  624 |   await page
  625 |     .getByTestId('window-settings')
  626 |     .getByRole('button', { name: 'Close' })
  627 |     .click()
  628 |   await expect(page.getByTestId('window-settings')).toHaveCount(0)
  629 |   await expect(page.getByRole('dialog', { name: 'Scoped window modal' })).toBeVisible()
  630 |   await page.getByRole('button', { name: 'Apply change' }).click()
  631 |   await expect(page.getByRole('dialog', { name: 'Scoped window modal' })).toHaveCount(0)
  632 |   await expect(page.getByRole('textbox', { name: 'Owner' })).toHaveValue('Window modal owner')
  633 | })
  634 | 
  635 | test('codeassistant history does not jump back to bottom while scrolling older messages', async ({
  636 |   page,
  637 | }) => {
  638 |   await page.goto('/?scenario=normal')
  639 | 
  640 |   await page.getByTestId('desktop-app-codeassistant').click()
  641 |   const historyPane = page.locator('[data-testid="window-codeassistant"] .shell-scrollbar').first()
  642 |   await expect(historyPane).toBeVisible()
  643 | 
  644 |   await expect.poll(async () => {
  645 |     const { scrollHeight, clientHeight, distanceToBottom } = await getScrollMetrics(historyPane)
  646 |     return scrollHeight > clientHeight ? distanceToBottom : Number.POSITIVE_INFINITY
  647 |   }).toBeLessThanOrEqual(24)
  648 | 
  649 |   await historyPane.hover()
  650 | 
  651 |   let scrolledMetrics = await getScrollMetrics(historyPane)
  652 |   for (let attempt = 0; attempt < 4; attempt += 1) {
  653 |     await page.mouse.wheel(0, -1200)
  654 |     await page.waitForTimeout(60)
  655 |     scrolledMetrics = await getScrollMetrics(historyPane)
  656 |     if (scrolledMetrics.distanceToBottom > 600) {
  657 |       break
  658 |     }
  659 |   }
  660 | 
  661 |   expect(scrolledMetrics.distanceToBottom).toBeGreaterThan(600)
  662 | 
  663 |   await page.waitForTimeout(900)
  664 |   const settledMetrics = await getScrollMetrics(historyPane)
  665 |   expect(settledMetrics.distanceToBottom).toBeGreaterThan(600)
  666 | })
  667 | 
```