import { useEffect, useState } from 'react'
import { Plus, Trash2, ChevronDown, ChevronRight, Key, Layers, Globe } from 'lucide-react'
import { toast } from '@/components/Toast'
import StatusBar from '@/components/StatusBar'
import { postJSON } from '@/lib/api'
import type { Provider, ModelProtocol } from '@/lib/types'
import { MODEL_PROTOCOLS } from '@/lib/types'

export default function Providers() {
  const [providers, setProviders] = useState<Provider[]>([])
  const [expanded, setExpanded] = useState<Set<number>>(new Set())
  const [activeProtocol, setActiveProtocol] = useState<Record<number, ModelProtocol>>({})
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
      // 后端还没写，用示例数据（学 myapi 的结构，模型级协议）
      setProviders([
        {
          name: 'agnes',
          base_url: 'https://apihub.agnes-ai.com/v1',
          disabled: false,
          headers: { 'User-Agent': 'opencode/1.18.30' },
          keys: [{ 'api-key': 'sk-xxx' }, { 'api-key': 'sk-yyy' }, { 'api-key': 'sk-zzz' }],
          models: [
            { name: 'agnes-2.5-flash', protocol: 'openai_chat' },
            { name: 'agnes-3.0-flash', protocol: 'openai_chat' },
          ],
        },
        {
          name: 'SenseNova',
          base_url: 'https://token.sensenova.cn/v1',
          disabled: false,
          headers: { 'User-Agent': 'opencode/1.18.30' },
          keys: [{ 'api-key': 'sk-xxx' }, { 'api-key': 'sk-yyy' }, { 'api-key': 'sk-zzz' }],
          models: [
            { name: 'deepseek-v4-flash', alias: 'deepseek-v4-flash', protocol: 'openai_chat' },
            { name: 'sensenova-6.8-flash-lite', alias: '', protocol: 'openai_chat' },
            { name: 'deepseek-flash', alias: 'deepseek-v4-flash', protocol: 'openai_chat' },
            { name: 'deepseek-v4.1-flash', alias: 'deepseek-v4-flash', protocol: 'openai_responses' },
            { name: 'sensenova-6.8-flash', protocol: 'openai_responses' },
          ],
        },
        {
          name: 'pie-xian',
          base_url: 'https://api.pie-xian.com/v1',
          disabled: false,
          headers: { 'User-Agent': 'opencode/1.18.30' },
          keys: [{ 'api-key': 'sk-xxx' }],
          models: [
            { name: 'agnes-2.5-flash', alias: 'agnes-3.0-flash', protocol: 'anthropic' },
            { name: 'agnes-3.0-flash', protocol: 'anthropic' },
            { name: 'minimax-m3.1-flash', protocol: 'openai_chat' },
            { name: 'nemotron-3.5-super-vl', protocol: 'openai_chat' },
            { name: 'step-5-preview', protocol: 'openai_responses' },
            { name: 'grok-4.7', protocol: 'openai_responses' },
            { name: 'ernie-5.5-flash-preview', protocol: 'openai_chat' },
            { name: 'gemini-3.1-flash-lite', alias: 'gemini-3.5-flash-lite', protocol: 'gemini' },
            { name: 'gemini-3.5-flash-lite', protocol: 'gemini' },
            { name: 'hy4-flash-free', alias: 'hy4', protocol: 'ollama' },
          ],
        },
      ])
    } finally {
      setBusy(false)
    }
  }

  function toggleExpand(idx: number) {
    const newExpanded = new Set(expanded)
    newExpanded.has(idx) ? newExpanded.delete(idx) : newExpanded.add(idx)
    setExpanded(newExpanded)
  }

  function addProvider() {
    setProviders([...providers, { name: '', base_url: '', disabled: false, headers: { 'User-Agent': 'opencode/1.18.30' }, keys: [{ 'api-key': '' }], models: [] }])
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

  function addModel(idx: number, protocol: ModelProtocol) {
    const newProviders = [...providers]
    newProviders[idx].models.push({ name: '', alias: '', protocol })
    setProviders(newProviders)
  }

  function removeModel(pidx: number, midx: number) {
    const newProviders = [...providers]
    newProviders[pidx].models.splice(midx, 1)
    setProviders(newProviders)
  }

  function updateModel(pidx: number, midx: number, field: 'name' | 'alias' | 'protocol', value: string) {
    const newProviders = [...providers]
    newProviders[pidx].models[midx][field] = value as any
    setProviders(newProviders)
  }

  async function saveConfig() {
    setBusy(true)
    try {
      await postJSON('/api/providers', { providers })
      toast('供应商已保存')
    } catch {
      toast('后端未实现，仅本地保存')
    } finally {
      setBusy(false)
    }
  }

  // 每个供应商的协议 tab 统计
  function protocolCount(p: Provider, proto: ModelProtocol) {
    return p.models.filter(m => (m.protocol || 'openai_chat') === proto).length
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold">Providers</h2>
          <p className="text-sm text-gray-500">
            上游供应商 · 一个供应商可绑定不同协议的模型（模型级协议）
          </p>
        </div>
        <div className="flex gap-2">
          <button onClick={saveConfig} disabled={busy} className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700 disabled:opacity-50">
            <Key className="h-4 w-4" />保存
          </button>
          <button onClick={addProvider} className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">
            <Plus className="h-4 w-4" />添加供应商
          </button>
        </div>
      </div>

      {providers.map((p, idx) => {
        const open = expanded.has(idx)
        const activeProto = activeProtocol[idx] || 'openai_chat'
        return (
          <div key={idx} className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
            {/* 供应商头部 */}
            <div className="flex items-center gap-3 p-4">
              <button onClick={() => toggleExpand(idx)} className="text-gray-400">
                {open ? <ChevronDown className="h-5 w-5" /> : <ChevronRight className="h-5 w-5" />}
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
            {open && (
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

                {/* 协议 tab（模型级协议分组，学 sub2api 模型分流） */}
                <div className="mb-3">
                  <div className="mb-2 flex items-center gap-2">
                    <Globe className="h-4 w-4 text-gray-400" />
                    <span className="text-sm font-medium">协议（按模型分流）</span>
                  </div>
                  <div className="flex flex-wrap gap-1 rounded-lg bg-gray-100 p-1 dark:bg-gray-700">
                    {MODEL_PROTOCOLS.map(proto => {
                      const count = protocolCount(p, proto.id)
                      return (
                        <button
                          key={proto.id}
                          onClick={() => setActiveProtocol({ ...activeProtocol, [idx]: proto.id })}
                          className={`flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs ${
                            activeProto === proto.id
                              ? 'bg-white font-semibold text-blue-600 shadow dark:bg-gray-800'
                              : 'text-gray-500 hover:text-gray-900 dark:hover:text-gray-100'
                          }`}
                        >
                          {proto.label}
                          <span className={`rounded px-1 text-[9px] ${count > 0 ? 'bg-blue-100 text-blue-600 dark:bg-blue-900/30' : 'bg-gray-200 text-gray-400 dark:bg-gray-600'}`}>
                            {count}
                          </span>
                        </button>
                      )
                    })}
                  </div>
                </div>

                {/* 当前协议的模型列表 */}
                <div>
                  <div className="mb-2 flex items-center justify-between">
                    <span className="flex items-center gap-2 text-sm font-medium">
                      <Layers className="h-4 w-4 text-gray-400" />
                      {MODEL_PROTOCOLS.find(x => x.id === activeProto)?.label} 模型
                      <span className="font-mono text-xs text-gray-400">{MODEL_PROTOCOLS.find(x => x.id === activeProto)?.path}</span>
                    </span>
                    <button onClick={() => addModel(idx, activeProto)} className="text-xs text-blue-600 hover:text-blue-700">+ 添加</button>
                  </div>
                  <table className="w-full text-left text-sm">
                    <thead>
                      <tr className="border-b border-gray-200 dark:border-gray-700">
                        <th className="pb-2 font-medium text-gray-500">Name（上游真实名）</th>
                        <th className="pb-2 font-medium text-gray-500">Alias（对外暴露，空=用name）</th>
                        <th className="pb-2 font-medium text-gray-500">协议</th>
                        <th className="pb-2 w-10"></th>
                      </tr>
                    </thead>
                    <tbody>
                      {p.models
                        .filter(m => (m.protocol || 'openai_chat') === activeProto)
                        .map((m, mi) => {
                          const realIdx = p.models.findIndex(x => x === m)
                          return (
                            <tr key={realIdx}>
                              <td className="py-1 pr-2">
                                <input
                                  value={m.name}
                                  onChange={e => updateModel(idx, realIdx, 'name', e.target.value)}
                                  placeholder="real-model-name"
                                  className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                                />
                              </td>
                              <td className="py-1 pr-2">
                                <input
                                  value={m.alias || ''}
                                  onChange={e => updateModel(idx, realIdx, 'alias', e.target.value)}
                                  placeholder="（空 = 用 name）"
                                  className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                                />
                              </td>
                              <td className="py-1 pr-2">
                                <select
                                  value={m.protocol || 'openai_chat'}
                                  onChange={e => updateModel(idx, realIdx, 'protocol', e.target.value)}
                                  className="w-full rounded border border-gray-300 px-2 py-1 text-xs dark:border-gray-600 dark:bg-gray-700"
                                >
                                  {MODEL_PROTOCOLS.map(proto => <option key={proto.id} value={proto.id}>{proto.label}</option>)}
                                </select>
                              </td>
                              <td className="py-1">
                                <button onClick={() => removeModel(idx, realIdx)} className="text-red-500 hover:text-red-700">
                                  <Trash2 className="h-3 w-3" />
                                </button>
                              </td>
                            </tr>
                          )
                        })}
                    </tbody>
                  </table>
                  {protocolCount(p, activeProto) === 0 && (
                    <p className="py-3 text-center text-xs text-gray-400">该协议下还没有模型，点「+ 添加」</p>
                  )}
                </div>
              </div>
            )}
          </div>
        )
      })}

      <StatusBar busy={busy} onClear={() => {}} />
    </div>
  )
}
