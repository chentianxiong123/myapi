import { describe, it, expect } from 'vitest'
import { render, screen } from '@testing-library/react'
import StatsHero from '@/components/StatsHero'
import type { StatsSummary } from '@/lib/types'

const stats: StatsSummary = {
  total_requests: 1234,
  success_count: 1204,
  success_rate: 97.6,
  error_count: 29,
  error_rate: 2.4,
  total_input_tokens: 355000,
  total_output_tokens: 101000,
  total_cache_creation_tokens: 123000,
  total_cache_read_tokens: 890000,
  real_total_tokens: 456000,
  cache_hit_rate: 39.4,
  avg_ttft_ms: 245,
  avg_latency_ms: 1800,
  avg_speed_tps: 45,
  active_providers: 3,
  total_providers: 3,
}

describe('StatsHero（统计概览）', () => {
  it('渲染统计卡（总 Tokens 替代总成本）', () => {
    render(<StatsHero stats={stats} loading={false} />)
    expect(screen.getByText('请求数')).toBeInTheDocument()
    expect(screen.getByText('总 Tokens')).toBeInTheDocument()
    expect(screen.getByText(/成功/)).toBeInTheDocument()
    expect(screen.getByText(/240\.0%/)).toBeInTheDocument()
  })

  it('不渲染成本（cost 已彻底删除）', () => {
    render(<StatsHero stats={stats} loading={false} />)
    expect(screen.queryByText(/成本|费用|cost/i)).not.toBeInTheDocument()
  })

  it('loading 时显示骨架', () => {
    render(<StatsHero stats={null as any} loading={true} />)
    expect(document.querySelector('.animate-pulse')).toBeInTheDocument()
  })
})
