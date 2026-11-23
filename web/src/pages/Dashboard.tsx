import { useEffect, useState } from 'react'
import { api } from '@/lib/api'
import type { StatsSummary, ProviderStat, RequestLog, TrendPoint } from '@/lib/types'
import StatsHero from '@/components/StatsHero'
import TrendChart from '@/components/TrendChart'
import ProviderStats from '@/components/ProviderStats'
import RequestLogs from '@/components/RequestLogs'

export default function Dashboard() {
  const [stats, setStats] = useState<StatsSummary | null>(null)
  const [providers, setProviders] = useState<ProviderStat[]>([])
  const [logs, setLogs] = useState<RequestLog[]>([])
  const [trend, setTrend] = useState<TrendPoint[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    async function load() {
      try {
        const [s, p, l, t] = await Promise.all([
          api.getStats(),
          api.getProviderStats(),
          api.getRequestLogs(100),
          api.getTrend(24),
        ])
        setStats(s)
        setProviders(p)
        setLogs(l)
        setTrend(t)
      } catch {
        // 后端还没写，用假数据（学 cc-switch 的完整字段）
        const total_input = 1245000
        const total_output = 456000
        const total_cache_read = 890000
        const total_cache_creation = 123000
        const total_requests = 1234
        const success_count = 1205
        const total_cost = 12.345678

        setStats({
          total_requests,
          success_count,
          success_rate: success_count / total_requests,
          error_count: total_requests - success_count,
          error_rate: (total_requests - success_count) / total_requests,
          total_input_tokens: total_input,
          total_output_tokens: total_output,
          total_cache_creation_tokens: total_cache_creation,
          total_cache_read_tokens: total_cache_read,
          real_total_tokens: total_input + total_output + total_cache_creation + total_cache_read,
          cache_hit_rate: total_cache_read / (total_input + total_cache_creation + total_cache_read),
          total_cost,
          avg_ttft_ms: 245,
          avg_latency_ms: 890,
          avg_speed_tps: 45,
          active_providers: 2,
          total_providers: 3,
        })

        setProviders([
          {
            name: 'SenseNova', base_url: 'https://token.sensenova.cn', status: 'healthy',
            total_requests: 800, avg_ttft_ms: 180, error_rate: 0.01, weight: 8,
            total_input_tokens: 800000, total_output_tokens: 300000,
            total_cache_read_tokens: 600000, total_cache_creation_tokens: 80000,
            cache_hit_rate: 0.72, total_cost: 8.23,
          },
          {
            name: 'pie-xian', base_url: 'https://api.pie-xian.com', status: 'healthy',
            total_requests: 400, avg_ttft_ms: 320, error_rate: 0.03, weight: 2,
            total_input_tokens: 400000, total_output_tokens: 150000,
            total_cache_read_tokens: 250000, total_cache_creation_tokens: 40000,
            cache_hit_rate: 0.65, total_cost: 3.89,
          },
          {
            name: 'iamhc', base_url: 'https://api.hcnsec.cn', status: 'cooldown',
            total_requests: 34, avg_ttft_ms: 0, error_rate: 1.0, weight: 0,
            total_input_tokens: 45000, total_output_tokens: 6000,
            total_cache_read_tokens: 40000, total_cache_creation_tokens: 3000,
            cache_hit_rate: 0.89, total_cost: 0.22,
          },
        ])

        setTrend(
          Array.from({ length: 24 }, (_, i) => ({
            time: `${String(i).padStart(2, '0')}:00`,
            requests: Math.floor(Math.random() * 50) + 10,
            avg_ttft_ms: Math.floor(Math.random() * 200) + 150,
            errors: Math.floor(Math.random() * 3),
            input_tokens: Math.floor(Math.random() * 50000) + 10000,
            output_tokens: Math.floor(Math.random() * 20000) + 5000,
            cache_read_tokens: Math.floor(Math.random() * 40000) + 8000,
            cost: Math.random() * 0.5 + 0.1,
          }))
        )

        setLogs(
          Array.from({ length: 20 }, (_, i) => {
            const input_tokens = Math.floor(Math.random() * 3000) + 100
            const output_tokens = Math.floor(Math.random() * 800) + 50
            const cache_read = Math.floor(Math.random() * 2000)
            const cache_creation = Math.floor(Math.random() * 500)
            const ttft = Math.floor(Math.random() * 300) + 100
            const latency = Math.floor(Math.random() * 2000) + 500
            const duration = latency / 1000
            const error = i % 7 === 0 ? 'timeout' : null
            return {
              id: `req-${i}`,
              timestamp: new Date(Date.now() - i * 45000).toISOString(),
              model: ['deepseek-v4-flash', 'agnes-3.0-flash', 'glm-5.3', 'kimi-k3.1-pro'][i % 4],
              provider: ['SenseNova', 'pie-xian', 'iamhc', 'agnes'][i % 4],
              app_type: ['pi', 'opencode', 'claude', 'codex'][i % 4],
              session_id: `session-${Math.floor(i / 3)}`,
              input_tokens,
              fresh_input_tokens: input_tokens - cache_read,
              output_tokens,
              cache_read_tokens: cache_read,
              cache_creation_tokens: cache_creation,
              cache_hit_rate: (cache_read / (input_tokens + cache_creation)) || 0,
              real_total_tokens: input_tokens + output_tokens + cache_creation + cache_read,
              cost: Math.random() * 0.05 + 0.001,
              ttft_ms: ttft,
              latency_ms: latency,
              duration_s: duration,
              output_speed_tps: output_tokens / duration,
              status_code: error ? 502 : 200,
              error,
              is_stream: true,
            }
          })
        )
      } finally {
        setLoading(false)
      }
    }
    load()
  }, [])

  return (
    <div className="space-y-6">
      <StatsHero stats={stats} loading={loading} />
      <TrendChart data={trend} loading={loading} />
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <ProviderStats providers={providers} loading={loading} />
        <RequestLogs logs={logs} loading={loading} />
      </div>
    </div>
  )
}
