import { CheckCircle, XCircle } from 'lucide-react'
import type { RequestLog } from '@/lib/types'

interface Props {
  logs: RequestLog[]
  loading: boolean
}

export default function RequestLogs({ logs, loading }: Props) {
  if (loading) {
    return <div className="h-64 animate-pulse rounded-xl bg-gray-200 dark:bg-gray-700" />
  }

  return (
    <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
      <h3 className="mb-4 text-sm font-semibold text-gray-700 dark:text-gray-300">
        请求日志（最近 100 条）
      </h3>
      <div className="overflow-x-auto">
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-gray-200 dark:border-gray-700">
              <th className="pb-2 font-medium text-gray-500">时间</th>
              <th className="pb-2 font-medium text-gray-500">模型</th>
              <th className="pb-2 font-medium text-gray-500">Provider</th>
              <th className="pb-2 font-medium text-gray-500">TTFT</th>
              <th className="pb-2 font-medium text-gray-500">Tokens</th>
              <th className="pb-2 font-medium text-gray-500">状态</th>
            </tr>
          </thead>
          <tbody>
            {logs.slice(0, 20).map((log, i) => (
              <tr key={i} className="border-b border-gray-100 dark:border-gray-800">
                <td className="py-2 text-gray-500">
                  {new Date(log.timestamp).toLocaleTimeString('zh-CN')}
                </td>
                <td className="py-2 font-mono text-xs">{log.model}</td>
                <td className="py-2">{log.provider}</td>
                <td className="py-2">{log.ttft_ms}ms</td>
                <td className="py-2 text-gray-500">
                  {log.input_tokens}→{log.output_tokens}
                </td>
                <td className="py-2">
                  {log.error ? (
                    <span className="flex items-center gap-1 text-red-500">
                      <XCircle className="h-4 w-4" />
                      <span className="text-xs">{log.error}</span>
                    </span>
                  ) : (
                    <CheckCircle className="h-4 w-4 text-green-500" />
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}
