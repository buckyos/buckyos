// Runs in a background tab of the Agent's browser (browser.background_run): the tab is at the page already;
// this hands back what the page's own script drew. Nothing is looked for in here: what the page must have is
// extract.ts's to say, so that a page that changed fails there, by name.
export default async ({ page }) => {
  await page.waitForLoadState('networkidle')
  return { url: page.url(), html: await page.content() }
}
