import { useEffect, useState } from 'react'
import { Layers, Plus, Trash2, Save, Pencil, Check, X, Key, Globe, Hash } from 'lucide-react'
import { toast } from '@/components/Toast'
import type { ExposedModel, ModelRoute } from '@/lib/types'

// 模型级路由规则（学 LiteLLM deployment + 9router combo）
// 同一模型多 route：不同 key（可空=不带 key）/不同代理（IPv4/IPv6）/独立额度
const DEFAULT_MODELS: ExposedModel[] = [
  {
    id: 'm-1', alias: 'deepseek-v4-flash', kind: 'chat',
    routes: [
      { id: 'r-1', name: '带key-ipv4', provider_name: 'SenseNova', real_name: 'deepseek-v4-flash', key: 'sk-sensenova', proxy: 'http://127.0.0.1:7890', weight: 8, quota: 45231 },
      { id: 'r-2', name: '带key-ipv6', provider_name: 'SenseNova', real_name: 'deepseek-v4.1-flash', key: 'sk-sensenova', proxy: 'socks5://[::1]:1080', weight: 2, quota: 11230 },
    ],
  },
  {
    id: 'm-2', alias: 'tavily/search', kind: 'webSearch',
    routes: [
      { id: 'r-3', name: '带key-ipv4', provider_name: 'tavily', real_name: 'tavily/search', key: 'sk-tavily', proxy: 'http://127.0.0.1:7890', weight: 6, quota: 8300 },
      { id: 'r-4', name: '无key-ipv6', provider_name: 'tavily', real_name: 'tavily/search', key: '', proxy: 'socks5://[::1]:1080', weight: 4, quota: 1200 },
    ],
  },
  {
    id: 'm-3', alias: 'text-embedding-3-small', kind: 'embedding',
    routes: [
      { id: 'r-5', name: '带key', provider_name: 'openai', real_name: 'text-embedding-3-small', key: 'sk-openai', proxy: '', weight: 10, quota: 9800 },
    ],
  },
  {
    id: 'm-4', alias: 'agnes-3.0-flash', kind: 'chat',
    routes: [
      { id: 'r-6', name: '带key', provider_name: 'agnes', real_name: 'agnes-3.0-flash', key: 'sk-agnes', proxy: '', weight: 5, quota: 21000 },
      { id: 'r-7', name: '带key-alt', provider_name: 'pie-xian', real_name: 'agnes-2.5-flash', key: 'sk-pie', proxy: '', weight: 5, quota: 15000 },
    ],
  },
]

function newRoute(): ModelRoute {
  return { id: `r-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`, name: '', provider_name: '', real_name: '', key: '', proxy: '', weight: 1, quota: 0 }
}

export default function Models() {
  const [models, setModels] = useState<ExposedModel[]>([])
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState<ExposedModel | null>(null)

  useEffect(() => { loadModels() }, [])

  async function loadModels() {
    try {
      const res = await fetch('/v1/models')
      const data = await res.json()
      setModels(data.data || [])
    } catch {
      setModels(DEFAULT_MODELS)
    } finally { setLoading(false) }
  }

  function addModel() {
    const m: ExposedModel = { id: `m-${Date.now()}`, alias: '', kind: 'chat', routes: [] }
    setModels([...models, m])
    setEditingId(m.id ?? null); setDraft(m)
  }
  function startEdit(m: ExposedModel) {
    setEditingId(m.id ?? null)
    setDraft(JSON.parse(JSON.stringify(m)))
  }
  function saveEdit() {
    if (!draft) return
    if (!draft.alias.trim()) { toast('请填对外模型名（alias）'); return }
    const exists = models.some(m => m.id === draft.id)
    setModels(exists ? models.map(m => m.id === draft.id ? draft : m) : [...models, draft])
    setEditingId(null); setDraft(null)
    toast('模型映射已保存')
  }
  function cancelEdit() { setEditingId(null); setDraft(null) }
  function removeModel(m: ExposedModel) {
    if (!confirm(`删除模型「${m.alias}」？`)) return
    setModels(models.filter(x => x.id !== m.id))
  }

  // route 操作
  function addRoute() { if (draft) setDraft({ ...draft, routes: [...draft.routes, newRoute()] }) }
  function updateRoute(ri: number, field: keyof ModelRoute, value: any) {
    if (!draft) return
    const rs = [...draft.routes]
    rs[ri] = { ...rs[ri], [field]: value }
    setDraft({ ...draft, routes: rs })
  }
  function removeRoute(ri: number) {
    if (!draft) return
    setDraft({ ...draft, routes: draft.routes.filter((_, i) => i !== ri) })
  }

  async function saveAll() {
    setBusy(true)
    try {
      await fetch('/api/models', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ models }) })
      toast('模型映射已保存')
    } catch {
      toast('后端未实现，仅本地保存')
    } finally { setBusy(false) }
  }

  if (loading) return <div className="animate-pulse text-gray-500">加载中…</div>

  return (
    <div className="space-y-4">
      <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div>
          <h2 className="text-lg font-semibold">模型映射（{models.length}）</h2>
          <p className="text-sm text-gray-500">
            模型级路由：一个模型多 route（不同 key/代理/独立额度，可不带 key 请求）
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button onClick={saveAll} disabled={busy} className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700 disabled:opacity-50">
            <Save className="h-4 w-4" />保存
          </button>
          <button onClick={addModel} className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">
            <Plus className="h-4 w-4" />添加模型
          </button>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        {models.map((m) => {
          const editing = editingId === m.id
          return (
            <div key={m.id} className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
              {editing && draft ? (
                /* 编辑模式：模型 + routes */
                <div>
                  <div className="mb-3 flex items-center gap-2">
                    <Layers className="h-4 w-4 text-blue-600" />
                    <input value={draft.alias} onChange={e => setDraft({ ...draft, alias: e.target.value })}
                      placeholder="对外模型名（alias）"
                      className="flex-1 rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700" />
                  </div>
                  <div className="mb-2 text-xs text-gray-400">路由规则（调用变体）：key 可空=不带 key，代理可空=直连，独立额度</div>
                  <div className="space-y-2">
                    {draft.routes.map((r, ri) => (
                      <div key={r.id} className="rounded-lg border border-dashed border-gray-300 p-2 dark:border-gray-600">
                        <div className="flex items-center gap-1.5">
                          <input value={r.name} onChange={e => updateRoute(ri, 'name', e.target.value)} placeholder="变体名（带key-ipv4）"
                            className="w-32 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                          <input value={r.provider_name} onChange={e => updateRoute(ri, 'provider_name', e.target.value)} placeholder="渠道"
                            className="w-24 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                          <input value={r.real_name} onChange={e => updateRoute(ri, 'real_name', e.target.value)} placeholder="上游真实模型"
                            className="flex-1 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                          <input type="number" value={r.weight} onChange={e => updateRoute(ri, 'weight', parseInt(e.target.value) || 0)} placeholder="权重"
                            className="w-14 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                          <button onClick={() => removeRoute(ri)} className="text-red-500 hover:text-red-700"><Trash2 className="h-3 w-3" /></button>
                        </div>
                        <div className="mt-1.5 flex items-center gap-1.5">
                          <div className="flex flex-1 items-center gap-1">
                            <Key className="h-3 w-3 text-gray-400" />
                            <input value={r.key} onChange={e => updateRoute(ri, 'key', e.target.value)} placeholder="key（空=不带 key 请求）"
                              className="flex-1 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                          </div>
                          <div className="flex flex-1 items-center gap-1">
                            <Globe className="h-3 w-3 text-gray-400" />
                            <input value={r.proxy} onChange={e => updateRoute(ri, 'proxy', e.target.value)} placeholder="代理（空=直连）"
                              className="flex-1 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                  <div className="mt-3 flex gap-2">
                    <button onClick={addRoute} className="flex items-center gap-1 text-xs text-blue-600 hover:text-blue-700"><Plus className="h-3 w-3" />添加路由</button>
                    <div className="flex-1" />
                    <button onClick={cancelEdit} className="rounded border border-gray-300 px-2 py-1 text-xs hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700"><X className="h-3 w-3" /></button>
                    <button onClick={saveEdit} className="rounded bg-blue-600 px-3 py-1 text-xs text-white hover:bg-blue-700"><Check className="h-3 w-3" /></button>
                  </div>
                </div>
              ) : (
                /* 展示模式 */
                <div>
                  <div className="mb-3 flex items-center gap-2">
                    <Layers className="h-4 w-4 text-blue-600" />
                    <span className="font-mono font-semibold">{m.alias}</span>
                    <span className="rounded bg-gray-100 px-1.5 py-0.5 text-[10px] text-gray-500 dark:bg-gray-700">{m.kind || 'chat'}</span>
                    <span className="ml-auto rounded bg-blue-50 px-2 py-0.5 text-xs text-blue-600 dark:bg-blue-900/20">{m.routes.length} route</span>
                  </div>
                  <div className="space-y-2">
                    {m.routes.map((r) => {
                      const total = m.routes.reduce((s, x) => s + x.weight, 0)
                      const pct = total > 0 ? Math.round((r.weight / total) * 100) : 0
                      return (
                        <div key={r.id}>
                          <div className="flex items-center gap-2 text-sm">
                            <span className="w-20 truncate text-gray-500" title={r.name}>{r.name}</span>
                            <span className="w-20 text-gray-400">{r.provider_name}</span>
                            <span className="flex-1 truncate font-mono text-xs text-gray-400">{r.real_name}</span>
                            <span className={`flex items-center gap-0.5 text-[10px] ${r.key ? 'text-blue-500' : 'text-amber-500'}`}>
                              <Key className="h-3 w-3" />{r.key ? '带key' : '无key'}
                            </span>
                            <span className={`flex items-center gap-0.5 text-[10px] ${r.proxy ? 'text-teal-500' : 'text-gray-400'}`}>
                              <Globe className="h-3 w-3" />{r.proxy ? '代理' : '直连'}
                            </span>
                            <span className="flex items-center gap-0.5 text-[10px] text-gray-400"><Hash className="h-3 w-3" />{r.quota.toLocaleString()}</span>
                            <span className="font-mono text-xs text-gray-400">{pct}%</span>
                          </div>
                          <div className="mt-0.5 h-1 overflow-hidden rounded-full bg-gray-100 dark:bg-gray-700">
                            <div className="h-full rounded-full bg-blue-500" style={{ width: `${pct}%` }} />
                          </div>
                        </div>
                      )
                    })}
                  </div>
                  <div className="mt-3 flex justify-end gap-1">
                    <button onClick={() => startEdit(m)} className="text-gray-400 hover:text-blue-600" title="编辑"><Pencil className="h-3.5 w-3.5" /></button>
                    <button onClick={() => removeModel(m)} className="text-gray-400 hover:text-red-600" title="删除"><Trash2 className="h-3.5 w-3.5" /></button>
                  </div>
                </div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}
