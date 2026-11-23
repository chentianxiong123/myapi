import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen } from '@testing-library/react'
import Models from '@/pages/Models'

describe('Models（模型级路由）', () => {
  beforeEach(() => {
    vi.restoreAllMocks()
  })

  it('渲染模型映射标题', async () => {
    // mock fetch（后端未启动，回退 mock）
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('backend down')))
    render(<Models />)
    expect(await screen.findByText(/模型映射/)).toBeInTheDocument()
  })

  it('展示模型级路由信息（key/代理/独立额度）', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('backend down')))
    render(<Models />)
    // 等 mock 数据加载（tavily/search + text-embedding-3-small）
    const tavily = await screen.findAllByText('tavily/search', {}, { timeout: 3000 })
    expect(tavily.length).toBeGreaterThan(0)
    expect(screen.getAllByText('text-embedding-3-small').length).toBeGreaterThan(0)
  })

  it('展示 route 的 带key/无key 徽标', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('backend down')))
    render(<Models />)
    await screen.findAllByText('tavily/search', {}, { timeout: 3000 })
    expect(screen.getAllByText(/带key/).length).toBeGreaterThan(0)
    expect(screen.getAllByText(/无key/).length).toBeGreaterThan(0)
  })
})
