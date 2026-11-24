import { useEffect, useState } from 'react'
import { Activity, AlertTriangle, Clock, Gauge, Layers, PauseCircle, ShieldCheck } from 'lucide-react'
import type { ExposedModel, ModelRoute, ModelKind } from '@/lib/types'

// 模型健康度台（只读，被动统计真实请求，学 LiteLLM cooldown：非主动探测）
// 一个模型（alias）由多条通道供（跨供应商），按权重路由
// 健康度/熔断/路由分流/通道来源 全部来自真实请求的被动统计

// 被动统计的通道健康（后端未实现，先 mock；字段学 LiteLLM cooldown_handlers）
interface ChannelHealth {
  requests: number        // 窗口内请求数
  success_rate: number    // 成功率 %
  avg_ttft_ms: number     // 平均首字
  error_count: number     // 错误数
  cooldown_until?: number // 熔断截止时间戳（有=熔断中）
  cooldown_reason?: string
}

// 一个模型下某条路由的运行态（= 通道引用 + 被动健康）
interface ModelRouteHealth extends ModelRoute {
  health: ChannelHealth
}

// 带健康度的模型（= ExposedModel，但 routes 带 health）
interface ModelWithHealth extends Omit<ExposedModel, 'routes'> {
  routes: ModelRouteHealth[]
}

const DEFAULT_MODELS: ModelWithHealth[] = [
  {
    id: 'm-1', alias: 'deepseek-v4-flash', kind: 'chat',
    routes: [
      { id: 'r-1', provider_name: 'SenseNova', channel_name: '带key-ipv4', real_name: 'deepseek-v4-flash', weight: 8, health: { requests: 812, success_rate: 99.1, avg_ttft_ms: 180, error_count: 7 } },
      { id: 'r-2', provider_name: 'SenseNova', channel_name: '带key-ipv6', real_name: 'deepseek-v4.1-flash', weight: 2, health: { requests: 198, success_rate: 96.5, avg_ttft_ms: 240, error_count: 7, cooldown_until: Date.now() + 120000, cooldown_reason: '失败率 100%（近 1 分钟 ≥ 5 次）' } },
    ],
  },
  {
    id: 'm-2', alias: 'tavily/search', kind: 'webSearch',
    routes: [
      { id: 'r-3', provider_name: 'tavily', channel_name: '带key-ipv4', real_name: 'tavily/search', weight: 6, health: { requests: 640, success_rate: 98.8, avg_ttft_ms: 320, error_count: 8 } },
      { id: 'r-4', provider_name: 'tavily', channel_name: '无key-ipv6', real_name: 'tavily/search', weight: 4, health: { requests: 410, success_rate: 92.1, avg_ttft_ms: 510, error_count: 33 } },
    ],
  },
  {
    id: 'm-3', alias: 'text-embedding-3-small', kind: 'embedding',
    routes: [
      { id: 'r-5', provider_name: 'openai', channel_name: '主通道', real_name: 'text-embedding-3-small', weight: 10, health: { requests: 980, success_rate: 100, avg_ttft_ms: 90, error_count: 0 } },
    ],
  },
  {
    id: 'm-4', alias: 'agnes-3.0-flash', kind: 'chat',
    routes: [
      { id: 'r-6', provider_name: 'agnes', channel_name: '主通道', real_name: 'agnes-3.0-flash', weight: 5, health: { requests: 500, success_rate: 97.0, avg_ttft_ms: 200, error_count: 15 } },
      { id: 'r-7', provider_name: 'pie-xian', channel_name: '主通道', real_name: 'agnes-2.5-flash', weight: 5, health: { requests: 500, success_rate: 0, avg_ttft_ms: 0, error_count: 500, cooldown_until: Date.now() + 300000, cooldown_reason: '失败率 100%（持续）' } },
    ],
  },
]

// 模型整体健康：由各通道加权聚合
function modelHealth(m: ModelWithHealth): { status: 'healthy' | 'degraded' | 'error'; successRate: number; avgTtft: number; requests: number } {
  const routes = m.routes as ModelRouteHealth[]
  const totalReq = routes.reduce((s, r) => s + r.health.requests, 0)
  if (totalReq === 0) return { status: 'healthy', successRate: 100, avgTtft: 0, requests: 0 }
  const weighted = routes.reduce((s, r) => s + r.health.success_rate * r.health.requests, 0) / totalReq
  const ttft = routes.reduce((s, r) => s + r.health.avg_ttft_ms * r.health.requests, 0) / totalReq
  const status = weighted >= 99 ? 'healthy' : weighted >= 90 ? 'degraded' : 'error'
  return { status, successRate: weighted, avgTtft: ttft, requests: totalReq }
}

// 路由实际命中比（按权重归一化）
function hitRatio(m: ModelWithHealth): Array<{ route: ModelRouteHealth; pct: number }> {
  const routes = m.routes as ModelRouteHealth[]
  const totalW = routes.reduce((s, r) => s + r.weight, 0)
  return routes.map(r => ({ route: r, pct: totalW > 0 ? Math.round((r.weight / totalW) * 100) : 0 }))
}

const KIND_LABEL: Record<ModelKind, string> = {
  chat: '对话', webSearch: '搜索', webFetch: '抓取', embedding: '嵌入', vision: '识图',
}

export default function Models() {
  const [models, setModels] = useState<ModelWithHealth[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => { loadModels() }, [])

  async function loadModels() {
    try {
      // 后端未实现，回退 mock（被动统计示例）
      const res = await fetch('/v1/models')
      const data = await res.json()
      setModels(data.data?.length ? data.data : DEFAULT_MODELS)
    } catch {
      setModels(DEFAULT_MODELS)
    } finally { setLoading(false) }
  }

  if (loading) return <div className="animate-pulse text-gray-500">加载中…</div>

  const summary = {
    total: models.length,
    healthy: models.filter(m => modelHealth(m).status === 'healthy').length,
    degraded: models.filter(m => modelHealth(m).status === 'degraded').length,
    error: models.filter(m => modelHealth(m).status === 'error').length,
  }

  return (
    <div className="space-y-4">
      {/* 头部（只读，无编辑按钮） */}
      <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div>
          <h2 className="text-lg font-semibold">模型健康度（{models.length}）</h2>
          <p className="text-sm text-gray-500">
            被动统计真实请求 · 健康度 / 熔断 / 路由分流 / 通道来源（配置在供应商页）
          </p>
        </div>
      </div>

      {/* 总览卡（学 StatsHero） */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        {[
          { label: '模型总数', value: summary.total, sub: '对外暴露', icon: Layers, color: 'text-blue-600' },
          { label: '健康', value: summary.healthy, sub: '≥99% 成功率', icon: ShieldCheck, color: 'text-green-600' },
          { label: '降级', value: summary.degraded, sub: '90~99%', icon: AlertTriangle, color: 'text-amber-600' },
          { label: '异常', value: summary.error, sub: '<90% / 熔断', icon: PauseCircle, color: 'text-red-600' },
        ].map(c => (
          <div key={c.label} className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
            <div className="flex items-center gap-2 text-xs text-gray-400"><c.icon className={`h-4 w-4 ${c.color}`} />{c.label}</div>
            <div className="mt-1 text-2xl font-semibold tabular-nums">{c.value}</div>
            <div className="text-xs text-gray-400">{c.sub}</div>
          </div>
        ))}
      </div>

      {/* 模型卡片（只读） */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        {models.map((m) => {
          const agg = modelHealth(m)
          const statusColor = agg.status === 'healthy' ? 'bg-green-500' : agg.status === 'degraded' ? 'bg-amber-500' : 'bg-red-500'
          const statusText = agg.status === 'healthy' ? '健康' : agg.status === 'degraded' ? '降级' : '异常'
          return (
            <div key={m.id} className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
              {/* 模型头：alias + 类型 + 整体健康 */}
              <div className="mb-3 flex items-center gap-2">
                <Layers className="h-4 w-4 text-blue-600" />
                <span className="font-mono font-semibold">{m.alias}</span>
                <span className="rounded bg-gray-100 px-1.5 py-0.5 text-[10px] text-gray-500 dark:bg-gray-700">{KIND_LABEL[m.kind || 'chat']}</span>
                <span className="ml-auto flex items-center gap-1 text-xs">
                  <span className={`h-2 w-2 rounded-full ${statusColor}`} />
                  <span className="text-gray-500">{statusText}</span>
                </span>
              </div>

              {/* 聚合指标 */}
              <div className="mb-3 grid grid-cols-3 gap-2 text-center">
                <div className="rounded-lg bg-gray-50 p-2 dark:bg-gray-700/50">
                  <div className="text-xs text-gray-400">成功率</div>
                  <div className="font-mono text-sm font-semibold">{agg.successRate.toFixed(1)}%</div>
                </div>
                <div className="rounded-lg bg-gray-50 p-2 dark:bg-gray-700/50">
                  <div className="flex items-center justify-center gap-0.5 text-xs text-gray-400"><Clock className="h-3 w-3" />首字</div>
                  <div className="font-mono text-sm font-semibold">{Math.round(agg.avgTtft)}ms</div>
                </div>
                <div className="rounded-lg bg-gray-50 p-2 dark:bg-gray-700/50">
                  <div className="flex items-center justify-center gap-0.5 text-xs text-gray-400"><Activity className="h-3 w-3" />请求数</div>
                  <div className="font-mono text-sm font-semibold">{agg.requests.toLocaleString()}</div>
                </div>
              </div>

              {/* 通道来源 + 路由分流 + 各通道健康 */}
              <div className="space-y-2">
                {hitRatio(m).map(({ route, pct }) => {
                  const h = route.health
                  const cooling = !!h.cooldown_until && h.cooldown_until > Date.now()
                  return (
                    <div key={route.id} className={`rounded-lg border p-2 ${cooling ? 'border-red-200 bg-red-50/50 dark:border-red-900/40 dark:bg-red-900/10' : 'border-gray-200 dark:border-gray-700'}`}>
                      {/* 通道行：供应商/通道名 + 上游真实名 + 权重命中比 */}
                      <div className="flex flex-wrap items-center gap-2 text-xs">
                        <span className="font-medium text-gray-700 dark:text-gray-300">{route.provider_name}</span>
                        <span className="text-gray-400">·</span>
                        <span className="font-mono text-gray-500">{route.channel_name}</span>
                        <span className="ml-auto flex items-center gap-1">
                          {cooling && <PauseCircle className="h-3.5 w-3.5 text-red-500" />}
                          <Gauge className="h-3 w-3 text-gray-400" />
                          <span className="font-mono text-gray-500">{pct}%</span>
                        </span>
                      </div>
                      <div className="mt-0.5 flex flex-wrap items-center gap-2 text-[11px] text-gray-400">
                        <span className="font-mono">{route.real_name}</span>
                        <span>·</span>
                        <span>成功率 {h.success_rate.toFixed(1)}%</span>
                        <span>·</span>
                        <span>首字 {h.avg_ttft_ms}ms</span>
                        <span>·</span>
                        <span>{h.requests} 次 · {h.error_count} 错</span>
                      </div>
                      {/* 命中比进度条 */}
                      <div className="mt-1 h-1 overflow-hidden rounded-full bg-gray-100 dark:bg-gray-700">
                        <div className={`h-full ${cooling ? 'bg-red-400' : 'bg-blue-500'}`} style={{ width: `${pct}%` }} />
                      </div>
                      {/* 熔断原因 */}
                      {cooling && h.cooldown_reason && (
                        <div className="mt-1 text-[11px] text-red-600 dark:text-red-400">
                          熔断中 · 剩 {Math.ceil(((h.cooldown_until ?? 0) - Date.now()) / 1000)}s · {h.cooldown_reason}
                        </div>
                      )}
                    </div>
                  )
                })}
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}
