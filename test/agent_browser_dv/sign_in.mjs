// A background-run script that plays the owner signing in at the test site (site.mjs), for runs through
// browser_stub.mjs. With a real Agent the owner signs in on the screen; no script of the Agent fills a password.
//   node browser_stub.mjs run sign_in.mjs --arg url=http://172.17.0.1:18741/login --arg username=dvuser --arg password=<password>
export default async ({ page, args }) => {
  await page.goto(args.url)
  if (new URL(page.url()).pathname !== '/login') return { signed_in: true, url: page.url() }
  await page.fill('#username', args.username)
  await page.fill('#password', args.password)
  await Promise.all([page.waitForURL((url) => url.pathname !== '/login'), page.click('button[type=submit]')])
  return { signed_in: true, url: page.url() }
}
