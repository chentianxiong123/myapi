import { test, expect } from '@playwright/test'
import { onlyDesktop, navTo } from './helpers'

/**
 * 模型映射页：模型级路由（route）+ 带key/无key + 代理 + 独立额度
 */
test.describe('模型映射页', () => {
  onlyDesktop()
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await navTo(page, '模型')
  })

  test('健康度标题（只读）', async ({ page }) => {
    await expect(page.getByText(/模型健康度/)).toBeVisible()
    // 模型页纯只读：无添加/保存按钮（配置全在供应商页）
    await expect(page.getByRole('button', { name: /添加模型/ })).toHaveCount(0)
    await expect(page.getByRole('button', { name: /^保存$/ })).toHaveCount(0)
  })

  test('模型通道来源 + 被动健康（成功率/首字/命中比）', async ({ page }) => {
    // 等 mock 数据加载
    await expect(page.getByText('tavily/search').first()).toBeVisible()
    await expect(page.getByText('text-embedding-3-small').first()).toBeVisible()
    // 被动健康指标标签（学 LiteLLM cooldown：非主动探测）
    await expect(page.getByText('成功率').first()).toBeVisible()
    await expect(page.getByText('首字').first()).toBeVisible()
    await expect(page.getByText('请求数').first()).toBeVisible()
  })

  test('模型类型标签（chat/search/embedding）', async ({ page }) => {
    // 类型标签展示（中文：webSearch→搜索，embedding→嵌入）
    await expect(page.getByText('搜索').first()).toBeVisible()
    await expect(page.getByText('嵌入').first()).toBeVisible()
  })
})

/**
 * MCP 接口页：搜索/识图作为 MCP 工具
 */
test.describe('MCP 接口页', () => {
  onlyDesktop()
  test('工具映射 + MCP Server 配置', async ({ page }) => {
    await page.goto('/')
    await navTo(page, 'MCP 接口')
    // 限定 main 内，避免与侧边栏导航按钮（同名）歧义
    await expect(page.locator('main').getByText('MCP 接口')).toBeVisible()
    await expect(page.locator('main').getByText('工具映射（模型 → MCP 工具）')).toBeVisible()
    await expect(page.locator('main').getByText('MCP Server（agent 的 mcpServers 配置）')).toBeVisible()
    // search / vision 工具
    await expect(page.locator('main').getByText('搜索（webSearch）')).toBeVisible()
    await expect(page.locator('main').getByText('识图（vision）')).toBeVisible()
  })
})
