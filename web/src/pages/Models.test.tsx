import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen } from '@testing-library/react'
import Models from '@/pages/Models'

describe('Models（模型健康度台，只读）', () => {
  beforeEach(() => {
    vi.restoreAllMocks()
  })

  it('渲染健康度标题', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('backend down')))
    render(<Models />)
    expect(await screen.findByText(/模型健康度/)).toBeInTheDocument()
  })

  it('展示模型 alias（通道来源）', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('backend down')))
    render(<Models />)
    const tavily = await screen.findAllByText('tavily/search', {}, { timeout: 3000 })
    expect(tavily.length).toBeGreaterThan(0)
    expect(screen.getAllByText('text-embedding-3-small').length).toBeGreaterThan(0)
  })

  it('展示被动健康指标（成功率/首字/请求数）', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('backend down')))
    render(<Models />)
    await screen.findAllByText('tavily/search', {}, { timeout: 3000 })
    // 聚合指标标签
    expect(screen.getAllByText(/成功率/).length).toBeGreaterThan(0)
    expect(screen.getAllByText(/首字/).length).toBeGreaterThan(0)
    expect(screen.getAllByText(/请求数/).length).toBeGreaterThan(0)
  })

  it('展示路由分流（权重命中比 %）', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('backend down')))
    render(<Models />)
    await screen.findAllByText('tavily/search', {}, { timeout: 3000 })
    // deepseek 8:2 → 80%/20%；tavily 6:4 → 60%/40%
    expect(screen.getAllByText('80%').length).toBeGreaterThan(0)
    expect(screen.getAllByText('60%').length).toBeGreaterThan(0)
  })

  it('展示熔断状态（熔断中徽标 + 原因）', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('backend down')))
    render(<Models />)
    await screen.findAllByText('tavily/search', {}, { timeout: 3000 })
    // deepseek 通道② 和 agnes 通道② 在 mock 里熔断中
    expect(screen.getAllByText(/熔断中/).length).toBeGreaterThan(0)
  })

  it('无编辑按钮（模型页纯只读）', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('backend down')))
    render(<Models />)
    await screen.findAllByText('tavily/search', {}, { timeout: 3000 })
    // 不应有 添加/保存/编辑 按钮（配置全在供应商页）
    expect(screen.queryByRole('button', { name: /添加模型/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /^保存$/ })).not.toBeInTheDocument()
  })
})
