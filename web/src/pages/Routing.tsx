import { useEffect, useState } from 'react'
import {
  Save, Shuffle, Clock, ShieldAlert, Timer, ListOrdered, Zap, RotateCcw,
} from 'lucide-react'
import { toast } from '@/components/Toast'
import StatusBar from '@/components/StatusBar'
import { postJSON } from '@/lib/api'
import type { RoutingConfig, CircuitState } from '@/lib/types'
import {
  ROUTING_STRATEGIES, STICKY_GRANULARITIES, WHEN_ALL_UNHEALTHY_OPTIONS, DEFAULT_ROUTING_CONFIG,
} from '@/lib/types'

// 运行态熔断器统计（学 cc-switch CircuitBreakerStats）
interface CircuitStats {
  channel: string
  state: CircuitState
  consecutiveFailures: number
  consecutiveSuccesses: number
  totalRequests: number
  failedRequests: number
}

const MOCK_STATS: CircuitStats[] = [
  { channel: 'SenseNova · 带key-ipv4', state: 'closed', consecutiveFailures: 0, consecutiveSuccesses: 12, totalRequests: 812, failedRequests: 7 },
  { channel: 'SenseNova · 带key-ipv6', state: 'open', consecutiveFailures: 6, consecutiveSuccesses: 0, totalRequests: 198, failedRequests: 60 },
  { channel: 'tavily · 无key-ipv6', state: 'half_open', consecutiveFailures: 0, consecutiveSuccesses: 1, totalRequests: 410, failedRequests: 33 },
]

export default function Routing() {
  const [config, setConfig] = useState<RoutingConfig>(DEFAULT_ROUTING_CONFIG)
  const [stats, setStats] = useState<CircuitStats[]>([])
  const [busy, setBusy] = useState(false)
  const [loaded, setLoaded] = useState(false)

  useEffect(() => { load() }, [])

  async function load() {
    try {
      const res = await fetch('/api/routing')
      const data = await res.json()
      setConfig(data.routing || DEFAULT_ROUTING_CONFIG)
    } catch {
      setConfig(DEFAULT_ROUTING_CONFIG)
    } finally {
      setStats(MOCK_STATS)
      setLoaded(true)
    }
  }

  function update<K extends keyof RoutingConfig>(field: K, value: RoutingConfig[K]) {
    setConfig({ ...config, [field]: value })
  }
  function updateNested<K extends keyof RoutingConfig>(field: K, key: string, value: any) {
    setConfig({ ...config, [field]: { ...(config[field] as any), [key]: value } })
  }

  async function save() {
    setBusy(true)
    try {
      await postJSON('/api/routing', { routing: config })
      toast('路由策略已保存')
    } catch {
      toast('后端未实现，仅本地保存')
    } finally { setBusy(false) }
  }

  function reset() {
    if (!confirm('恢复默认路由策略？')) return
    setConfig(DEFAULT_ROUTING_CONFIG)
    toast('已恢复默认')
  }

  const STATE_STYLE: Record<CircuitState, { label: string; cls: string }> = {
    closed: { label: '正常', cls: 'bg-green-100 text-green-700 dark:bg-green-900/30' },
    open: { label: '熔断中', cls: 'bg-red-100 text-red-700 dark:bg-red-900/30' },
    half_open: { label: '半开试探', cls: 'bg-amber-100 text-amber-700 dark:bg-amber-900/30' },
  }

  if (!loaded) return <div className="animate-pulse text-gray-500">加载中…</div>

  return (
    <div className="space-y-4">
      {/* 头部 */}
      <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div>
          <h2 className="text-lg font-semibold">路由策略</h2>
          <p className="text-sm text-gray-500">
            同模型多通道怎么选 · 黏性省 token · 三态熔断（学 LiteLLM 策略 + cc-switch 熔断器）
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button onClick={reset} className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">
            <RotateCcw className="h-4 w-4" />恢复默认
          </button>
          <button onClick={save} disabled={busy} className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700 disabled:opacity-50">
            <Save className="h-4 w-4" />保存
          </button>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        {/* ① 路由策略 */}
        <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
          <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><Shuffle className="h-4 w-4" />路由策略</h3>
          <div className="space-y-3">
            <div>
              <label className="mb-1 block text-xs text-gray-500">策略（同模型多通道怎么选）</label>
              <select
                value={config.strategy}
                onChange={e => update('strategy', e.target.value as any)}
                className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-700"
              >
                {ROUTING_STRATEGIES.map(s => <option key={s.id} value={s.id}>{s.label}</option>)}
              </select>
            </div>
            <div>
              <label className="mb-1 flex items-center gap-1 text-xs text-gray-500"><Clock className="h-3 w-3" />延迟窗口（秒）</label>
              <input
                type="number" value={config.latency_window_seconds}
                onChange={e => update('latency_window_seconds', parseInt(e.target.value) || 60)}
                className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700"
              />
              <p className="mt-0.5 text-[11px] text-gray-400">TTFT 滑动窗口，窗口内统计平均首字</p>
            </div>
          </div>
        </div>

        {/* ② 黏性路由（省 token） */}
        <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
          <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><Zap className="h-4 w-4 text-amber-500" />黏性路由（省 token）</h3>
          <div className="space-y-3">
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox" checked={config.sticky.enabled}
                onChange={e => updateNested('sticky', 'enabled', e.target.checked)}
                className="h-4 w-4"
              />
              启用黏性（同会话路由到同一通道，命中上游 prompt cache）
            </label>
            <div>
              <label className="mb-1 block text-xs text-gray-500">粒度</label>
              <select
                value={config.sticky.granularity}
                onChange={e => updateNested('sticky', 'granularity', e.target.value)}
                disabled={!config.sticky.enabled}
                className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm disabled:opacity-50 dark:border-gray-600 dark:bg-gray-700"
              >
                {STICKY_GRANULARITIES.map(g => <option key={g.id} value={g.id}>{g.label}</option>)}
              </select>
            </div>
            <div>
              <label className="mb-1 block text-xs text-gray-500">TTL（秒）</label>
              <input
                type="number" value={config.sticky.ttl_seconds}
                onChange={e => updateNested('sticky', 'ttl_seconds', parseInt(e.target.value) || 300)}
                disabled={!config.sticky.enabled}
                className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm disabled:opacity-50 dark:border-gray-600 dark:bg-gray-700"
              />
              <p className="mt-0.5 text-[11px] text-gray-400">亲和保持时长，过期后重新按策略选通道</p>
            </div>
          </div>
        </div>

        {/* ③ 熔断器（三态，学 cc-switch） */}
        <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
          <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><ShieldAlert className="h-4 w-4 text-red-500" />熔断器（三态）</h3>
          <div className="space-y-3">
            <div className="grid grid-cols-2 gap-2">
              <div>
                <label className="mb-1 block text-xs text-gray-500">连续失败阈值（次）</label>
                <input
                  type="number" value={config.circuit_breaker.failure_threshold}
                  onChange={e => updateNested('circuit_breaker', 'failure_threshold', parseInt(e.target.value) || 5)}
                  className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700"
                />
              </div>
              <div>
                <label className="mb-1 block text-xs text-gray-500">错误率阈值（%）</label>
                <input
                  type="number" value={config.circuit_breaker.error_rate_threshold}
                  onChange={e => updateNested('circuit_breaker', 'error_rate_threshold', parseInt(e.target.value) || 50)}
                  className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700"
                />
              </div>
              <div>
                <label className="mb-1 block text-xs text-gray-500">最小请求数</label>
                <input
                  type="number" value={config.circuit_breaker.min_requests}
                  onChange={e => updateNested('circuit_breaker', 'min_requests', parseInt(e.target.value) || 5)}
                  className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700"
                />
              </div>
              <div>
                <label className="mb-1 block text-xs text-gray-500">恢复等待（秒）</label>
                <input
                  type="number" value={config.circuit_breaker.recovery_timeout_seconds}
                  onChange={e => updateNested('circuit_breaker', 'recovery_timeout_seconds', parseInt(e.target.value) || 30)}
                  className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700"
                />
              </div>
            </div>
            <div>
              <label className="mb-1 block text-xs text-gray-500">恢复成功阈值（次）</label>
              <input
                type="number" value={config.circuit_breaker.recovery_success_threshold}
                onChange={e => updateNested('circuit_breaker', 'recovery_success_threshold', parseInt(e.target.value) || 3)}
                className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700"
              />
              <p className="mt-0.5 text-[11px] text-gray-400">
                closed →（连续失败/错误率超）→ open →（等恢复等待）→ half_open →（连续成功达阈值）→ closed
              </p>
            </div>
          </div>
        </div>

        {/* ④ 超时 + 全挂兜底 */}
        <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
          <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><Timer className="h-4 w-4" />超时与兜底</h3>
          <div className="space-y-3">
            <div className="grid grid-cols-3 gap-2">
              <div>
                <label className="mb-1 block text-xs text-gray-500">流式首字（秒）</label>
                <input
                  type="number" value={config.timeout.streaming_first_byte}
                  onChange={e => updateNested('timeout', 'streaming_first_byte', parseInt(e.target.value) || 30)}
                  className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                />
              </div>
              <div>
                <label className="mb-1 block text-xs text-gray-500">流式空闲（秒）</label>
                <input
                  type="number" value={config.timeout.streaming_idle}
                  onChange={e => updateNested('timeout', 'streaming_idle', parseInt(e.target.value) || 60)}
                  className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                />
              </div>
              <div>
                <label className="mb-1 block text-xs text-gray-500">非流式（秒）</label>
                <input
                  type="number" value={config.timeout.non_streaming}
                  onChange={e => updateNested('timeout', 'non_streaming', parseInt(e.target.value) || 120)}
                  className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                />
              </div>
            </div>
            <div>
              <label className="mb-1 block text-xs text-gray-500">全挂兜底（所有通道都不健康时）</label>
              <select
                value={config.when_all_unhealthy}
                onChange={e => update('when_all_unhealthy', e.target.value as any)}
                className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-700"
              >
                {WHEN_ALL_UNHEALTHY_OPTIONS.map(o => <option key={o.id} value={o.id}>{o.label}</option>)}
              </select>
            </div>
          </div>
        </div>
      </div>

      {/* ⑤ 运行态熔断器统计（只读） */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><ListOrdered className="h-4 w-4" />通道熔断状态（实时）</h3>
        <div className="overflow-x-auto">
          <table className="w-full min-w-[560px] text-left text-sm">
            <thead>
              <tr className="border-b border-gray-200 dark:border-gray-700">
                <th className="pb-2 font-medium text-gray-500">通道</th>
                <th className="pb-2 font-medium text-gray-500">状态</th>
                <th className="pb-2 font-medium text-gray-500">连续失败</th>
                <th className="pb-2 font-medium text-gray-500">请求数</th>
                <th className="pb-2 font-medium text-gray-500">错误数</th>
              </tr>
            </thead>
            <tbody>
              {stats.map(s => (
                <tr key={s.channel} className="border-b border-gray-100 dark:border-gray-800">
                  <td className="py-2 font-mono text-xs">{s.channel}</td>
                  <td className="py-2">
                    <span className={`rounded px-1.5 py-0.5 text-[10px] ${STATE_STYLE[s.state].cls}`}>{STATE_STYLE[s.state].label}</span>
                  </td>
                  <td className="py-2 font-mono text-xs">{s.consecutiveFailures}</td>
                  <td className="py-2 font-mono text-xs">{s.totalRequests}</td>
                  <td className="py-2 font-mono text-xs">{s.failedRequests}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      {/* ⑥ Auto 模式（未来） */}
      <div className="rounded-xl border border-dashed border-gray-300 bg-white p-5 opacity-80 dark:border-gray-600 dark:bg-gray-800">
        <h3 className="mb-2 flex items-center gap-2 text-sm font-semibold"><Zap className="h-4 w-4 text-violet-500" />Auto 模式（即将上线）</h3>
        <p className="mb-3 text-xs text-gray-400">
          调集一切可用模型自动路由（学 9router combo：capability 匹配 + 动态排序 + 链式 fallback）
        </p>
        <div className="space-y-2">
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox" checked={config.auto_mode.enabled}
              onChange={e => updateNested('auto_mode', 'enabled', e.target.checked)}
              className="h-4 w-4"
            />
            启用 Auto（一个 auto 别名背后是所有健康通道的链式池）
          </label>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox" checked={config.auto_mode.capability_match}
              onChange={e => updateNested('auto_mode', 'capability_match', e.target.checked)}
              className="h-4 w-4"
            />
            capability 匹配（vision/pdf 硬能力必须满足，搜索软能力可降级）
          </label>
        </div>
      </div>

      <StatusBar busy={busy} onClear={() => {}} />
    </div>
  )
}
