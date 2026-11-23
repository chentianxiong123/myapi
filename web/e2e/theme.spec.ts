import { test, expect } from '@playwright/test'
import { onlyDesktop } from './helpers'

/**
 * 主题 + 语言切换（Sidebar 底部工具）
 */
test.describe('主题与语言', () => {
  onlyDesktop()
  test('深色模式切换', async ({ page }) => {
    await page.goto('/')
    const html = page.locator('html')
    const before = await html.evaluate(el => el.classList.contains('dark'))
    // 点主题切换按钮（侧边栏底部）
    await page.getByRole('button', { name: /深色|浅色/ }).click()
    const after = await html.evaluate(el => el.classList.contains('dark'))
    expect(after).not.toBe(before)
  })

  test('主题持久化（刷新保持）', async ({ page }) => {
    await page.goto('/')
    await page.getByRole('button', { name: /深色|浅色/ }).click()
    await page.reload()
    const afterReload = await page.locator('html').evaluate(el => el.classList.contains('dark'))
    expect(afterReload).toBe(true)
  })

  test('语言切换（中/英）', async ({ page }) => {
    await page.goto('/')
    // 中文 → English
    await page.getByRole('button', { name: '中文' }).click()
    // 导航项变英文
    await expect(page.getByRole('button', { name: 'Dashboard' })).toBeVisible()
  })

  test('语言持久化', async ({ page }) => {
    await page.goto('/')
    await page.getByRole('button', { name: '中文' }).click()
    await page.reload()
    await expect(page.getByRole('button', { name: 'Dashboard' })).toBeVisible()
  })
})
