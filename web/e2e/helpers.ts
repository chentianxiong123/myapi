import { test, type TestInfo } from '@playwright/test'

/**
 * 按 Playwright project 条件过滤（桌面/移动端）
 * 用法：在 describe 块内第一行调用
 *   test.describe('xxx', () => {
 *     onlyDesktop()
 *     test('...', async ({ page }) => { ... })
 *   })
 */
/** 点击侧边栏导航（限定 aside，名称精确匹配，避免与页面内同名词按钮歧义） */
export async function navTo(page: import('@playwright/test').Page, label: string) {
  await page.locator('aside').getByRole('button', { name: label, exact: true }).click()
}

const gate = (name: string) => () => {
  test.beforeEach(async ({}, testInfo: TestInfo) => {
    if (testInfo.project.name !== name) testInfo.skip(true)
  })
}

/** 仅桌面视口（桌面侧边栏在窄屏 hidden） */
export const onlyDesktop = gate('chromium-desktop')
/** 仅移动端视口（桌面视口无 hamburger 顶栏） */
export const onlyMobile = gate('mobile')
