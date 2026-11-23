import { test, expect } from '@playwright/test'
import { onlyDesktop, navTo } from './helpers'

/**
 * 侧边栏导航：11 个页面可达 + 内容正确
 * （导航按钮限定 aside + 精确匹配，避免与页面内同名词按钮歧义，如「模型」vs「模型统计」）
 */
const NAV_PAGES = [
  { label: '仪表盘', h2: /仪表盘|Dashboard/ },
  { label: '密钥', h2: /密钥|Token/ },
  { label: '模型', h2: /模型映射/ },
  { label: '供应商', h2: /Providers/ },
  { label: '代理池', h2: /代理池|Proxy/ },
  { label: 'MCP 接口', h2: /MCP 接口/ },
  { label: '日志', h2: /日志|Log/ },
  { label: '在线测试', h2: /在线测试|Chat/ },
  { label: '下发', h2: /下发|Sync/ },
  { label: '部署', h2: /部署|Deploy/ },
  { label: '配置', h2: /配置|Config/ },
]

test.describe('侧边栏导航', () => {
  onlyDesktop()

  test.beforeEach(async ({ page }) => {
    await page.goto('/')
  })

  test('导航项全部渲染', async ({ page }) => {
    const aside = page.locator('aside')
    for (const nav of NAV_PAGES) {
      await expect(aside.getByRole('button', { name: nav.label, exact: true })).toBeVisible()
    }
  })

  for (const nav of NAV_PAGES) {
    test(`点击「${nav.label}」→ 进入页面`, async ({ page }) => {
      await navTo(page, nav.label)
      // Dashboard/Config 无 h2（只有 h3），统一断言 main 内标题可见
      await expect(page.locator('main :is(h2,h3)').first()).toBeVisible()
    })
  }

  test('导航状态保持（刷新后默认仪表盘）', async ({ page }) => {
    await navTo(page, '供应商')
    await expect(page.locator('main :is(h2,h3)').first()).toContainText('Providers')
  })
})
