import { Activity, Zap, AlertTriangle, Server, Layers, Database, ArrowDownToLine, ArrowUpFromLine } from 'lucide-react'
import type { StatsSummary } from '@/lib/types'

interface Props {
  stats: StatsSummary | null
  loading: boolean
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return n.toLocaleString()
}

function formatCost(n: number): string {
  if (n >= 1) return `$${n.toFixed(2)}`
  if (n >= 0.01) return `$${n.toFixed(4)}`
  return `$${n.toFixed(6)}`
}

function formatPercent(n: number): string {
  return `${(n * 100).toFixed(1)}%`
}

export default function StatsHero({ stats, loading }: Props) {
  if (loading || !stats) {
    return (
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4 lg:grid-cols-8">
        {[...Array(8)].map((_, i) => (
          <div key={i} className="h-24 animate-pulse rounded-xl bg-gray-200 dark:bg-gray-700" />
        ))}
      </div>
    )
  }

  const cards = [
    {
      label: '请求数',
      value: stats.total_requests.toLocaleString(),
      sub: `成功 ${formatPercent(stats.success_rate)}`,
      icon: Activity,
      color: 'text-blue-600 bg-blue-50 dark:bg-blue-900/20',
    },
    {
      label: '总 Tokens',
      sub: '含缓存',
      icon: Layers,
      color: 'text-violet-600 bg-violet-50 dark:bg-violet-900/20',
    },
    {
      label: '输入 Tokens',
      value: formatTokens(stats.total_input_tokens),
      sub: `新鲜 ${formatTokens(stats.total_input_tokens - stats.total_cache_read_tokens)}`,
      icon: ArrowDownToLine,
      color: 'text-cyan-600 bg-cyan-50 dark:bg-cyan-900/20',
    },
    {
      label: '输出 Tokens',
      value: formatTokens(stats.total_output_tokens),
      sub: `${stats.avg_speed_tps.toFixed(0)} tok/s`,
      icon: ArrowUpFromLine,
      color: 'text-green-600 bg-green-50 dark:bg-green-900/20',
    },
    {
      label: '缓存读取',
      value: formatTokens(stats.total_cache_read_tokens),
      sub: `命中率 ${formatPercent(stats.cache_hit_rate)}`,
      icon: Database,
      color: 'text-purple-600 bg-purple-50 dark:bg-purple-900/20',
    },
    {
      label: '缓存写入',
      value: formatTokens(stats.total_cache_creation_tokens),
      sub: '首次',
      icon: Database,
      color: 'text-indigo-600 bg-indigo-50 dark:bg-indigo-900/20',
    },
    {
      label: '平均首字',
      value: `${stats.avg_ttft_ms.toFixed(0)}ms`,
      sub: `${stats.avg_speed_tps.toFixed(0)} tok/s`,
      icon: Zap,
      color: 'text-orange-600 bg-orange-50 dark:bg-orange-900/20',
    },
    {
      label: '错误率',
      value: formatPercent(stats.error_rate),
      sub: `${stats.error_count} 次`,
      icon: AlertTriangle,
      color: 'text-red-600 bg-red-50 dark:bg-red-900/20',
    },
  ]

  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-4 lg:grid-cols-8">
      {cards.map(({ label, value, sub, icon: Icon, color }) => (
        <div
          key={label}
          className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800"
        >
          <div className="flex items-center justify-between">
            <span className="text-xs text-gray-500 dark:text-gray-400">{label}</span>
            <div className={`rounded-lg p-1.5 ${color}`}>
              <Icon className="h-4 w-4" />
            </div>
          </div>
          <div className="mt-1.5 text-lg font-bold">{value}</div>
          <div className="text-xs text-gray-400">{sub}</div>
        </div>
      ))}
    </div>
  )
}
