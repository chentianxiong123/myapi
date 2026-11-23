import { Server, CheckCircle, AlertCircle, Clock } from 'lucide-react'
import type { ProviderStat } from '@/lib/types'

interface Props {
  providers: ProviderStat[]
  loading: boolean
}

export default function ProviderStats({ providers, loading }: Props) {
  if (loading) {
    return <div className="h-40 animate-pulse rounded-xl bg-gray-200 dark:bg-gray-700" />
  }

  const statusIcon = (status: string) => {
    switch (status) {
      case 'healthy':
        return <CheckCircle className="h-4 w-4 text-green-500" />
      case 'cooldown':
        return <Clock className="h-4 w-4 text-yellow-500" />
      case 'error':
        return <AlertCircle className="h-4 w-4 text-red-500" />
      default:
        return <Server className="h-4 w-4 text-gray-400" />
    }
  }

  return (
    <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
      <h3 className="mb-4 text-sm font-semibold text-gray-700 dark:text-gray-300">
        Provider 状态
      </h3>
      <div className="overflow-x-auto">
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-gray-200 dark:border-gray-700">
              <th className="pb-2 font-medium text-gray-500">Provider</th>
              <th className="pb-2 font-medium text-gray-500">请求数</th>
              <th className="pb-2 font-medium text-gray-500">平均 TTFT</th>
              <th className="pb-2 font-medium text-gray-500">错误率</th>
              <th className="pb-2 font-medium text-gray-500">权重</th>
              <th className="pb-2 font-medium text-gray-500">状态</th>
            </tr>
          </thead>
          <tbody>
            {providers.map((p) => (
              <tr key={p.name} className="border-b border-gray-100 dark:border-gray-800">
                <td className="py-2 font-medium">{p.name}</td>
                <td className="py-2">{p.total_requests.toLocaleString()}</td>
                <td className="py-2">{p.avg_ttft_ms.toFixed(0)}ms</td>
                <td className="py-2">{(p.error_rate * 100).toFixed(1)}%</td>
                <td className="py-2">{p.weight}</td>
                <td className="py-2">
                  <span className="flex items-center gap-1">
                    {statusIcon(p.status)}
                    <span className="text-xs capitalize">{p.status}</span>
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}
