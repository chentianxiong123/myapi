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

  test('标题 + 添加模型按钮', async ({ page }) => {
    await expect(page.getByText(/模型映射/)).toBeVisible()
    await expect(page.getByRole('button', { name: /添加模型/ })).toBeVisible()
  })

  test('模型级路由展示（route + 带key/无key）', async ({ page }) => {
    // 等 mock 数据加载
    await expect(page.getByText('tavily/search').first()).toBeVisible()
    await expect(page.getByText('text-embedding-3-small').first()).toBeVisible()
    // 带key / 无key 徽标（用户刁钻场景：无 key 请求）
    await expect(page.getByText('带key').first()).toBeVisible()
    await expect(page.getByText('无key').first()).toBeVisible()
  })

  test('模型类型筛选（全部/chat/search/embedding）', async ({ page }) => {
    // 类型标签展示（webSearch 青色徽标）
    await expect(page.getByText('webSearch').first()).toBeVisible()
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
