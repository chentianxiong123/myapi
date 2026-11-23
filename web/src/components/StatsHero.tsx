import { Activity, Zap, AlertTriangle, Server } from 'lucide-react'
import type { StatsSummary } from '@/lib/types'

interface Props {
  stats: StatsSummary | null
  loading: boolean
}

export default function StatsHero({ stats, loading }: Props) {
  if (loading || !stats) {
    return (
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {[...Array(4)].map((_, i) => (
          <div key={i} className="h-24 animate-pulse rounded-xl bg-gray-200 dark:bg-gray-700" />
        ))}
      </div>
    )
  }

  const cards = [
    {
      label: '总请求数',
      value: stats.total_requests.toLocaleString(),
      icon: Activity,
      color: 'text-blue-600 bg-blue-50 dark:bg-blue-900/20',
    },
    {
      label: '平均 TTFT',
      value: `${stats.avg_ttft_ms.toFixed(0)}ms`,
      icon: Zap,
      color: 'text-green-600 bg-green-50 dark:bg-green-900/20',
    },
    {
      label: '错误率',
      value: `${(stats.error_rate * 100).toFixed(1)}%`,
      icon: AlertTriangle,
      color: 'text-red-600 bg-red-50 dark:bg-red-900/20',
    },
    {
      label: '活跃 Provider',
      value: `${stats.active_providers}/${stats.total_providers}`,
      icon: Server,
      color: 'text-purple-600 bg-purple-50 dark:bg-purple-900/20',
    },
  ]

  return (
    <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
      {cards.map(({ label, value, icon: Icon, color }) => (
        <div
          key={label}
          className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800"
        >
          <div className="flex items-center justify-between">
            <span className="text-sm text-gray-500 dark:text-gray-400">{label}</span>
            <div className={`rounded-lg p-2 ${color}`}>
              <Icon className="h-5 w-5" />
            </div>
          </div>
          <div className="mt-2 text-2xl font-bold">{value}</div>
        </div>
      ))}
    </div>
  )
}
