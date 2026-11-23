import { useEffect, useState } from 'react'
import { Layers, Plus, Trash2, RefreshCw, Save, Pencil, Check, X } from 'lucide-react'
import { toast } from '@/components/Toast'
import type { ExposedModel, ModelKind } from '@/lib/types'
import { MODEL_KINDS } from '@/lib/types'

// 模仿 CPA 模型映射：alias（对外名）→ 多个 provider 真实模型 + 权重（路由分流）
const DEFAULT_MODELS: ExposedModel[] = [
  { id: 'm-0', alias: 'tavily/search', kind: 'webSearch', providers: [
    { provider_name: 'tavily', real_name: 'tavily/search', weight: 10 },
  ]},
  { id: 'm-00', alias: 'text-embedding-3-small', kind: 'embedding', providers: [
    { provider_name: 'openai', real_name: 'text-embedding-3-small', weight: 10 },
  ]},
  { id: 'm-1', alias: 'deepseek-v4-flash', providers: [
    { provider_name: 'SenseNova', real_name: 'deepseek-v4-flash', weight: 8 },
    { provider_name: 'SenseNova', real_name: 'deepseek-flash', weight: 0 },
    { provider_name: 'SenseNova', real_name: 'deepseek-v4.1-flash', weight: 2 },
  ]},
  { id: 'm-2', alias: 'agnes-3.0-flash', providers: [
    { provider_name: 'agnes', real_name: 'agnes-3.0-flash', weight: 5 },
    { provider_name: 'pie-xian', real_name: 'agnes-2.5-flash', weight: 5 },
  ]},
  { id: 'm-3', alias: 'gemini-3.5-flash-lite', providers: [
    { provider_name: 'pie-xian', real_name: 'gemini-3.1-flash-lite', weight: 5 },
    { provider_name: 'pie-xian', real_name: 'gemini-3.5-flash-lite', weight: 5 },
  ]},
  { id: 'm-4', alias: 'hy4', providers: [
    { provider_name: 'pie-xian', real_name: 'hy4-flash-free', weight: 10 },
  ]},
  { id: 'm-5', alias: 'sensenova-6.8-flash', providers: [
    { provider_name: 'SenseNova', real_name: 'sensenova-6.8-flash', weight: 5 },
    { provider_name: 'iamhc', real_name: 'sensenova-6.8-flash', weight: 5 },
  ]},
]

export default function Models() {
  const [models, setModels] = useState<ExposedModel[]>([])
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState<ExposedModel | null>(null)
  const [kindFilter, setKindFilter] = useState<ModelKind | 'all'>('all')

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
    const m: ExposedModel = { id: `m-${Date.now()}`, alias: '', kind: kindFilter === 'all' ? 'chat' : kindFilter, providers: [] }
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

  function addProvider() {
    if (!draft) return
    setDraft({ ...draft, providers: [...draft.providers, { provider_name: '', real_name: '', weight: 1 }] })
  }
  function updateProvider(pi: number, field: 'provider_name' | 'real_name' | 'weight', value: any) {
    if (!draft) return
    const ps = [...draft.providers]
    ps[pi] = { ...ps[pi], [field]: value }
    setDraft({ ...draft, providers: ps })
  }
  function removeProvider(pi: number) {
    if (!draft) return
    setDraft({ ...draft, providers: draft.providers.filter((_, i) => i !== pi) })
  }
  function removeModel(m: ExposedModel) {
    if (!confirm(`删除模型映射「${m.alias}」？`)) return
    setModels(models.filter(x => x.id !== m.id))
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

  // 权重占比（供进度条）
  function weightPct(p: { weight: number }) {
    if (!draft) return 0
    const total = draft.providers.reduce((s, x) => s + x.weight, 0)
    return total > 0 ? Math.round((p.weight / total) * 100) : 0
  }

  if (loading) return <div className="animate-pulse text-gray-500">加载中…</div>

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold">模型映射（{models.length}）</h2>
          <p className="text-sm text-gray-500">
            模仿 CPA：alias（对外名）→ 多个上游真实模型 + 权重（路由分流，weight 高的调用多）
          </p>
        </div>
        <div className="flex gap-2">
          <button onClick={saveAll} disabled={busy} className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700 disabled:opacity-50">
            <Save className="h-4 w-4" />保存
          </button>
          <button onClick={addModel} className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">
            <Plus className="h-4 w-4" />添加映射
          </button>
        </div>
      </div>

      {/* 模型类型筛选（学 9router /v1/models 分类）*/}
      <div className="flex flex-wrap gap-1 rounded-lg bg-gray-100 p-1 dark:bg-gray-700">
        <button onClick={() => setKindFilter('all')}
          className={`rounded px-3 py-1 text-sm ${kindFilter === 'all' ? 'bg-white text-blue-600 shadow-sm dark:bg-gray-800' : 'text-gray-500 hover:text-gray-700'}`}>
          全部（{models.length}）
        </button>
        {MODEL_KINDS.map(kind => {
          const count = models.filter(m => (m.kind || 'chat') === kind.id).length
          return (
            <button key={kind.id} onClick={() => setKindFilter(kind.id)}
              className={`rounded px-3 py-1 text-sm ${kindFilter === kind.id ? 'bg-white text-blue-600 shadow-sm dark:bg-gray-800' : 'text-gray-500 hover:text-gray-700'}`}>
              {kind.label}（{count}）
            </button>
          )
        })}
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        {models.filter(m => kindFilter === 'all' || (m.kind || 'chat') === kindFilter).map((m) => {
          const editing = editingId === m.id
          const totalWeight = m.providers.reduce((s, p) => s + p.weight, 0)
          return (
            <div key={m.id} className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
              {editing && draft ? (
                /* 编辑模式 */
                <div>
                  <div className="mb-3 flex items-center gap-2">
                    <Layers className="h-4 w-4 text-blue-600" />
                    <input value={draft.alias} onChange={e => setDraft({ ...draft, alias: e.target.value })}
                      placeholder="对外模型名（alias）"
                      className="flex-1 rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700" />
                  </div>
                  <div className="mb-2 text-xs text-gray-400">上游真实模型 + 权重（同一 alias 按权重分流）</div>
                  <div className="space-y-1.5">
                    {draft.providers.map((p, pi) => (
                      <div key={pi} className="flex items-center gap-2">
                        <input value={p.provider_name} onChange={e => updateProvider(pi, 'provider_name', e.target.value)} placeholder="provider"
                          className="w-28 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                        <input value={p.real_name} onChange={e => updateProvider(pi, 'real_name', e.target.value)} placeholder="上游真实模型"
                          className="flex-1 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                        <input type="number" value={p.weight} onChange={e => updateProvider(pi, 'weight', parseInt(e.target.value) || 0)} placeholder="权重"
                          className="w-16 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                        <button onClick={() => removeProvider(pi)} className="text-red-500 hover:text-red-700"><Trash2 className="h-3 w-3" /></button>
                      </div>
                    ))}
                  </div>
                  <div className="mt-3 flex gap-2">
                    <button onClick={addProvider} className="flex items-center gap-1 text-xs text-blue-600 hover:text-blue-700"><Plus className="h-3 w-3" />添加上游</button>
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
                    <span className={`rounded px-1.5 py-0.5 text-[10px] ${m.kind === 'webSearch' ? 'bg-teal-100 text-teal-700 dark:bg-teal-900/20' : m.kind === 'embedding' ? 'bg-purple-100 text-purple-700 dark:bg-purple-900/20' : m.kind === 'webFetch' ? 'bg-cyan-100 text-cyan-700 dark:bg-cyan-900/20' : 'bg-gray-100 text-gray-500 dark:bg-gray-700'}`}>
                      {MODEL_KINDS.find(k => k.id === (m.kind || 'chat'))?.label || 'chat'}
                    </span>
                    <span className="ml-auto rounded bg-blue-50 px-2 py-0.5 text-xs text-blue-600 dark:bg-blue-900/20">
                      {m.providers.length} provider{m.providers.length > 1 ? 's' : ''} · w={totalWeight}
                    </span>
                  </div>
                  <div className="space-y-1.5">
                    {m.providers.map((p, pi) => {
                      const pct = totalWeight > 0 ? Math.round((p.weight / totalWeight) * 100) : 0
                      return (
                        <div key={pi}>
                          <div className="flex items-center gap-2 text-sm">
                            <span className="w-24 text-gray-500">{p.provider_name}</span>
                            <span className="flex-1 font-mono text-xs text-gray-400">{p.real_name}</span>
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
                    <button onClick={() => startEdit(m)} className="text-gray-400 hover:text-blue-600" title="编辑映射"><Pencil className="h-3.5 w-3.5" /></button>
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
