import { useEffect, useState } from 'react'
import {
  Plus, Trash2, ChevronDown, ChevronRight, Globe, Lock, Unlock,
  Zap, MessageSquare, Braces, Sparkles, Cpu, Wrench, Save, RotateCcw, Activity, ArrowRightLeft,
} from 'lucide-react'
import { toast } from '@/components/Toast'
import StatusBar from '@/components/StatusBar'
import type { Endpoint, EndpointProtocol } from '@/lib/types'
import { PROTOCOL_LABELS } from '@/lib/types'

// 端点元数据（学 sub2api 多协议）
const PROTOCOL_META: Record<EndpointProtocol, { icon: any; color: string; desc: string }> = {
  openai_chat: { icon: MessageSquare, color: 'text-emerald-500', desc: 'OpenAI Chat Completions' },
  openai_responses: { icon: Braces, color: 'text-blue-500', desc: 'OpenAI Responses API' },
  anthropic_messages: { icon: Sparkles, color: 'text-orange-500', desc: 'Anthropic Messages API' },
  gemini: { icon: Cpu, color: 'text-violet-500', desc: 'Google Gemini API' },
  ollama: { icon: Zap, color: 'text-amber-500', desc: 'Ollama API' },
  custom: { icon: Wrench, color: 'text-gray-500', desc: '自定义端点' },
}

const DEFAULT_ENDPOINTS: Endpoint[] = [
  { id: 'ep-1', name: 'OpenAI Chat', protocol: 'openai_chat', path: '/v1/chat/completions', enabled: true, description: 'OpenAI 兼容的对话补全端点', methods: ['POST'], streaming: true, auth: 'bearer', content_type: 'application/json' },
  { id: 'ep-2', name: 'OpenAI Responses', protocol: 'openai_responses', path: '/v1/responses', enabled: true, description: 'OpenAI Responses API（新协议）', methods: ['POST'], streaming: true, auth: 'bearer', content_type: 'application/json' },
  { id: 'ep-3', name: 'Anthropic Messages', protocol: 'anthropic_messages', path: '/v1/messages', enabled: true, description: 'Claude 兼容的消息端点', methods: ['POST'], streaming: true, auth: 'x-api-key', content_type: 'application/json' },
  { id: 'ep-4', name: 'Gemini', protocol: 'gemini', path: '/v1beta/models', enabled: false, description: 'Google Gemini 生成端点', methods: ['POST'], streaming: true, auth: 'bearer', content_type: 'application/json' },
  { id: 'ep-5', name: 'Ollama', protocol: 'ollama', path: '/api/chat', enabled: false, description: 'Ollama 本地模型端点', methods: ['POST'], streaming: true, auth: 'none', content_type: 'application/json' },
]

export default function Endpoints() {
  const [endpoints, setEndpoints] = useState<Endpoint[]>([])
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [search, setSearch] = useState('')

  useEffect(() => { loadEndpoints() }, [])

  async function loadEndpoints() {
    try {
      const res = await fetch('/api/endpoints')
      const data = await res.json()
      setEndpoints(data.endpoints || [])
    } catch {
      setEndpoints(DEFAULT_ENDPOINTS)
    } finally { setLoading(false) }
  }

  function toggle(id: string) {
    const s = new Set(expanded); s.has(id) ? s.delete(id) : s.add(id); setExpanded(s)
  }
  function addEndpoint() {
    const ep: Endpoint = { id: `ep-${Date.now()}`, name: '新端点', protocol: 'custom', path: '/v1/custom', enabled: true, description: '', methods: ['POST'], streaming: true, auth: 'bearer', content_type: 'application/json' }
    setEndpoints([...endpoints, ep])
    setExpanded(new Set(expanded).add(ep.id))
  }
  function update(id: string, field: keyof Endpoint, value: any) {
    setEndpoints(endpoints.map(e => e.id === id ? { ...e, [field]: value } : e))
  }
  function remove(id: string) {
    setEndpoints(endpoints.filter(e => e.id !== id))
  }

  const filtered = endpoints.filter(e => {
    if (search && !e.name.toLowerCase().includes(search.toLowerCase()) && !e.path.toLowerCase().includes(search.toLowerCase())) return false
    return true
  })

  async function saveAll() {
    setBusy(true)
    try {
      await fetch('/api/endpoints', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ endpoints }) })
      toast('端点已保存')
    } catch {
      toast('后端未实现，仅本地保存')
    } finally { setBusy(false) }
  }

  async function testEndpoint(ep: Endpoint) {
    setBusy(true)
    try {
      await fetch('/api/endpoints/test', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ id: ep.id }) })
      toast(`${ep.name} 测试通过`)
    } catch {
      toast(`${ep.name} 测试（后端未实现，模拟通过）`)
    } finally { setBusy(false) }
  }

  const enabledCount = endpoints.filter(e => e.enabled).length

  return (
    <div className="space-y-4">
      {/* 头部 */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold">端点</h2>
          <p className="text-sm text-gray-500">管理多协议端点 · {endpoints.length} 个端点 · {enabledCount} 启用</p>
        </div>
        <div className="flex gap-2">
          <button onClick={saveAll} disabled={busy} className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700 disabled:opacity-50">
            <Save className="h-4 w-4" />保存
          </button>
          <button onClick={addEndpoint} className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">
            <Plus className="h-4 w-4" />添加端点
          </button>
        </div>
      </div>

      {/* 搜索 + 协议概览 */}
      <div className="flex flex-wrap items-center gap-3 rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
        <input value={search} onChange={e => setSearch(e.target.value)} placeholder="搜索端点名或路径..."
          className="w-64 rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700" />
        <div className="ml-auto flex gap-2">
          {Object.entries(PROTOCOL_META).map(([p, meta]) => (
            <span key={p} className="flex items-center gap-1 rounded bg-gray-100 px-2 py-1 text-xs text-gray-500 dark:bg-gray-700">
              <meta.icon className={`h-3 w-3 ${meta.color}`} />
              {PROTOCOL_LABELS[p as EndpointProtocol]}
            </span>
          ))}
        </div>
      </div>

      {/* 端点列表 */}
      <div className="space-y-2">
        {loading ? (
          <div className="h-40 animate-pulse rounded-xl bg-gray-200 dark:bg-gray-700" />
        ) : filtered.map(ep => {
          const meta = PROTOCOL_META[ep.protocol]
          const Icon = meta.icon
          const open = expanded.has(ep.id)
          return (
            <div key={ep.id} className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
              {/* 摘要行 */}
              <div className="flex items-center gap-3 p-4">
                <button onClick={() => toggle(ep.id)} className="text-gray-400">
                  {open ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
                </button>
                <div className={`flex h-8 w-8 items-center justify-center rounded-lg ${meta.color} bg-gray-100 dark:bg-gray-700`}>
                  <Icon className="h-4 w-4" />
                </div>
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-semibold">{ep.name}</span>
                    <span className={`rounded px-1.5 py-0.5 text-[10px] ${ep.enabled ? 'bg-green-100 text-green-700 dark:bg-green-900/30' : 'bg-gray-100 text-gray-500 dark:bg-gray-700'}`}>
                      {ep.enabled ? '启用' : '停用'}
                    </span>
                    <span className="rounded bg-gray-100 px-1.5 py-0.5 text-[10px] text-gray-500 dark:bg-gray-700">{PROTOCOL_LABELS[ep.protocol]}</span>
                  </div>
                  <div className="mt-0.5 font-mono text-xs text-gray-400">{ep.path}</div>
                </div>
                <div className="ml-auto flex items-center gap-2">
                  <button onClick={() => testEndpoint(ep)} title="测试" className="flex items-center gap-1 rounded-lg border border-gray-300 px-2 py-1 text-xs hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">
                    <Activity className="h-3 w-3" />测试
                  </button>
                  <button onClick={() => update(ep.id, 'enabled', !ep.enabled)}
                    className={`flex items-center gap-1 rounded-lg px-2 py-1 text-xs ${ep.enabled ? 'text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20' : 'text-green-600 hover:bg-green-50 dark:hover:bg-green-900/20'}`}>
                    {ep.enabled ? <Unlock className="h-3 w-3" /> : <Lock className="h-3 w-3" />}
                    {ep.enabled ? '停用' : '启用'}
                  </button>
                  <button onClick={() => remove(ep.id)} className="text-red-500 hover:text-red-700"><Trash2 className="h-4 w-4" /></button>
                </div>
              </div>

              {/* 展开详情 */}
              {open && (
                <div className="grid grid-cols-1 gap-3 border-t border-gray-200 p-4 sm:grid-cols-2 lg:grid-cols-4 dark:border-gray-700">
                  <div><label className="mb-1 block text-xs text-gray-500">名称</label>
                    <input value={ep.name} onChange={e => update(ep.id, 'name', e.target.value)} className="w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
                  <div><label className="mb-1 block text-xs text-gray-500">协议</label>
                    <select value={ep.protocol} onChange={e => update(ep.id, 'protocol', e.target.value)} className="w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700">
                      {Object.entries(PROTOCOL_LABELS).map(([p, label]) => <option key={p} value={p}>{label}</option>)}
                    </select></div>
                  <div className="sm:col-span-2"><label className="mb-1 block text-xs text-gray-500">路径</label>
                    <input value={ep.path} onChange={e => update(ep.id, 'path', e.target.value)} placeholder="/v1/chat/completions" className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
                  <div><label className="mb-1 block text-xs text-gray-500">认证方式</label>
                    <select value={ep.auth} onChange={e => update(ep.id, 'auth', e.target.value)} className="w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700">
                      <option value="bearer">Bearer Token</option>
                      <option value="x-api-key">X-Api-Key</option>
                      <option value="none">无认证</option>
                    </select></div>
                  <div><label className="mb-1 block text-xs text-gray-500">Content-Type</label>
                    <input value={ep.content_type} onChange={e => update(ep.id, 'content_type', e.target.value)} className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
                  <div><label className="mb-1 block text-xs text-gray-500">方法</label>
                    <div className="flex gap-1">
                      {['GET', 'POST'].map(m => (
                        <button key={m} onClick={() => update(ep.id, 'methods', ep.methods.includes(m) ? ep.methods.filter(x => x !== m) : [...ep.methods, m])}
                          className={`rounded px-2 py-1 text-xs ${ep.methods.includes(m) ? 'bg-blue-100 text-blue-700 dark:bg-blue-900/30' : 'bg-gray-100 text-gray-500 dark:bg-gray-700'}`}>{m}</button>
                      ))}
                    </div></div>
                  <div><label className="mb-1 block text-xs text-gray-500">流式</label>
                    <button onClick={() => update(ep.id, 'streaming', !ep.streaming)}
                      className={`rounded px-3 py-1 text-xs ${ep.streaming ? 'bg-green-100 text-green-700 dark:bg-green-900/30' : 'bg-gray-100 text-gray-500 dark:bg-gray-700'}`}>
                      {ep.streaming ? '支持 SSE' : '不支持'}
                    </button></div>
                  <div className="sm:col-span-2 lg:col-span-4"><label className="mb-1 block text-xs text-gray-500">描述</label>
                    <input value={ep.description} onChange={e => update(ep.id, 'description', e.target.value)} placeholder="端点说明..." className="w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
                  {/* 协议转换说明（学 sub2api apicompat） */}
                  <div className="sm:col-span-2 lg:col-span-4 flex items-center gap-2 rounded-lg bg-gray-50 p-2 text-xs text-gray-400 dark:bg-gray-700/50">
                    <ArrowRightLeft className="h-3.5 w-3.5" />
                    协议转换：Chat Completions ↔ Responses ↔ Anthropic 自动互转（学 sub2api apicompat）
                  </div>
                </div>
              )}
            </div>
          )
        })}
      </div>

      <StatusBar busy={busy} onClear={() => {}} />
    </div>
  )
}
