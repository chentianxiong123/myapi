import { useEffect, useState } from 'react'
import {
  Save, Shuffle, Clock, ShieldAlert, Timer, ListOrdered, Zap, RotateCcw,
  Activity, ChevronDown, Settings2,
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

// 实时路由决策（开箱即用：看它怎么自动路由）
interface RoutingDecision {
  time: string
  model: string
  channel: string
  reason: string
  ttft_ms: number
  status: 'ok' | 'error'
}

const MOCK_STATS: CircuitStats[] = [
  { channel: 'SenseNova · 带key-ipv4', state: 'closed', consecutiveFailures: 0, consecutiveSuccesses: 12, totalRequests: 812, failedRequests: 7 },
  { channel: 'SenseNova · 带key-ipv6', state: 'open', consecutiveFailures: 6, consecutiveSuccesses: 0, totalRequests: 198, failedRequests: 60 },
  { channel: 'tavily · 无key-ipv6', state: 'half_open', consecutiveFailures: 0, consecutiveSuccesses: 1, totalRequests: 410, failedRequests: 33 },
]

const MOCK_DECISIONS: RoutingDecision[] = [
  { time: '10:23:41', model: 'deepseek-v4-flash', channel: 'SenseNova · 带key-ipv4', reason: '黏性命中（会话 abc123）+ TTFT 最低', ttft_ms: 412, status: 'ok' },
  { time: '10:23:12', model: 'deepseek-v4-flash', channel: 'SenseNova · 带key-ipv4', reason: '延迟感知（窗口内 380ms 最低）', ttft_ms: 387, status: 'ok' },
  { time: '10:22:58', model: 'web-search', channel: 'tavily · 无key-ipv6', reason: 'capability 匹配（搜索）', ttft_ms: 1240, status: 'ok' },
  { time: '10:22:30', model: 'deepseek-v4-flash', channel: 'pie-xian · 带key', reason: '熔断跳过 SenseNova-ipv6 → 切下一个', ttft_ms: 521, status: 'ok' },
  { time: '10:22:01', model: 'deepseek-v4-flash', channel: 'SenseNova · 带key-ipv6', reason: '请求失败（连续 6 次）→ 熔断', ttft_ms: 8021, status: 'error' },
]

export default function Routing() {
  const [config, setConfig] = useState<RoutingConfig>(DEFAULT_ROUTING_CONFIG)
  const [stats, setStats] = useState<CircuitStats[]>([])
  const [decisions, setDecisions] = useState<RoutingDecision[]>([])
  const [busy, setBusy] = useState(false)
  const [loaded, setLoaded] = useState(false)
  const [showAdvanced, setShowAdvanced] = useState(false)

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
      setDecisions(MOCK_DECISIONS)
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
      {/* 头部 + Auto 开关 */}
      <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div>
          <h2 className="text-lg font-semibold">路由策略</h2>
          <p className="text-sm text-gray-500">
            配完通道就自动路由（延迟感知 + 黏性省 token + 三态熔断），开箱即用
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <label className="flex items-center gap-2 rounded-lg border border-violet-300 bg-violet-50 px-3 py-1.5 text-sm dark:border-violet-700 dark:bg-violet-900/20">
            <input
              type="checkbox" checked={config.auto_mode.enabled}
              onChange={e => updateNested('auto_mode', 'enabled', e.target.checked)}
              className="h-4 w-4"
            />
            <Zap className="h-4 w-4 text-violet-500" />Auto 自动路由
          </label>
          <button onClick={reset} className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">
            <RotateCcw className="h-4 w-4" />恢复默认
          </button>
          <button onClick={save} disabled={busy} className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700 disabled:opacity-50">
            <Save className="h-4 w-4" />保存
          </button>
        </div>
      </div>

      {/* ① 实时路由决策流（观测台主视图） */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <Activity className="h-4 w-4 text-green-500" />路由决策流（实时）
        </h3>
        <div className="overflow-x-auto">
          <table className="w-full min-w-[620px] text-left text-sm">
            <thead>
              <tr className="border-b border-gray-200 dark:border-gray-700">
                <th className="pb-2 font-medium text-gray-500">时间</th>
                <th className="pb-2 font-medium text-gray-500">模型</th>
                <th className="pb-2 font-medium text-gray-500">命中通道</th>
                <th className="pb-2 font-medium text-gray-500">选择原因</th>
                <th className="pb-2 font-medium text-gray-500">首字</th>
                <th className="pb-2 font-medium text-gray-500">状态</th>
              </tr>
            </thead>
            <tbody>
              {decisions.map((d, i) => (
                <tr key={i} className="border-b border-gray-100 dark:border-gray-800">
                  <td className="py-2 font-mono text-xs text-gray-500">{d.time}</td>
                  <td className="py-2 font-mono text-xs">{d.model}</td>
                  <td className="py-2 font-mono text-xs">{d.channel}</td>
                  <td className="py-2 text-xs text-gray-600 dark:text-gray-300">{d.reason}</td>
                  <td className="py-2 font-mono text-xs">{d.ttft_ms}ms</td>
                  <td className="py-2">
                    <span className={`rounded px-1.5 py-0.5 text-[10px] ${d.status === 'ok' ? 'bg-green-100 text-green-700 dark:bg-green-900/30' : 'bg-red-100 text-red-700 dark:bg-red-900/30'}`}>
                      {d.status === 'ok' ? '成功' : '失败'}
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p className="mt-2 text-[11px] text-gray-400">每次请求命中哪条通道 + 为什么选它（黏性/延迟/熔断跳过/capability），全透明</p>
      </div>

      {/* ② 通道熔断状态（实时） */}
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
        <p className="mt-2 text-[11px] text-gray-400">closed 正常放行 · open 熔断跳过 · half_open 试探恢复（学 cc-switch 三态机）</p>
      </div>

      {/* ③ 高级设置（折叠，开箱即用默认收起） */}
      <div className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <button
          onClick={() => setShowAdvanced(!showAdvanced)}
          className="flex w-full items-center justify-between p-5 text-left"
        >
          <span className="flex items-center gap-2 text-sm font-semibold">
            <Settings2 className="h-4 w-4" />高级设置（默认已开箱即用，一般无需调整）
          </span>
          <ChevronDown className={`h-4 w-4 transition-transform ${showAdvanced ? 'rotate-180' : ''}`} />
        </button>
        {showAdvanced && (
          <div className="space-y-4 border-t border-gray-200 p-5 dark:border-gray-700">
            <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
              {/* 路由策略 */}
              <div>
                <h4 className="mb-2 flex items-center gap-1.5 text-sm font-medium"><Shuffle className="h-3.5 w-3.5" />路由策略</h4>
                <div className="space-y-2">
                  <select
                    value={config.strategy}
                    onChange={e => update('strategy', e.target.value as any)}
                    className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-700"
                  >
                    {ROUTING_STRATEGIES.map(s => <option key={s.id} value={s.id}>{s.label}</option>)}
                  </select>
                  <div>
                    <label className="mb-1 flex items-center gap-1 text-xs text-gray-500"><Clock className="h-3 w-3" />延迟窗口（秒）</label>
                    <input
                      type="number" value={config.latency_window_seconds}
                      onChange={e => update('latency_window_seconds', parseInt(e.target.value) || 60)}
                      className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700"
                    />
                  </div>
                </div>
              </div>

              {/* 黏性路由 */}
              <div>
                <h4 className="mb-2 flex items-center gap-1.5 text-sm font-medium"><Zap className="h-3.5 w-3.5 text-amber-500" />黏性路由（省 token）</h4>
                <div className="space-y-2">
                  <label className="flex items-center gap-2 text-sm">
                    <input
                      type="checkbox" checked={config.sticky.enabled}
                      onChange={e => updateNested('sticky', 'enabled', e.target.checked)}
                      className="h-4 w-4"
                    />
                    启用（同会话命中上游 prompt cache）
                  </label>
                  <select
                    value={config.sticky.granularity}
                    onChange={e => updateNested('sticky', 'granularity', e.target.value)}
                    disabled={!config.sticky.enabled}
                    className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm disabled:opacity-50 dark:border-gray-600 dark:bg-gray-700"
                  >
                    {STICKY_GRANULARITIES.map(g => <option key={g.id} value={g.id}>{g.label}</option>)}
                  </select>
                  <div>
                    <label className="mb-1 block text-xs text-gray-500">TTL（秒）</label>
                    <input
                      type="number" value={config.sticky.ttl_seconds}
                      onChange={e => updateNested('sticky', 'ttl_seconds', parseInt(e.target.value) || 300)}
                      disabled={!config.sticky.enabled}
                      className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm disabled:opacity-50 dark:border-gray-600 dark:bg-gray-700"
                    />
                  </div>
                </div>
              </div>

              {/* 熔断器 */}
              <div>
                <h4 className="mb-2 flex items-center gap-1.5 text-sm font-medium"><ShieldAlert className="h-3.5 w-3.5 text-red-500" />熔断器（三态）</h4>
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <label className="mb-1 block text-xs text-gray-500">连续失败（次）</label>
                    <input
                      type="number" value={config.circuit_breaker.failure_threshold}
                      onChange={e => updateNested('circuit_breaker', 'failure_threshold', parseInt(e.target.value) || 5)}
                      className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700"
                    />
                  </div>
                  <div>
                    <label className="mb-1 block text-xs text-gray-500">错误率（%）</label>
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
                  <div className="col-span-2">
                    <label className="mb-1 block text-xs text-gray-500">恢复成功阈值（次）</label>
                    <input
                      type="number" value={config.circuit_breaker.recovery_success_threshold}
                      onChange={e => updateNested('circuit_breaker', 'recovery_success_threshold', parseInt(e.target.value) || 3)}
                      className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700"
                    />
                  </div>
                </div>
              </div>

              {/* 超时 + 兜底 */}
              <div>
                <h4 className="mb-2 flex items-center gap-1.5 text-sm font-medium"><Timer className="h-3.5 w-3.5" />超时与兜底</h4>
                <div className="space-y-2">
                  <div className="grid grid-cols-3 gap-2">
                    <div>
                      <label className="mb-1 block text-xs text-gray-500">流式首字</label>
                      <input
                        type="number" value={config.timeout.streaming_first_byte}
                        onChange={e => updateNested('timeout', 'streaming_first_byte', parseInt(e.target.value) || 30)}
                        className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                      />
                    </div>
                    <div>
                      <label className="mb-1 block text-xs text-gray-500">流式空闲</label>
                      <input
                        type="number" value={config.timeout.streaming_idle}
                        onChange={e => updateNested('timeout', 'streaming_idle', parseInt(e.target.value) || 60)}
                        className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                      />
                    </div>
                    <div>
                      <label className="mb-1 block text-xs text-gray-500">非流式</label>
                      <input
                        type="number" value={config.timeout.non_streaming}
                        onChange={e => updateNested('timeout', 'non_streaming', parseInt(e.target.value) || 120)}
                        className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                      />
                    </div>
                  </div>
                  <div>
                    <label className="mb-1 block text-xs text-gray-500">全挂兜底</label>
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

            {/* Auto capability 匹配 */}
            <div className="border-t border-gray-100 pt-3 dark:border-gray-800">
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox" checked={config.auto_mode.capability_match}
                  onChange={e => updateNested('auto_mode', 'capability_match', e.target.checked)}
                  className="h-4 w-4"
                />
                Auto capability 匹配（vision/pdf 硬能力必须满足，搜索软能力可降级）
              </label>
            </div>
          </div>
        )}
      </div>

      <StatusBar busy={busy} onClear={() => {}} />
    </div>
  )
}
