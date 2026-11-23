import { useEffect, useState } from 'react'
import { Plus, Trash2, ChevronDown, ChevronRight, Key } from 'lucide-react'
import { toast } from '@/components/Toast'
import StatusBar from '@/components/StatusBar'
import { postJSON } from '@/lib/api'
import type { Provider } from '@/lib/types'

export default function Providers() {
  const [providers, setProviders] = useState<Provider[]>([])
  const [expanded, setExpanded] = useState<Set<number>>(new Set())
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    loadProviders()
  }, [])

  async function loadProviders() {
    try {
      const res = await fetch('/api/providers')
      const data = await res.json()
      setProviders(data.providers || [])
    } catch {
      // 后端还没写，用示例数据（学cpa 的结构）
      setProviders([
        {
          name: 'agnes',
          base_url: 'https://apihub.agnes-ai.com/v1',
          disabled: false,
          headers: { 'User-Agent': 'opencode/1.18.30' },
          keys: [{ 'api-key': 'sk-xxx' }, { 'api-key': 'sk-yyy' }, { 'api-key': 'sk-zzz' }],
          models: [
            { name: 'agnes-2.5-flash' },
            { name: 'agnes-3.0-flash' },
          ],
        },
        {
          name: 'SenseNova',
          base_url: 'https://token.sensenova.cn/v1',
          disabled: false,
          headers: { 'User-Agent': 'opencode/1.18.30' },
          keys: [{ 'api-key': 'sk-xxx' }, { 'api-key': 'sk-yyy' }, { 'api-key': 'sk-zzz' }],
          models: [
            { name: 'deepseek-v4-flash', alias: 'deepseek-v4-flash' },
            { name: 'sensenova-6.8-flash-lite', alias: '' },
            { name: 'deepseek-flash', alias: 'deepseek-v4-flash' },
            { name: 'deepseek-v4.1-flash', alias: 'deepseek-v4-flash' },
            { name: 'sensenova-6.8-flash' },
          ],
        },
        {
          name: 'pie-xian',
          base_url: 'https://api.pie-xian.com/v1',
          disabled: false,
          headers: { 'User-Agent': 'opencode/1.18.30' },
          keys: [{ 'api-key': 'sk-xxx' }],
          models: [
            { name: 'agnes-2.5-flash', alias: 'agnes-3.0-flash' },
            { name: 'agnes-3.0-flash' },
            { name: 'minimax-m3.1-flash' },
            { name: 'nemotron-3.5-super-vl' },
            { name: 'step-5-preview' },
            { name: 'grok-4.7' },
            { name: 'ernie-5.5-flash-preview' },
            { name: 'gemini-3.1-flash-lite', alias: 'gemini-3.5-flash-lite' },
            { name: 'gemini-3.5-flash-lite' },
            { name: 'hy4-flash-free', alias: 'hy4' },
          ],
        },
      ])
    }
  }

  function toggleExpand(idx: number) {
    const newExpanded = new Set(expanded)
    if (newExpanded.has(idx)) newExpanded.delete(idx)
    else newExpanded.add(idx)
    setExpanded(newExpanded)
  }

  function addProvider() {
    setProviders([...providers, {
      name: '',
      base_url: '',
      disabled: false,
      headers: { 'User-Agent': 'opencode/1.18.30' },
      keys: [],
      models: [],
    }])
  }

  function removeProvider(idx: number) {
    setProviders(providers.filter((_, i) => i !== idx))
  }

  function updateProvider(idx: number, field: keyof Provider, value: any) {
    const newProviders = [...providers]
    newProviders[idx] = { ...newProviders[idx], [field]: value }
    setProviders(newProviders)
  }

  function addKey(idx: number) {
    const newProviders = [...providers]
    newProviders[idx].keys.push({ 'api-key': '' })
    setProviders(newProviders)
  }

  function removeKey(pidx: number, kidx: number) {
    const newProviders = [...providers]
    newProviders[pidx].keys.splice(kidx, 1)
    setProviders(newProviders)
  }

  function updateKey(pidx: number, kidx: number, value: string) {
    const newProviders = [...providers]
    newProviders[pidx].keys[kidx]['api-key'] = value
    setProviders(newProviders)
  }

  function addModel(idx: number) {
    const newProviders = [...providers]
    newProviders[idx].models.push({ name: '', alias: '' })
    setProviders(newProviders)
  }

  function removeModel(pidx: number, midx: number) {
    const newProviders = [...providers]
    newProviders[pidx].models.splice(midx, 1)
    setProviders(newProviders)
  }

  function updateModel(pidx: number, midx: number, field: 'name' | 'alias', value: string) {
    const newProviders = [...providers]
    newProviders[pidx].models[midx][field] = value
    setProviders(newProviders)
  }

  async function saveConfig() {
    setBusy(true)
    try {
      await postJSON('/api/providers', { providers })
      toast('配置已保存')
    } catch {
      toast('保存失败')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">Providers ({providers.length})</h2>
        <div className="flex gap-2">
          <button onClick={addProvider} className="flex items-center gap-1 rounded-lg bg-blue-600 px-3 py-1.5 text-xs text-white hover:bg-blue-700">
            <Plus className="h-3 w-3" /> 添加 Provider
          </button>
          <button onClick={saveConfig} disabled={busy} className="rounded-lg bg-green-600 px-3 py-1.5 text-xs text-white hover:bg-green-700 disabled:opacity-50">
            保存
          </button>
        </div>
      </div>

      {providers.map((p, idx) => (
        <div key={idx} className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
          {/* Provider 头部 */}
          <div className="flex items-center gap-3 p-4">
            <button onClick={() => toggleExpand(idx)} className="text-gray-400">
              {expanded.has(idx) ? <ChevronDown className="h-5 w-5" /> : <ChevronRight className="h-5 w-5" />}
            </button>
            <input
              value={p.name}
              onChange={e => updateProvider(idx, 'name', e.target.value)}
              placeholder="provider-name"
              className="w-40 bg-transparent font-semibold outline-none"
            />
            <input
              value={p.base_url}
              onChange={e => updateProvider(idx, 'base_url', e.target.value)}
              placeholder="https://api.example.com/v1"
              className="flex-1 rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700"
            />
            <span className="rounded bg-gray-100 px-2 py-0.5 text-xs text-gray-500 dark:bg-gray-700">
              {p.keys.length} keys · {p.models.length} models
            </span>
            <button onClick={() => removeProvider(idx)} className="text-red-500 hover:text-red-700">
              <Trash2 className="h-4 w-4" />
            </button>
          </div>

          {/* 展开内容 */}
          {expanded.has(idx) && (
            <div className="border-t border-gray-200 p-4 dark:border-gray-700">
              {/* Keys */}
              <div className="mb-4">
                <div className="mb-2 flex items-center gap-2">
                  <Key className="h-4 w-4 text-gray-400" />
                  <span className="text-sm font-medium">API Keys</span>
                  <button onClick={() => addKey(idx)} className="text-xs text-blue-600 hover:text-blue-700">+ 添加</button>
                </div>
                <div className="space-y-1">
                  {p.keys.map((k, kidx) => (
                    <div key={kidx} className="flex gap-2">
                      <input
                        value={k['api-key']}
                        onChange={e => updateKey(idx, kidx, e.target.value)}
                        type="password"
                        placeholder="sk-..."
                        className="flex-1 rounded border border-gray-300 px-2 py-1 text-sm font-mono dark:border-gray-600 dark:bg-gray-700"
                      />
                      <button onClick={() => removeKey(idx, kidx)} className="text-red-500 hover:text-red-700">
                        <Trash2 className="h-3 w-3" />
                      </button>
                    </div>
                  ))}
                </div>
              </div>

              {/* Models */}
              <div>
                <div className="mb-2 flex items-center justify-between">
                  <span className="text-sm font-medium">Models</span>
                  <button onClick={() => addModel(idx)} className="text-xs text-blue-600 hover:text-blue-700">+ 添加</button>
                </div>
                <table className="w-full text-left text-sm">
                  <thead>
                    <tr className="border-b border-gray-200 dark:border-gray-700">
                      <th className="pb-2 font-medium text-gray-500">Name（上游真实名）</th>
                      <th className="pb-2 font-medium text-gray-500">Alias（对外暴露，空=用name）</th>
                      <th className="pb-2"></th>
                    </tr>
                  </thead>
                  <tbody>
                    {p.models.map((m, midx) => (
                      <tr key={midx}>
                        <td className="py-1 pr-2">
                          <input
                            value={m.name}
                            onChange={e => updateModel(idx, midx, 'name', e.target.value)}
                            placeholder="real-model-name"
                            className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                          />
                        </td>
                        <td className="py-1 pr-2">
                          <input
                            value={m.alias || ''}
                            onChange={e => updateModel(idx, midx, 'alias', e.target.value)}
                            placeholder="（空 = 用 name）"
                            className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                          />
                        </td>
                        <td className="py-1">
                          <button onClick={() => removeModel(idx, midx)} className="text-red-500 hover:text-red-700">
                            <Trash2 className="h-3 w-3" />
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </div>
      ))}

      <StatusBar busy={busy} />
    </div>
  )
}
