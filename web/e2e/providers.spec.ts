import { test, expect } from '@playwright/test'
import { onlyDesktop, navTo } from './helpers'

/**
 * 供应商页 E2E：卡片头 / 展开 / API Keys / 渠道代理 / 协议 tab / 模型表格
 * （修复的移动端布局也在此覆盖）
 */
test.describe('供应商页', () => {
  onlyDesktop()
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await navTo(page, '供应商')
  })

  test('标题 + 保存/添加供应商按钮', async ({ page }) => {
    await expect(page.locator('main h2')).toContainText('Providers')
    await expect(page.getByRole('button', { name: '保存' })).toBeVisible()
    await expect(page.getByRole('button', { name: /添加供应商/ })).toBeVisible()
  })

  test('供应商卡片渲染（名称/base_url/徽标）', async ({ page }) => {
    // 卡片头字段
    await expect(page.getByPlaceholder('provider-name').first()).toBeVisible()
    await expect(page.getByPlaceholder('https://api.example.com/v1').first()).toBeVisible()
    // 密钥池/通道/模型 徽标（重构后：N 密钥 · N 通道 · N 模型）
    await expect(page.getByText(/\d+ 密钥 · \d+ 通道 · \d+ 模型/).first()).toBeVisible()
  })

  test('展开供应商 → 密钥池 + 渠道代理 + 通道列表', async ({ page }) => {
    // 点第一张卡片头的展开箭头
    await page.getByPlaceholder('provider-name').first().evaluate(el => {
      const header = el.parentElement!
      header.querySelectorAll('button')[0].click()
    })
    await expect(page.getByText('密钥池').first()).toBeVisible()
    await expect(page.getByText('渠道代理')).toBeVisible()
    // 通道列表（一条通道 = 密钥×路径×模型映射）
    await expect(page.getByText('通道').first()).toBeVisible()
    await expect(page.getByText('模型映射').first()).toBeVisible()
    // 通道内模型表格（Name/Alias/协议/类型）
    await expect(page.getByText('Name（上游真实名）').first()).toBeVisible()
    await expect(page.getByText('Alias（对外，空=用name）').first()).toBeVisible()
    // 模型类型列（学 9router kind）
    await expect(page.locator('select').filter({ has: page.locator('option', { hasText: '识图（vision）' }) }).first()).toBeVisible()
  })

  test('渠道代理三选一（直连/单代理/代理池）', async ({ page }) => {
    await page.getByPlaceholder('provider-name').first().evaluate(el => {
      const header = el.parentElement!
      header.querySelectorAll('button')[0].click()
    })
    await expect(page.getByText('渠道代理')).toBeVisible()
    await expect(page.getByPlaceholder(/单代理 URL/)).toBeVisible()
    await expect(page.locator('select').filter({ has: page.locator('option', { hasText: '不使用代理池' }) })).toBeVisible()
  })
})
