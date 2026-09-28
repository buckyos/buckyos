export default async ({ page }) => {
  await page.goto('http://127.0.0.1:4179/?scenario=normal')
  await page.getByRole('button', {name: 'AI 画布', exact: true}).waitFor({timeout:90000})
}
