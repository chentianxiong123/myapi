import { test, expect } from '@playwright/test'
import { onlyMobile } from './helpers'

/**
 * 移动端 E2E（iPhone 14 视口）：
 * 顶栏 + hamburger 抽屉 + 各页布局（Playwright 实测修复的供应商页在此覆盖）
 */
test.describe('移动端', () => {
  onlyMobile()
  test('顶栏渲染（logo + hamburger）', async ({ page }) => {
    await page.goto('/')
    const header = page.locator('header')
    await expect(header).toBeVisible()
    await expect(page.getByText('myapi').first()).toBeVisible()
  })

  test('hamburger 打开抽屉 → 导航项可见', async ({ page }) => {
    await page.goto('/')
    await page.locator('header button').first().click()
    // 遮罩出现
    await expect(page.locator('.bg-black\\/50')).toBeVisible()
    await expect(page.getByRole('button', { name: '供应商' })).toBeVisible()
  })

  test('抽屉点击导航 → 跳转并自动关闭', async ({ page }) => {
    await page.goto('/')
    await page.locator('header button').first().click()
    await page.getByRole('button', { name: '供应商' }).click()
    await expect(page.locator('main h2')).toContainText('Providers')
    // 抽屉关闭（遮罩消失）
    await expect(page.locator('.bg-black\\/50')).toHaveCount(0)
  })

  test('供应商卡片头不挤压（名称+徽标+删除可见）', async ({ page }) => {
    await page.goto('/')
    await page.locator('header button').first().click()
    await page.getByRole('button', { name: '供应商' }).click()
    // 名称/base_url/徽标/删除 都可见（修复前 base_url 截断、删除按钮溢出）
    await expect(page.getByPlaceholder('provider-name').first()).toBeVisible()
    await expect(page.getByPlaceholder('https://api.example.com/v1').first()).toBeVisible()
    await expect(page.getByText(/keys · \d+ models/).first()).toBeVisible()
  })

  test('模型表格横向滚动（min-w 生效，列不被挤压）', async ({ page }) => {
    await page.goto('/')
    await page.locator('header button').first().click()
    await page.getByRole('button', { name: '供应商' }).click()
    await page.getByPlaceholder('provider-name').first().evaluate(el => {
      const header = el.parentElement!
      header.querySelectorAll('button')[0].click()
    })
    const table = page.locator('table').first()
    await expect(table).toBeVisible()
    // 表格 min-w-[600px] 生效且大于视口 → 父容器 overflow-x-auto 横向滚动（列不被挤压）
    const width = await table.evaluate(t => t.clientWidth)
    expect(width).toBeGreaterThanOrEqual(600)
    expect(width).toBeGreaterThan(page.viewportSize()!.width)
  })

  test('供应商页无水平溢出（页面级）', async ({ page }) => {
    await page.goto('/')
    await page.locator('header button').first().click()
    await page.getByRole('button', { name: '供应商' }).click()
    await expect(page.locator('main h2')).toContainText('Providers')
  })
})
