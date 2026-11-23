import { useEffect, useState } from 'react'
import { Save, RotateCcw, Plus, Trash2 } from 'lucide-react'
import { postJSON } from '@/lib/api'
import { toast } from '@/components/Toast'
import StatusBar from '@/components/StatusBar'

interface Model {
  name: string
  alias?: string
  weight: number
}

interface Provider {
  name: string
  base_url: string
  api_key: string
  models: Model[]
}

interface GatewayConfig {
  // 网关配置
  providers: Provider[]
  routing: {
    strategy: 'latency-based' | 'round-robin'
    session_affinity: boolean
    passthrough_headers: boolean
  }
}

export default function Config() {
  const [config, setConfig] = useState<GatewayConfig | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    loadConfig()
  }, [])

  async function loadConfig() {
    try {
      const res = await fetch('/api/config')
      const data = await res.json()
      if (data.config) {
        setConfig(data.config)
      }
    } catch {
      setConfig({
        providers: [
          {
            name: 'SenseNova',
            base_url: 'https://token.sensenova.cn/v1',
            api_key: 'sk-xxx',
            models: [{ name: 'deepseek-flash', alias: 'deepseek-v4-flash', weight: 8 }],
          },
        ],
        routing: { strategy: 'latency-based', session_affinity: true, passthrough_headers: true },
      })
    }
  }

  async function saveConfig() {
    if (!config) return
    setBusy(true)
    try {
      await postJSON('/api/config', { config })
      toast('配置已保存')
    } catch (e) {
      toast('保存失败')
    } finally {
      setBusy(false)
    }
  }

  function updateProvider(idx: number, field: keyof Provider, value: any) {
    if (!config) return
    const providers = [...config.providers]
    providers[idx] = { ...providers[idx], [field]: value }
    setConfig({ ...config, providers })
  }

  function addProvider() {
    if (!config) return
    setConfig({
      ...config,
      providers: [...config.providers, { name: '', base_url: '', api_key: '', models: [] }],
    })
  }

  function removeProvider(idx: number) {
    if (!config) return
    setConfig({ ...config, providers: config.providers.filter((_, i) => i !== idx) })
  }

  if (!config) return <div className="animate-pulse text-gray-500">加载中…</div>

  return (
    <div className="space-y-4">
      {/* 网关配置：Provider */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-sm font-semibold">Provider 管理</h3>
          <button onClick={addProvider} className="flex items-center gap-1 rounded-lg bg-blue-600 px-3 py-1.5 text-xs text-white hover:bg-blue-700">
            <Plus className="h-3 w-3" />添加
          </button>
        </div>
        <div className="space-y-4">
          {config.providers.map((p, idx) => (
            <div key={idx} className="rounded-lg border border-gray-200 p-4 dark:border-gray-700">
              <div className="mb-3 flex items-center justify-between">
                <input
                  value={p.name}
                  onChange={e => updateProvider(idx, 'name', e.target.value)}
                  placeholder="Provider 名称"
                  className="bg-transparent font-semibold outline-none"
                />
                <button onClick={() => removeProvider(idx)} className="text-xs text-red-500 hover:text-red-700">
                  <Trash2 className="h-4 w-4" />
                </button>
              </div>
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <div>
                  <label className="mb-1 block text-xs text-gray-500">Base URL</label>
                  <input
                    value={p.base_url}
                    onChange={e => updateProvider(idx, 'base_url', e.target.value)}
                    className="w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700"
                  />
                </div>
                <div>
                  <label className="mb-1 block text-xs text-gray-500">API Key</label>
                  <input
                    value={p.api_key}
                    onChange={e => updateProvider(idx, 'api_key', e.target.value)}
                    type="password"
                    className="w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700"
                  />
                </div>
              </div>
              <div className="mt-3">
                <label className="mb-1 block text-xs text-gray-500">Models</label>
                {p.models.map((m, mi) => (
                  <div key={mi} className="mb-1 flex gap-2 text-sm">
                    <input value={m.name} readOnly className="flex-1 rounded border border-gray-200 px-2 py-1 dark:border-gray-700 dark:bg-gray-800" />
                    <input value={m.alias || ''} placeholder="alias" readOnly className="w-40 rounded border border-gray-200 px-2 py-1 dark:border-gray-700 dark:bg-gray-800" />
                    <input value={m.weight} readOnly className="w-16 rounded border border-gray-200 px-2 py-1 dark:border-gray-700 dark:bg-gray-800" />
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* 网关配置：路由策略 */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <h3 className="mb-4 text-sm font-semibold">路由策略</h3>
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
          <div>
            <label className="mb-1 block text-xs text-gray-500">策略</label>
            <select
              value={config.routing.strategy}
              onChange={e => setConfig({ ...config, routing: { ...config.routing, strategy: e.target.value as any } })}
              className="w-full rounded border border-gray-300 px-3 py-2 text-sm dark:border-gray-600 dark:bg-gray-700"
            >
              <option value="latency-based">延迟优先</option>
              <option value="round-robin">轮询</option>
            </select>
          </div>
          <div>
            <label className="mb-1 block text-xs text-gray-500">会话粘性</label>
            <select
              value={config.routing.session_affinity ? 'true' : 'false'}
              onChange={e => setConfig({ ...config, routing: { ...config.routing, session_affinity: e.target.value === 'true' } })}
              className="w-full rounded border border-gray-300 px-3 py-2 text-sm dark:border-gray-600 dark:bg-gray-700"
            >
              <option value="true">开启</option>
              <option value="false">关闭</option>
            </select>
          </div>
          <div>
            <label className="mb-1 block text-xs text-gray-500">透传 Headers</label>
            <select
              value={config.routing.passthrough_headers ? 'true' : 'false'}
              onChange={e => setConfig({ ...config, routing: { ...config.routing, passthrough_headers: e.target.value === 'true' } })}
              className="w-full rounded border border-gray-300 px-3 py-2 text-sm dark:border-gray-600 dark:bg-gray-700"
            >
              <option value="true">开启</option>
              <option value="false">关闭</option>
            </select>
          </div>
        </div>
      </div>

      {/* 操作按钮 */}
      <div className="flex gap-2">
        <button
          onClick={saveConfig}
          disabled={busy}
          className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-4 py-2 text-sm text-white hover:bg-blue-700 disabled:opacity-50"
        >
          <Save className="h-4 w-4" />
          保存配置
        </button>
        <button
          onClick={loadConfig}
          className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-4 py-2 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700"
        >
          <RotateCcw className="h-4 w-4" />
          重载
        </button>
      </div>

      <StatusBar busy={busy} />
    </div>
  )
}
