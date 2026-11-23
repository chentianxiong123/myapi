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
      } catch (e) {
        console.error('加载失败:', e)
        // 后端还没写，用假数据先看UI
        setStats({
          total_requests: 1234,
          success_count: 1205,
          error_count: 29,
          error_rate: 0.023,
          avg_ttft_ms: 245,
          avg_latency_ms: 890,
          active_providers: 2,
          total_providers: 3,
        })
        setProviders([
          { name: 'SenseNova', base_url: 'https://token.sensenova.cn', status: 'healthy', total_requests: 800, avg_ttft_ms: 180, error_rate: 0.01, weight: 8 },
          { name: 'pie-xian', base_url: 'https://api.pie-xian.com', status: 'healthy', total_requests: 400, avg_ttft_ms: 320, error_rate: 0.03, weight: 2 },
          { name: 'iamhc', base_url: 'https://api.hcnsec.cn', status: 'cooldown', total_requests: 34, avg_ttft_ms: 0, error_rate: 1.0, weight: 0 },
        ])
        setTrend(
          Array.from({ length: 24 }, (_, i) => ({
            time: `${String(i).padStart(2, '0')}:00`,
            requests: Math.floor(Math.random() * 50) + 10,
            avg_ttft_ms: Math.floor(Math.random() * 200) + 150,
            errors: Math.floor(Math.random() * 3),
          }))
        )
        setLogs(
          Array.from({ length: 20 }, (_, i) => ({
            timestamp: new Date(Date.now() - i * 60000).toISOString(),
            model: ['deepseek-v4-flash', 'agnes-3.0-flash', 'glm-5.3'][i % 3],
            provider: ['SenseNova', 'pie-xian', 'iamhc'][i % 3],
            ttft_ms: Math.floor(Math.random() * 300) + 100,
            latency_ms: Math.floor(Math.random() * 1000) + 500,
            tokens_in: Math.floor(Math.random() * 2000) + 100,
            tokens_out: Math.floor(Math.random() * 500) + 50,
            error: i % 7 === 0 ? 'timeout' : null,
            session_id: `session-${i}`,
          }))
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
