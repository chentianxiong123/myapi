import { CheckCircle, XCircle, Layers, Database, Zap, Activity } from 'lucide-react'
import type { RequestLog } from '@/lib/types'

interface Props {
  logs: RequestLog[]
  loading: boolean
  /** 简版（Dashboard tab 用）：只显示核心列 */
  compact?: boolean
  /** 点击行（打开详情） */
  onRowClick?: (log: RequestLog) => void
}

// 学 new-api getColorByElapsedTime：耗时颜色分级（首字/耗时共用）
function elapsedColor(ms: number): string {
  if (ms <= 0) return 'text-gray-400'
  if (ms < 1000) return 'text-green-600'
  if (ms < 3000) return 'text-yellow-700'
  if (ms < 5000) return 'text-yellow-600'
  if (ms < 10000) return 'text-orange-500'
  return 'text-red-600'
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return n.toString()
}

// 学 new-api timestamp2string：YYYY-MM-DD HH:mm:ss
function formatTime(ts: string): string {
  const d = new Date(ts)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`
}

/** new-api 风格请求日志表格（统一 Dashboard tab + Logs 页） */
export default function RequestLogTable({ logs, loading, compact = false, onRowClick }: Props) {
  return (
    <div className="overflow-x-auto rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
      <table className="w-full text-left text-sm">
        <thead>
          <tr className="border-b border-gray-200 dark:border-gray-700">
            <th className="whitespace-nowrap px-3 py-3 font-medium text-gray-500 dark:text-gray-400">时间</th>
            {!compact && <th className="whitespace-nowrap px-3 py-3 font-medium text-gray-500 dark:text-gray-400">渠道</th>}
            {!compact && <th className="whitespace-nowrap px-3 py-3 font-medium text-gray-500 dark:text-gray-400">用户</th>}
            <th className="whitespace-nowrap px-3 py-3 font-medium text-gray-500 dark:text-gray-400">模型</th>
            <th className="whitespace-nowrap px-3 py-3 font-medium text-gray-500 dark:text-gray-400">流</th>
            <th className="whitespace-nowrap px-3 py-3 font-medium text-gray-500 dark:text-gray-400">Token</th>
            {!compact && <th className="whitespace-nowrap px-3 py-3 font-medium text-gray-500 dark:text-gray-400">费用</th>}
            <th className="whitespace-nowrap px-3 py-3 font-medium text-gray-500 dark:text-gray-400">耗时</th>
            <th className="whitespace-nowrap px-3 py-3 font-medium text-gray-500 dark:text-gray-400">详情</th>
          </tr>
        </thead>
        <tbody>
          {loading ? (
            <tr><td colSpan={compact ? 6 : 9} className="px-4 py-8 text-center text-gray-400">加载中…</td></tr>
          ) : logs.length === 0 ? (
            <tr><td colSpan={compact ? 6 : 9} className="px-4 py-8 text-center text-gray-400">没有日志</td></tr>
          ) : logs.map((log, i) => (
            <tr
              key={i}
              onClick={() => onRowClick?.(log)}
              className={`border-b border-gray-100 hover:bg-gray-50 dark:border-gray-800 dark:hover:bg-gray-700/50 ${onRowClick ? 'cursor-pointer' : ''}`}
            >
              {/* 时间 + 类型标签 */}
              <td className="whitespace-nowrap px-3 py-2">
                <div className="font-mono text-xs text-gray-500">{formatTime(log.timestamp)}</div>
                <div className="mt-0.5">
                  {log.error ? (
                    <span className="rounded bg-red-100 px-1 text-[9px] text-red-600 dark:bg-red-900/30">错误</span>
                  ) : (
                    <span className="rounded bg-blue-100 px-1 text-[9px] text-blue-600 dark:bg-blue-900/30">消耗</span>
                  )}
                </div>
              </td>

              {/* 渠道 */}
              {!compact && (
                <td className="whitespace-nowrap px-3 py-2">
                  <div className="font-mono text-xs text-purple-600">#{log.status_code > 0 ? '8' : '-'}</div>
                  <div className="mt-0.5 text-xs text-gray-400">{log.provider}</div>
                </td>
              )}

              {/* 用户 */}
              {!compact && (
                <td className="whitespace-nowrap px-3 py-2">
                  <div className="flex items-center gap-1.5">
                    <span className="flex h-4 w-4 items-center justify-center rounded-full bg-red-100 text-[9px] font-bold text-red-600 dark:bg-red-900/30">
                      {log.app_type.slice(0, 1).toUpperCase()}
                    </span>
                    <span className="text-xs">{log.app_type}</span>
                  </div>
                </td>
              )}

              {/* 模型（图标 + 名字） */}
              <td className="whitespace-nowrap px-3 py-2">
                <div className="flex items-center gap-1.5">
                  <Layers className="h-3.5 w-3.5 text-blue-500" />
                  <span className="font-mono text-xs">{log.model}</span>
                </div>
                <div className="mt-0.5 text-[10px] text-gray-400">会话 {log.session_id.slice(0, 10)}</div>
              </td>

              {/* 流 + 速度 */}
              <td className="whitespace-nowrap px-3 py-2">
                {log.is_stream ? (
                  <div className="flex items-center gap-1">
                    <Zap className="h-3 w-3 text-blue-500" />
                    <span className="rounded bg-blue-100 px-1 text-[9px] text-blue-600 dark:bg-blue-900/30">流</span>
                  </div>
                ) : (
                  <span className="rounded bg-gray-100 px-1 text-[9px] text-gray-500 dark:bg-gray-700">—</span>
                )}
                <div className="mt-0.5 font-mono text-[10px] text-blue-500">{log.output_speed_tps.toFixed(0)} t/s</div>
              </td>

              {/* Token（输入/输出 + 缓存） */}
              <td className="whitespace-nowrap px-3 py-2">
                <div className="font-mono text-xs">
                  {formatTokens(log.input_tokens)} / {formatTokens(log.output_tokens)}
                </div>
                <div className="mt-0.5 flex items-center gap-1 font-mono text-[10px] text-gray-400">
                  <Database className="h-2.5 w-2.5" />
                  缓存: {formatTokens(log.cache_read_tokens)}
                </div>
              </td>

              {/* 费用（用户已要求删成本，显示 —） */}
              {!compact && (
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs text-gray-400">—</td>
              )}

              {/* 耗时：首字 / 总时长 两行，带颜色分级（new-api 核心） */}
              <td className="whitespace-nowrap px-3 py-2 font-mono text-xs">
                <div className={elapsedColor(log.ttft_ms)}>
                  <span className="text-gray-400">首字</span> {log.ttft_ms > 0 ? `${(log.ttft_ms / 1000).toFixed(1)}s` : '不适用'}
                </div>
                <div className={elapsedColor(log.latency_ms)}>
                  <span className="text-gray-400">耗时</span> {log.duration_s.toFixed(1)}s
                </div>
              </td>

              {/* 详情 */}
              <td className="whitespace-nowrap px-3 py-2">
                {log.error ? (
                  <span className="flex items-center gap-1 text-red-500">
                    <XCircle className="h-3.5 w-3.5" />
                    <span className="text-xs">{log.error}</span>
                  </span>
                ) : (
                  <span className="flex items-center gap-1 text-blue-500">
                    <CheckCircle className="h-3.5 w-3.5" />
                    <span className="text-xs">标准</span>
                  </span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
