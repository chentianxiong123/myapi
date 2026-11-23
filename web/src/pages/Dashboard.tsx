import { useEffect, useState } from 'react'
import { api } from '@/lib/api'
import type { StatsSummary, ProviderStat, RequestLog, TrendPoint } from '@/lib/types'
import StatsHero from '@/components/StatsHero'
import TrendChart from '@/components/TrendChart'
import ProviderStats from '@/components/ProviderStats'
import RequestLogs from '@/components/RequestLogs'
import DateRangePicker, { type RangeSelection, getRangeDates } from '@/components/DateRangePicker'

type Tab = 'logs' | 'providers' | 'models'

export default function Dashboard() {
  const [stats, setStats] = useState<StatsSummary | null>(null)
  const [providers, setProviders] = useState<ProviderStat[]>([])
  const [logs, setLogs] = useState<RequestLog[]>([])
  const [trend, setTrend] = useState<TrendPoint[]>([])
  const [loading, setLoading] = useState(true)
  const [range, setRange] = useState<RangeSelection>({ preset: '7d' })
  const [tab, setTab] = useState<Tab>('logs')

  useEffect(() => {
    load()
  }, [range])

  async function load() {
    setLoading(true)
    try {
      const [s, p, l, t] = await Promise.all([
        api.getStats(),
        api.getProviderStats(),
        api.getRequestLogs(100),
        api.getTrend(24),
      ])
      setStats(s); setProviders(p); setLogs(l); setTrend(t)
    } catch {
      // 假数据
      const ti = 1245000, to = 456000, tcr = 890000, tcc = 123000, tr = 1234, sc = 1205
      setStats({
        total_requests: tr, success_count: sc, success_rate: sc / tr,
        error_count: tr - sc, error_rate: (tr - sc) / tr,
        total_input_tokens: ti, total_output_tokens: to,
        total_cache_creation_tokens: tcc, total_cache_read_tokens: tcr,
        real_total_tokens: ti + to + tcc + tcr,
        cache_hit_rate: tcr / (ti + tcc + tcr),
        total_cost: 12.345678, avg_ttft_ms: 245, avg_latency_ms: 890,
        avg_speed_tps: 45, active_providers: 2, total_providers: 3,
      })
      setProviders([
        { name: 'SenseNova', base_url: '', status: 'healthy', total_requests: 800, avg_ttft_ms: 180, error_rate: 0.01, weight: 8, total_input_tokens: 800000, total_output_tokens: 300000, total_cache_read_tokens: 600000, total_cache_creation_tokens: 80000, cache_hit_rate: 0.72, total_cost: 8.23 },
        { name: 'pie-xian', base_url: '', status: 'healthy', total_requests: 400, avg_ttft_ms: 320, error_rate: 0.03, weight: 2, total_input_tokens: 400000, total_output_tokens: 150000, total_cache_read_tokens: 250000, total_cache_creation_tokens: 40000, cache_hit_rate: 0.65, total_cost: 3.89 },
        { name: 'iamhc', base_url: '', status: 'cooldown', total_requests: 34, avg_ttft_ms: 0, error_rate: 1.0, weight: 0, total_input_tokens: 45000, total_output_tokens: 6000, total_cache_read_tokens: 40000, total_cache_creation_tokens: 3000, cache_hit_rate: 0.89, total_cost: 0.22 },
      ])
      setTrend(Array.from({ length: 24 }, (_, i) => ({
        time: `${String(i).padStart(2, '0')}:00`,
        requests: Math.floor(Math.random() * 50) + 10,
        avg_ttft_ms: Math.floor(Math.random() * 200) + 150,
        errors: Math.floor(Math.random() * 3),
        input_tokens: Math.floor(Math.random() * 50000) + 10000,
        output_tokens: Math.floor(Math.random() * 20000) + 5000,
        cache_read_tokens: Math.floor(Math.random() * 40000) + 8000,
        cost: Math.random() * 0.5 + 0.1,
      })))
      setLogs(Array.from({ length: 20 }, (_, i) => {
        const input = Math.floor(Math.random() * 3000) + 100
        const output = Math.floor(Math.random() * 800) + 50
        const cache_read = Math.floor(Math.random() * 2000)
        const cache_creation = Math.floor(Math.random() * 500)
        const latency = Math.floor(Math.random() * 2000) + 500
        const duration = latency / 1000
        const error = i % 7 === 0 ? 'timeout' : null
        return {
          id: `req-${i}`, timestamp: new Date(Date.now() - i * 45000).toISOString(),
          model: ['deepseek-v4-flash', 'agnes-3.0-flash', 'glm-5.3', 'kimi-k3.1-pro'][i % 4],
          provider: ['SenseNova', 'pie-xian', 'iamhc', 'agnes'][i % 4],
          app_type: ['pi', 'opencode', 'claude', 'codex'][i % 4],
          session_id: `session-${Math.floor(i / 3)}`,
          input_tokens: input, fresh_input_tokens: input - cache_read, output_tokens: output,
          cache_read_tokens: cache_read, cache_creation_tokens: cache_creation,
          cache_hit_rate: (cache_read / (input + cache_creation)) || 0,
          real_total_tokens: input + output + cache_creation + cache_read,
          cost: Math.random() * 0.05 + 0.001,
          ttft_ms: error ? 0 : Math.floor(Math.random() * 300) + 100,
          latency_ms: latency, duration_s: duration,
          output_speed_tps: output / duration,
          status_code: error ? 502 : 200, error, is_stream: true,
        }
      }))
    } finally {
      setLoading(false)
    }
  }

  const tabs: Array<{ id: Tab; label: string }> = [
    { id: 'logs', label: '请求日志' },
    { id: 'providers', label: 'Provider 统计' },
    { id: 'models', label: '模型统计' },
  ]

  return (
    <div className="space-y-4">
      {/* 顶部：日期选择器 */}
      <div className="flex items-center justify-between">
        <DateRangePicker selection={range} onChange={setRange} />
        <span className="text-xs text-gray-400">
          {getRangeDates(range).start.toLocaleDateString('zh-CN')} ~ {getRangeDates(range).end.toLocaleDateString('zh-CN')}
        </span>
      </div>

      {/* 统计卡片 */}
      <StatsHero stats={stats} loading={loading} />

      {/* 趋势图 */}
      <TrendChart data={trend} loading={loading} />

      {/* Tab 切换（学cc-switch PageTabs） */}
      <div className="flex gap-1 rounded-lg bg-gray-100 p-1 dark:bg-gray-700">
        {tabs.map(t => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={`rounded-md px-4 py-1.5 text-sm ${
              tab === t.id
                ? 'bg-white font-semibold text-blue-600 shadow dark:bg-gray-800'
                : 'text-gray-500 hover:text-gray-900 dark:hover:text-gray-100'
            }`}
          >{t.label}</button>
        ))}
      </div>

      {/* 内容区（按tab 切换） */}
      {tab === 'logs' && <RequestLogs logs={logs} loading={loading} />}
      {tab === 'providers' && <ProviderStats providers={providers} loading={loading} />}
      {tab === 'models' && (
        <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
          <h3 className="mb-4 text-sm font-semibold">模型统计</h3>
          <table className="w-full text-left text-sm">
            <thead>
              <tr className="border-b border-gray-200 dark:border-gray-700">
                <th className="pb-2 font-medium text-gray-500">模型</th>
                <th className="pb-2 font-medium text-gray-500">请求数</th>
                <th className="pb-2 font-medium text-gray-500">输入</th>
                <th className="pb-2 font-medium text-gray-500">输出</th>
                <th className="pb-2 font-medium text-gray-500">缓存命中率</th>
                <th className="pb-2 font-medium text-gray-500">成本</th>
              </tr>
            </thead>
            <tbody>
              {Object.entries(
                logs.reduce<Record<string, { count: number; input: number; output: number; cache_hit: number[]; cost: number }>>((acc, log) => {
                  if (!acc[log.model]) acc[log.model] = { count: 0, input: 0, output: 0, cache_hit: [], cost: 0 }
                  acc[log.model].count++
                  acc[log.model].input += log.input_tokens
                  acc[log.model].output += log.output_tokens
                  acc[log.model].cache_hit.push(log.cache_hit_rate)
                  acc[log.model].cost += log.cost
                  return acc
                }, {})
              ).map(([model, s]) => (
                <tr key={model} className="border-b border-gray-100 dark:border-gray-800">
                  <td className="py-2 font-mono text-xs">{model}</td>
                  <td className="py-2">{s.count}</td>
                  <td className="py-2 font-mono text-xs">{s.input.toLocaleString()}</td>
                  <td className="py-2 font-mono text-xs">{s.output.toLocaleString()}</td>
                  <td className="py-2 font-mono text-xs">
                    {(s.cache_hit.reduce((a, b) => a + b, 0) / s.cache_hit.length * 100).toFixed(1)}%
                  </td>
                  <td className="py-2 font-mono text-xs">${s.cost.toFixed(4)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
