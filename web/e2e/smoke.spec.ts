import { test, expect } from '@playwright/test'
import { onlyDesktop } from './helpers'

/**
 * 冒烟测试：应用可加载 + 侧边栏 logo + 仪表盘统计
 * 后端未启动（8317），前端走 mock 兜底——E2E 验证 UI 流程，不依赖后端
 *
 * 注：Dashboard 无 h2 标题（直接渲染 StatsHero）；
 *     Logo 在顶栏（移动端）+ 侧边栏（桌面）各一份，须限定可见容器
 */
test.describe('冒烟测试', () => {
  onlyDesktop()

  test('首页加载 + 标题', async ({ page }) => {
    await page.goto('/')
    await expect(page).toHaveTitle(/myapi/)
    // 仪表盘无 h2，直接渲染统计卡
    await expect(page.getByText('请求数').first()).toBeVisible()
  })

  test('侧边栏 Logo（MY + myapi）', async ({ page }) => {
    await page.goto('/')
    // 限定 aside（桌面可见侧边栏），避免取到隐藏的移动端顶栏
    const aside = page.locator('aside')
    await expect(aside).toBeVisible()
    await expect(aside.getByText('MY', { exact: true })).toBeVisible()
    await expect(aside.getByText('myapi')).toBeVisible()
  })

  test('统计概览渲染（总 Tokens 替代总成本）', async ({ page }) => {
    await page.goto('/')
    await expect(page.getByText('总 Tokens').first()).toBeVisible()
    // cost 已彻底删除
    await expect(page.getByText(/总成本|费用/)).toHaveCount(0)
  })
})
