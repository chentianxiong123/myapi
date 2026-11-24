import { useEffect, useState } from 'react'
import { Plus, Trash2, ChevronDown, ChevronRight, Key, Layers, Globe, Zap, RefreshCw, Route } from 'lucide-react'
import { toast } from '@/components/Toast'
import StatusBar from '@/components/StatusBar'
import { postJSON } from '@/lib/api'
import type { Provider, ModelProtocol, ProxyPool, Channel } from '@/lib/types'
import { MODEL_PROTOCOLS, PROXY_STRATEGIES, MODEL_KINDS } from '@/lib/types'

export default function Providers() {
  const [providers, setProviders] = useState<Provider[]>([])
  const [expanded, setExpanded] = useState<Set<number>>(new Set())
  const [busy, setBusy] = useState(false)
  const [testingKeys, setTestingKeys] = useState<Set<string>>(new Set())
  const [proxyPools, setProxyPools] = useState<ProxyPool[]>([])
  const [testResults, setTestResults] = useState<Map<string, 'ok' | 'fail'>>(new Map())

  useEffect(() => {
    loadProviders()
    loadProxyPools()
  }, [])

  async function loadProxyPools() {
    try {
      const res = await fetch('/api/proxypools')
      const data = await res.json()
      setProxyPools(data.pools || [])
    } catch {
      // 示例池（学 9router proxyPools）
      setProxyPools([
        { id: 'p-1', name: '家里 Clash', proxyUrl: 'http://127.0.0.1:7890', type: 'http', isActive: true, strictProxy: false, testStatus: 'ok' },
        { id: 'p-2', name: '服务器代理', proxyUrl: 'socks5://192.168.31.12:1080', type: 'socks5', isActive: true, strictProxy: true, testStatus: 'unknown' },
      ])
    }
  }

  async function loadProviders() {
    try {
      const res = await fetch('/api/providers')
      const data = await res.json()
      setProviders(data.providers || [])
    } catch {
      // 后端还没写，用示例数据（密钥池 + 通道，学 LiteLLM Deployment）
      setProviders([
        {
          name: 'SenseNova',
          base_url: 'https://token.sensenova.cn/v1',
          disabled: false,
          headers: { 'User-Agent': 'opencode/1.18.30' },
          proxy: '',
          proxy_pool_id: '',
          proxy_strategy: 'none',
          keys: [
            { name: '主', 'api-key': 'sk-sensenova' },
            { name: '备用', 'api-key': 'sk-sensenova-backup' },
          ],
          channels: [
            {
              id: 'c-1', name: '带key-ipv4', path: 'IPv4 直连', key_name: '主', weight: 8,
              models: [
                { name: 'deepseek-v4-flash', alias: 'deepseek-v4-flash', protocol: 'openai_chat' },
                { name: 'sensenova-6.8-flash-lite', protocol: 'openai_chat', kind: 'chat' },
              ],
            },
            {
              id: 'c-2', name: '带key-ipv6', path: 'IPv6', key_name: '备用', weight: 2,
              models: [
                { name: 'deepseek-v4.1-flash', alias: 'deepseek-v4-flash', protocol: 'openai_responses' },
              ],
            },
          ],
        },
        {
          name: 'tavily',
          base_url: 'https://api.tavily.com',
          disabled: false,
          headers: {},
          proxy: '',
          proxy_pool_id: '',
          proxy_strategy: 'none',
          keys: [
            { name: '主', 'api-key': 'sk-tavily' },
          ],
          channels: [
            {
              id: 'c-3', name: '带key-ipv4', path: 'IPv4', key_name: '主', weight: 6,
              models: [{ name: 'tavily/search', alias: 'tavily/search', protocol: 'openai_chat', kind: 'webSearch' }],
            },
            {
              id: 'c-4', name: '无key-ipv6', path: 'IPv6', key_name: '', weight: 4,
              models: [{ name: 'tavily/search', alias: 'tavily/search', protocol: 'openai_chat', kind: 'webSearch' }],
            },
          ],
        },
        {
          name: 'pie-xian',
          base_url: 'https://api.pie-xian.com/v1',
          disabled: false,
          headers: {},
          proxy: '',
          proxy_pool_id: '',
          proxy_strategy: 'none',
          keys: [{ name: '主', 'api-key': 'sk-pie' }],
          channels: [
            {
              id: 'c-5', name: '主通道', key_name: '主', weight: 1,
              models: [
                { name: 'gemini-3.1-flash-lite', alias: 'gemini-3.5-flash-lite', protocol: 'gemini' },
                { name: 'hy4-flash-free', alias: 'hy4', protocol: 'ollama' },
              ],
            },
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
    setProviders([...providers, {
      name: '', base_url: '', disabled: false, headers: { 'User-Agent': 'opencode/1.18.30' },
      keys: [{ name: '主', 'api-key': '' }], channels: [], proxy: '', proxy_pool_id: '', proxy_strategy: 'none',
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

  // ---- 密钥池 ----
  function addKey(idx: number) {
    const ps = [...providers]
    const used = new Set(ps[idx].keys.map(k => k.name))
    let name = '密钥'; let n = 1
    while (used.has(name)) name = `密钥${++n}`
    ps[idx].keys.push({ name, 'api-key': '' })
    setProviders(ps)
  }
  function removeKey(pidx: number, kidx: number) {
    const ps = [...providers]
    ps[pidx].keys.splice(kidx, 1)
    setProviders(ps)
  }
  function updateKey(pidx: number, kidx: number, field: 'name' | 'api-key', value: string) {
    const ps = [...providers]
    ps[pidx].keys[kidx] = { ...ps[pidx].keys[kidx], [field]: value }
    setProviders(ps)
  }

  // ---- 通道 ----
  function addChannel(idx: number) {
    const ps = [...providers]
    const ch: Channel = { id: `c-${Date.now()}`, name: '', key_name: ps[idx].keys[0]?.name || '', weight: 1, models: [] }
    ps[idx].channels.push(ch)
    setProviders(ps)
  }
  function removeChannel(pidx: number, cidx: number) {
    const ps = [...providers]
    ps[pidx].channels.splice(cidx, 1)
    setProviders(ps)
  }
  function updateChannel(pidx: number, cidx: number, field: keyof Channel, value: any) {
    const ps = [...providers]
    ps[pidx].channels[cidx] = { ...ps[pidx].channels[cidx], [field]: value }
    setProviders(ps)
  }

  // ---- 通道内的模型映射 ----
  // 拉取上游模型列表（学 new-api FetchUpstreamModels）：GET 上游 /models → 填充模型映射（去重）
  const [fetchingIdx, setFetchingIdx] = useState<{ p: number; c: number } | null>(null)
  const [fetchMsg, setFetchMsg] = useState<string>('')
  async function fetchUpstreamModels(pidx: number, cidx: number) {
    const p = providers[pidx]
    const apiKey = p.keys.find(k => k.name === (p.channels[cidx].key_name || p.keys[0]?.name))?.['api-key'] || p.keys[0]?.['api-key'] || ''
    setFetchingIdx({ p: pidx, c: cidx })
    setFetchMsg('')
    try {
      const res = await postJSON<{ ok: boolean; models?: string[]; error?: string }>('/api/providers/fetch-models', {
        base_url: p.base_url,
        api_key: apiKey,
        headers: p.headers || {},
      })
      if (!res.ok || !res.models) {
        setFetchMsg(res.error || '拉取失败')
        return
      }
      const existing = new Set(providers[pidx].channels[cidx].models.map(m => m.name))
      const added = res.models.filter(id => !existing.has(id))
      if (added.length === 0) { setFetchMsg('无新模型'); return }
      const newModels = added.map(id => ({ name: id, alias: '', protocol: 'openai_chat' as const }))
      const next = [...providers]
      next[pidx].channels[cidx].models = [...next[pidx].channels[cidx].models, ...newModels]
      setProviders(next)
      setFetchMsg(`+${added.length}`)
    } catch (e: any) {
      setFetchMsg(e?.message || '拉取失败')
    } finally {
      setFetchingIdx(null)
    }
  }

  // 通道测试（学 new-api TestChannel）：真发一个 chat 请求验证通道，显示能否出 token
  const [testingCh, setTestingCh] = useState<string | null>(null)
  const [testResult, setTestResult] = useState<Record<string, any>>({})
  async function testChannel(pidx: number, cidx: number) {
    const ch = providers[pidx].channels[cidx]
    const key = `${pidx}-${cidx}`
    setTestingCh(key)
    try {
      const res = await postJSON<any>('/api/channels/test', { provider: providers[pidx].name, channel_id: ch.id })
      setTestResult(prev => ({ ...prev, [key]: res }))
    } catch (e: any) {
      setTestResult(prev => ({ ...prev, [key]: { ok: false, error: e?.message || '测试失败' } }))
    } finally {
      setTestingCh(null)
    }
  }

  function addChannelModel(pidx: number, cidx: number, protocol: ModelProtocol) {
    const ps = [...providers]
    ps[pidx].channels[cidx].models.push({ name: '', alias: '', protocol })
    setProviders(ps)
  }
  function removeChannelModel(pidx: number, cidx: number, midx: number) {
    const ps = [...providers]
    ps[pidx].channels[cidx].models.splice(midx, 1)
    setProviders(ps)
  }
  function updateChannelModel(pidx: number, cidx: number, midx: number, field: 'name' | 'alias' | 'protocol' | 'kind', value: string) {
    const ps = [...providers]
    const m = { ...ps[pidx].channels[cidx].models[midx] } as any
    m[field] = value
    ps[pidx].channels[cidx].models[midx] = m
    setProviders(ps)
  }

  // 测试密钥连通性（以密钥池里某条 key 为粒度）
  async function testKey(pidx: number, kidx: number) {
    const p = providers[pidx]
    const key = p.keys[kidx]?.['api-key']
    const id = `${pidx}-${kidx}`
    if (!key) { toast('请先填密钥'); return }
    setTestingKeys(s => new Set(s).add(id))
    try {
      const base = p.base_url.replace(/\/$/, '')
      const res = await fetch(`${base}/models`, { headers: { Authorization: `Bearer ${key}` } })
      setTestResults(m => new Map(m).set(id, res.ok ? 'ok' : 'fail'))
      toast(res.ok ? `${p.name} 密钥可用` : `${p.name} 密钥不可用（${res.status}）`)
    } catch {
      setTestResults(m => new Map(m).set(id, 'fail'))
      toast(`${p.name} 测试失败（网络/跨域）`)
    } finally {
      setTestingKeys(s => { const ns = new Set(s); ns.delete(id); return ns })
    }
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

  // 统计：该供应商所有通道的模型总数 / 通道数
  function channelModelCount(p: Provider) {
    return p.channels.reduce((s, c) => s + c.models.length, 0)
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div>
          <h2 className="text-lg font-semibold">Providers</h2>
          <p className="text-sm text-gray-500">
            上游供应商 · 密钥池 + 通道（一条通道 = 密钥×路径×模型映射，学 LiteLLM Deployment）
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
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
        return (
          <div key={idx} className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
            {/* 供应商头部 */}
            <div className="flex flex-wrap items-center gap-3 p-4">
              <button onClick={() => toggleExpand(idx)} className="text-gray-400">
                {open ? <ChevronDown className="h-5 w-5" /> : <ChevronRight className="h-5 w-5" />}
              </button>
              <input
                value={p.name}
                onChange={e => updateProvider(idx, 'name', e.target.value)}
                placeholder="provider-name"
                className="w-32 bg-transparent font-semibold outline-none"
              />
              <input
                value={p.base_url}
                onChange={e => updateProvider(idx, 'base_url', e.target.value)}
                placeholder="https://api.example.com/v1"
                className="min-w-0 flex-1 rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700"
              />
              <span className="shrink-0 rounded bg-gray-100 px-2 py-0.5 text-xs text-gray-500 dark:bg-gray-700">
                {p.keys.length} 密钥 · {p.channels.length} 通道 · {channelModelCount(p)} 模型
              </span>
              <button onClick={() => removeProvider(idx)} className="shrink-0 text-red-500 hover:text-red-700">
                <Trash2 className="h-4 w-4" />
              </button>
            </div>

            {open && (
              <div className="border-t border-gray-200 p-4 dark:border-gray-700">
                {/* ① 密钥池 */}
                <div className="mb-4">
                  <div className="mb-2 flex items-center gap-2">
                    <Key className="h-4 w-4 text-gray-400" />
                    <span className="text-sm font-medium">密钥池</span>
                    <span className="text-xs text-gray-400">通道引用这里的名字（第一个默认为「主」）</span>
                    <button onClick={() => addKey(idx)} className="ml-auto text-xs text-blue-600 hover:text-blue-700">+ 添加密钥</button>
                  </div>
                  <div className="space-y-1">
                    {p.keys.map((k, kidx) => (
                      <div key={kidx} className="flex flex-wrap gap-2">
                        <input
                          value={k.name}
                          onChange={e => updateKey(idx, kidx, 'name', e.target.value)}
                          placeholder="名字（主/VIP...）"
                          className="w-24 rounded border border-gray-300 px-2 py-1 text-xs dark:border-gray-600 dark:bg-gray-700"
                        />
                        <input
                          value={k['api-key']}
                          onChange={e => updateKey(idx, kidx, 'api-key', e.target.value)}
                          type="password"
                          placeholder="sk-..."
                          className="min-w-0 flex-1 rounded border border-gray-300 px-2 py-1 text-sm font-mono dark:border-gray-600 dark:bg-gray-700"
                        />
                        <button
                          onClick={() => testKey(idx, kidx)}
                          disabled={testingKeys.has(`${idx}-${kidx}`)}
                          title="测试密钥连通性"
                          className={`flex items-center gap-1 rounded border px-2 py-1 text-xs ${testResults.get(`${idx}-${kidx}`) === 'ok' ? 'border-green-200 bg-green-50 text-green-700 dark:bg-green-900/20' : testResults.get(`${idx}-${kidx}`) === 'fail' ? 'border-red-200 bg-red-50 text-red-600 dark:bg-red-900/20' : 'border-gray-300 hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700'}`}
                        >
                          {testingKeys.has(`${idx}-${kidx}`) ? <RefreshCw className="h-3 w-3 animate-spin" /> : <Zap className="h-3 w-3" />}
                          {testResults.get(`${idx}-${kidx}`) === 'ok' ? '可用' : testResults.get(`${idx}-${kidx}`) === 'fail' ? '失败' : '测试'}
                        </button>
                        <button onClick={() => removeKey(idx, kidx)} className="text-red-500 hover:text-red-700">
                          <Trash2 className="h-3 w-3" />
                        </button>
                      </div>
                    ))}
                  </div>
                </div>

                {/* ② 渠道代理 + 代理池 */}
                <div className="mb-4">
                  <div className="mb-2 flex items-center gap-2">
                    <Globe className="h-4 w-4 text-gray-400" />
                    <span className="text-sm font-medium">渠道代理</span>
                    <span className="text-xs text-gray-400">直连 / 单代理 / 代理池（通道可再覆盖路径）</span>
                  </div>
                  <div className="space-y-2">
                    <div className="flex gap-2">
                      <input
                        value={p.proxy}
                        onChange={e => updateProvider(idx, 'proxy', e.target.value)}
                        placeholder="单代理 URL（如 http://127.0.0.1:7890，留空=不用单代理）"
                        className="flex-1 rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700"
                      />
                      {p.proxy && (
                        <button onClick={() => updateProvider(idx, 'proxy', '')} className="rounded border border-gray-300 px-2 text-xs text-gray-500 hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">清除</button>
                      )}
                    </div>
                    <div className="flex gap-2">
                      <select
                        value={p.proxy_pool_id}
                        onChange={e => updateProvider(idx, 'proxy_pool_id', e.target.value)}
                        className="flex-1 rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700"
                      >
                        <option value="">不使用代理池</option>
                        {proxyPools.map(pool => (
                          <option key={pool.id} value={pool.id}>
                            {pool.name}（{pool.type} · {pool.testStatus === 'ok' ? '可用' : pool.testStatus === 'fail' ? '失效' : '未测'}）
                          </option>
                        ))}
                      </select>
                      <select
                        value={p.proxy_strategy}
                        onChange={e => updateProvider(idx, 'proxy_strategy', e.target.value)}
                        disabled={!p.proxy_pool_id}
                        className="w-44 rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700 disabled:opacity-50"
                      >
                        {PROXY_STRATEGIES.map(s => <option key={s.id} value={s.id}>{s.label}</option>)}
                      </select>
                    </div>
                  </div>
                </div>

                {/* ③ 通道列表 */}
                <div>
                  <div className="mb-2 flex items-center gap-2">
                    <Route className="h-4 w-4 text-gray-400" />
                    <span className="text-sm font-medium">通道</span>
                    <span className="text-xs text-gray-400">一条通道 = 密钥×路径×模型映射（模型按权重分流到通道）</span>
                    <button onClick={() => addChannel(idx)} className="ml-auto flex items-center gap-1 text-xs text-blue-600 hover:text-blue-700"><Plus className="h-3 w-3" />添加通道</button>
                  </div>

                  {p.channels.length === 0 && (
                    <p className="py-3 text-center text-xs text-gray-400">还没有通道，点「添加通道」</p>
                  )}

                  <div className="space-y-3">
                    {p.channels.map((ch, cidx) => (
                      <div key={ch.id} className="rounded-lg border border-gray-200 p-3 dark:border-gray-700">
                        {/* 通道头：名称/路径/密钥引用/权重 */}
                        <div className="flex flex-wrap items-center gap-2">
                          <input
                            value={ch.name}
                            onChange={e => updateChannel(idx, cidx, 'name', e.target.value)}
                            placeholder="通道名（带key-ipv4）"
                            className="w-36 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                          />
                          <input
                            value={ch.path || ''}
                            onChange={e => updateChannel(idx, cidx, 'path', e.target.value)}
                            placeholder="路径（IPv4/IPv6，空=继承）"
                            className="w-36 rounded border border-gray-300 px-2 py-1 text-xs dark:border-gray-600 dark:bg-gray-700"
                          />
                          <select
                            value={ch.key_name || ''}
                            onChange={e => updateChannel(idx, cidx, 'key_name', e.target.value)}
                            title="引用密钥池（空=用主密钥）"
                            className="w-28 rounded border border-gray-300 px-2 py-1 text-xs dark:border-gray-600 dark:bg-gray-700"
                          >
                            <option value="">主密钥（默认）</option>
                            {p.keys.map(k => <option key={k.name} value={k.name}>{k.name}</option>)}
                          </select>
                          <div className="flex items-center gap-1">
                            <span className="text-xs text-gray-400">权重</span>
                            <input
                              type="number"
                              value={ch.weight}
                              onChange={e => updateChannel(idx, cidx, 'weight', parseInt(e.target.value) || 0)}
                              className="w-16 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                            />
                          </div>
                          <div className="ml-auto flex items-center gap-2">
                            <button
                              onClick={() => testChannel(idx, cidx)}
                              disabled={testingCh === `${idx}-${cidx}`}
                              className="flex items-center gap-1 text-xs text-emerald-600 hover:text-emerald-700 disabled:opacity-50"
                              title="真发一个测试请求验证通道"
                            >
                              <Zap className="h-3 w-3" />
                              {testingCh === `${idx}-${cidx}` ? '测试中…' : '测试'}
                            </button>
                            <button onClick={() => removeChannel(idx, cidx)} className="text-red-500 hover:text-red-700">
                              <Trash2 className="h-3.5 w-3.5" />
                            </button>
                          </div>
                        </div>

                        {/* 通道测试结果 */}
                        {testResult[`${idx}-${cidx}`] && (() => {
                          const r = testResult[`${idx}-${cidx}`]
                          return (
                            <div className={`mt-2 rounded border p-2 text-xs ${r.ok ? 'border-emerald-300 bg-emerald-50 dark:border-emerald-700 dark:bg-emerald-900/20' : 'border-red-300 bg-red-50 dark:border-red-700 dark:bg-red-900/20'}`}>
                              <div className="flex items-center gap-2 font-medium">
                                {r.ok ? (
                                  <span className="text-emerald-600 dark:text-emerald-400">✓ 通道可用</span>
                                ) : (
                                  <span className="text-red-600 dark:text-red-400">✗ 通道异常</span>
                                )}
                                <span className="text-gray-400">HTTP {r.status} · {r.latency_ms}ms · 模型 {r.model}</span>
                              </div>
                              {r.ok && r.content ? (
                                <p className="mt-1 truncate text-gray-600 dark:text-gray-300" title={r.content}>回复：{r.content}</p>
                              ) : r.error ? (
                                <p className="mt-1 text-red-500">{r.error}</p>
                              ) : null}
                            </div>
                          )
                        })()}

                        {/* 通道内模型映射 */}
                        <div className="mt-2">
                          <div className="mb-1 flex items-center justify-between">
                            <span className="flex items-center gap-1 text-xs font-medium text-gray-500">
                              <Layers className="h-3 w-3" />模型映射（{ch.models.length}）
                            </span>
                            <div className="flex items-center gap-2">
                              {p.base_url && !ch.models.some(m => m.kind === 'webSearch') && (
                                <button
                                  onClick={() => fetchUpstreamModels(idx, cidx)}
                                  disabled={fetchingIdx?.p === idx && fetchingIdx?.c === cidx}
                                  className="text-xs text-emerald-600 hover:text-emerald-700 disabled:opacity-50"
                                  title="从上游 /models 拉取模型列表"
                                >
                                  {fetchingIdx?.p === idx && fetchingIdx?.c === cidx ? '拉取中…' : '↓ 拉取模型'}
                                </button>
                              )}
                              {fetchMsg && fetchingIdx?.p === idx && fetchingIdx?.c === cidx && (
                                <span className="text-xs text-gray-400">{fetchMsg}</span>
                              )}
                              <button onClick={() => addChannelModel(idx, cidx, 'openai_chat')} className="text-xs text-blue-600 hover:text-blue-700">+ 添加</button>
                            </div>
                          </div>
                          <div className="overflow-x-auto">
                            <table className="w-full min-w-[560px] text-left text-xs">
                              <thead>
                                <tr className="border-b border-gray-200 dark:border-gray-700">
                                  <th className="pb-1 font-medium text-gray-500">Name（上游真实名）</th>
                                  <th className="pb-1 font-medium text-gray-500">Alias（对外，空=用name）</th>
                                  <th className="pb-1 font-medium text-gray-500">协议</th>
                                  <th className="pb-1 font-medium text-gray-500">类型</th>
                                  <th className="pb-1 w-8"></th>
                                </tr>
                              </thead>
                              <tbody>
                                {ch.models.map((m, midx) => (
                                  <tr key={midx}>
                                    <td className="py-1 pr-2">
                                      <input
                                        value={m.name}
                                        onChange={e => updateChannelModel(idx, cidx, midx, 'name', e.target.value)}
                                        placeholder="real-model-name"
                                        className="w-full rounded border border-gray-300 px-2 py-1 font-mono dark:border-gray-600 dark:bg-gray-700"
                                      />
                                    </td>
                                    <td className="py-1 pr-2">
                                      <input
                                        value={m.alias || ''}
                                        onChange={e => updateChannelModel(idx, cidx, midx, 'alias', e.target.value)}
                                        placeholder="（空 = 用 name）"
                                        className="w-full rounded border border-gray-300 px-2 py-1 font-mono dark:border-gray-600 dark:bg-gray-700"
                                      />
                                    </td>
                                    <td className="py-1 pr-2">
                                      <select
                                        value={m.protocol || 'openai_chat'}
                                        onChange={e => updateChannelModel(idx, cidx, midx, 'protocol', e.target.value)}
                                        className="w-full rounded border border-gray-300 px-2 py-1 dark:border-gray-600 dark:bg-gray-700"
                                      >
                                        {MODEL_PROTOCOLS.map(proto => <option key={proto.id} value={proto.id}>{proto.label}</option>)}
                                      </select>
                                    </td>
                                    <td className="py-1 pr-2">
                                      <select
                                        value={m.kind || 'chat'}
                                        onChange={e => updateChannelModel(idx, cidx, midx, 'kind', e.target.value)}
                                        className="w-full rounded border border-gray-300 px-2 py-1 dark:border-gray-600 dark:bg-gray-700"
                                      >
                                        {MODEL_KINDS.map(kind => <option key={kind.id} value={kind.id}>{kind.label}</option>)}
                                      </select>
                                    </td>
                                    <td className="py-1">
                                      <button onClick={() => removeChannelModel(idx, cidx, midx)} className="text-red-500 hover:text-red-700">
                                        <Trash2 className="h-3 w-3" />
                                      </button>
                                    </td>
                                  </tr>
                                ))}
                              </tbody>
                            </table>
                          </div>
                          {ch.models.length === 0 && (
                            <p className="py-2 text-center text-xs text-gray-400">该通道还没有模型映射，点「+ 添加」</p>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
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
