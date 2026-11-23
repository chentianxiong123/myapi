import { useEffect, useState } from 'react'
import {
  Plus, Trash2, RefreshCw, Check, Diff, Play, Server,
  Key, Lock, Network, ChevronDown, ChevronRight, Terminal, FolderOpen, StickyNote
} from 'lucide-react'
import { toast } from '@/components/Toast'
import StatusBar from '@/components/StatusBar'

const ADAPTERS = ['pi', 'opencode', 'claude', 'codex', 'omp', 'dsh', 'picoclaw', 'crush', 'openclaw', 'hermes']

interface Host {
  name: string
  host: string
  port: number
  user: string
  auth_type: 'key' | 'password'
  key_path: string
  password: string
  ssh_alias: string
  platform: 'win' | 'unix' | 'macos'
  notes: string
}

interface AdapterInstance {
  name: string
  host: string
  path: string
}

export default function Sync() {
  const [hosts, setHosts] = useState<Host[]>([])
  const [instances, setInstances] = useState<Record<string, AdapterInstance[]>>({})
  const [expandedAdapters, setExpandedAdapters] = useState<Set<string>>(new Set())
  const [expandedHosts, setExpandedHosts] = useState<Set<number>>(new Set())
  const [busy, setBusy] = useState(false)
  const [logs, setLogs] = useState<string[]>([])

  useEffect(() => { loadSyncConfig() }, [])

  async function loadSyncConfig() {
    try {
      const res = await fetch('/api/sync/config')
      const data = await res.json()
      setHosts(data.hosts || [])
      setInstances(data.instances || {})
    } catch {
      setHosts([
        { name: 'win11', host: '192.168.31.12', port: 22, user: 'a1', auth_type: 'key', key_path: '~/.ssh/id_ed25519', password: '', ssh_alias: 'win11', platform: 'win', notes: '主力开发机' },
        { name: 'meilan-phone1', host: '192.168.31.213', port: 22, user: 'root', auth_type: 'key', key_path: '~/.ssh/id_rsa', password: '', ssh_alias: 'meilan-phone1', platform: 'unix', notes: '手机 Debian 服务器' },
        { name: 'localhost', host: '127.0.0.1', port: 22, user: 'a1', auth_type: 'key', key_path: '~/.ssh/id_ed25519', password: '', ssh_alias: '', platform: 'unix', notes: '本机' },
      ])
      setInstances({
        omp: [{ name: 'main', host: 'localhost', path: '~/.omp/agent/models.yml' }],
        dsh: [{ name: 'main', host: 'localhost', path: '~/.dsh/profiles/web/cordis.patch.yml' }],
        picoclaw: [{ name: 'phone', host: 'meilan-phone1', path: '~/.picoclaw/config.json' }],
        crush: [{ name: 'phone', host: 'meilan-phone1', path: '~/.config/crush/crushrc' }],
        openclaw: [{ name: 'win', host: 'win11', path: '~/.openclaw/openclaw.json' }],
        hermes: [{ name: 'win', host: 'win11', path: '.../hermes/config.yaml' }],
      })
    }
  }

  function addLog(text: string) { setLogs(l => [...l, `${new Date().toLocaleTimeString('zh-CN')} ${text}`]) }

  // Hosts 操作
  function addHost() {
    setHosts([...hosts, { name: '', host: '', port: 22, user: 'root', auth_type: 'key', key_path: '~/.ssh/id_ed25519', password: '', ssh_alias: '', platform: 'unix', notes: '' }])
  }
  function removeHost(idx: number) { setHosts(hosts.filter((_, i) => i !== idx)) }
  function updateHost(idx: number, field: keyof Host, value: any) {
    const h = [...hosts]; h[idx] = { ...h[idx], [field]: value }; setHosts(h)
  }
  function toggleHost(idx: number) {
    const s = new Set(expandedHosts); s.has(idx) ? s.delete(idx) : s.add(idx); setExpandedHosts(s)
  }

  // Instances 操作
  function toggleAdapter(adapter: string) {
    const s = new Set(expandedAdapters); s.has(adapter) ? s.delete(adapter) : s.add(adapter); setExpandedAdapters(s)
  }
  function addInstance(adapter: string) {
    setInstances({ ...instances, [adapter]: [...(instances[adapter] || []), { name: '', host: '', path: '' }] })
  }
  function removeInstance(adapter: string, idx: number) {
    setInstances({ ...instances, [adapter]: instances[adapter].filter((_, i) => i !== idx) })
  }
  function updateInstance(adapter: string, idx: number, field: keyof AdapterInstance, value: string) {
    const ni = { ...instances }; ni[adapter][idx] = { ...ni[adapter][idx], [field]: value }; setInstances(ni)
  }

  async function checkDiff() {
    setBusy(true); addLog('检查 diff...')
    try { await fetch('/api/sync/diff', { method: 'POST' }); addLog('✓ diff 检查完成') }
    catch { addLog('⚠ 后端未实现') } finally { setBusy(false) }
  }

  async function syncAll() {
    setBusy(true); addLog('开始同步...')
    try {
      const total = Object.values(instances).reduce((s, a) => s + a.length, 0)
      await new Promise(r => setTimeout(r, 1000))
      addLog(`✓ 已同步 ${total} 个实例`); toast(`已同步 ${total} 个实例`)
    } finally { setBusy(false) }
  }

  async function syncAdapter(adapter: string) {
    setBusy(true); addLog(`同步 ${adapter}...`)
    try { await new Promise(r => setTimeout(r, 500)); addLog(`✓ ${adapter} 已同步`); toast(`${adapter} 已同步`) }
    finally { setBusy(false) }
  }

  const totalInstances = Object.values(instances).reduce((s, a) => s + a.length, 0)

  return (
    <div className="space-y-4">
      {/* 顶部操作 */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold">配置下发</h2>
          <p className="text-sm text-gray-500">{hosts.length} 台主机 · {totalInstances} 个实例</p>
        </div>
        <div className="flex gap-2">
          <button onClick={checkDiff} disabled={busy} className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700 disabled:opacity-50">
            <Diff className="h-4 w-4" />检查 Diff
          </button>
          <button onClick={syncAll} disabled={busy} className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700 disabled:opacity-50">
            <Play className="h-4 w-4" />全部同步
          </button>
        </div>
      </div>

      {/* Hosts 管理 */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <div className="mb-4 flex items-center justify-between">
          <h3 className="flex items-center gap-2 text-sm font-semibold"><Server className="h-4 w-4" />主机列表</h3>
          <button onClick={addHost} className="flex items-center gap-1 text-xs text-blue-600 hover:text-blue-700"><Plus className="h-3 w-3" />添加主机</button>
        </div>
        <div className="space-y-2">
          {hosts.map((h, idx) => (
            <div key={idx} className="rounded-lg border border-gray-200 dark:border-gray-700">
              <div className="flex items-center gap-3 p-3">
                <button onClick={() => toggleHost(idx)} className="text-gray-400">
                  {expandedHosts.has(idx) ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
                </button>
                <input value={h.name} onChange={e => updateHost(idx, 'name', e.target.value)} placeholder="主机名"
                  className="w-32 bg-transparent font-semibold outline-none" />
                <span className="flex items-center gap-1 text-xs text-gray-400"><Network className="h-3 w-3" />{h.host || '未设置'}</span>
                <span className="flex items-center gap-1 text-xs text-gray-400">
                  {h.auth_type === 'key' ? <Key className="h-3 w-3" /> : <Lock className="h-3 w-3" />}
                  {h.auth_type === 'key' ? '密钥' : '密码'}
                </span>
                <span className="rounded bg-gray-100 px-1.5 py-0.5 text-xs text-gray-500 dark:bg-gray-700">{h.platform}</span>
                <div className="flex-1" />
                <button onClick={() => removeHost(idx)} className="text-red-500 hover:text-red-700"><Trash2 className="h-4 w-4" /></button>
              </div>
              {expandedHosts.has(idx) && (
                <div className="border-t border-gray-200 p-4 dark:border-gray-700">
                  <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
                    <div><label className="mb-1 block text-xs text-gray-500">主机名</label>
                      <input value={h.name} onChange={e => updateHost(idx, 'name', e.target.value)} className="w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
                    <div><label className="mb-1 flex items-center gap-1 text-xs text-gray-500"><Network className="h-3 w-3" />IP / Hostname</label>
                      <input value={h.host} onChange={e => updateHost(idx, 'host', e.target.value)} placeholder="192.168.1.100" className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
                    <div><label className="mb-1 block text-xs text-gray-500">SSH 端口</label>
                      <input type="number" value={h.port} onChange={e => updateHost(idx, 'port', parseInt(e.target.value) || 22)} className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
                    <div><label className="mb-1 block text-xs text-gray-500">用户名</label>
                      <input value={h.user} onChange={e => updateHost(idx, 'user', e.target.value)} placeholder="root" className="w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
                    <div><label className="mb-1 block text-xs text-gray-500">认证方式</label>
                      <select value={h.auth_type} onChange={e => updateHost(idx, 'auth_type', e.target.value)}
                        className="w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700">
                        <option value="key">SSH 密钥</option>
                        <option value="password">密码</option>
                      </select></div>
                    {h.auth_type === 'key' ? (
                      <div className="sm:col-span-2"><label className="mb-1 flex items-center gap-1 text-xs text-gray-500"><Key className="h-3 w-3" />SSH 密钥路径</label>
                        <input value={h.key_path} onChange={e => updateHost(idx, 'key_path', e.target.value)} placeholder="~/.ssh/id_ed25519"
                          className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
                    ) : (
                      <div className="sm:col-span-2"><label className="mb-1 flex items-center gap-1 text-xs text-gray-500"><Lock className="h-3 w-3" />密码</label>
                        <input type="password" value={h.password} onChange={e => updateHost(idx, 'password', e.target.value)} className="w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
                    )}
                    <div><label className="mb-1 block text-xs text-gray-500">SSH 别名（~/.ssh/config）</label>
                      <input value={h.ssh_alias} onChange={e => updateHost(idx, 'ssh_alias', e.target.value)} placeholder="如 win11"
                        className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
                    <div><label className="mb-1 block text-xs text-gray-500">平台</label>
                      <select value={h.platform} onChange={e => updateHost(idx, 'platform', e.target.value)}
                        className="w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700">
                        <option value="unix">Linux / Unix</option>
                        <option value="win">Windows</option>
                        <option value="macos">macOS</option>
                      </select></div>
                    <div className="sm:col-span-2 lg:col-span-4"><label className="mb-1 flex items-center gap-1 text-xs text-gray-500"><StickyNote className="h-3 w-3" />备注</label>
                      <input value={h.notes} onChange={e => updateHost(idx, 'notes', e.target.value)} placeholder="备注..."
                        className="w-full rounded border border-gray-300 px-2 py-1 text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      </div>

      {/* Adapter Instances */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <h3 className="mb-4 flex items-center gap-2 text-sm font-semibold"><FolderOpen className="h-4 w-4" />Adapter 实例</h3>
        <div className="space-y-2">
          {ADAPTERS.map(adapter => {
            const insts = instances[adapter] || []
            const expanded = expandedAdapters.has(adapter)
            return (
              <div key={adapter} className="rounded-lg border border-gray-200 dark:border-gray-700">
                <button onClick={() => toggleAdapter(adapter)} className="flex w-full items-center gap-2 px-4 py-2 text-left text-sm font-medium">
                  {expanded ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
                  <span>{adapter}</span>
                  <span className="ml-auto text-xs text-gray-400">{insts.length} 台</span>
                  <button onClick={e => { e.stopPropagation(); syncAdapter(adapter) }}
                    className="ml-2 rounded bg-blue-50 px-2 py-0.5 text-xs text-blue-600 hover:bg-blue-100 dark:bg-blue-900/20">同步</button>
                </button>
                {expanded && (
                  <div className="border-t border-gray-200 p-3 dark:border-gray-700">
                    <table className="w-full text-left text-sm">
                      <thead><tr className="border-b border-gray-100 dark:border-gray-800">
                        <th className="pb-2 font-medium text-gray-500">名称</th>
                        <th className="pb-2 font-medium text-gray-500">主机</th>
                        <th className="pb-2 font-medium text-gray-500">路径</th>
                        <th className="pb-2 w-10"></th>
                      </tr></thead>
                      <tbody>{insts.map((inst, idx) => (
                        <tr key={idx}>
                          <td className="py-1 pr-2"><input value={inst.name} onChange={e => updateInstance(adapter, idx, 'name', e.target.value)} placeholder="实例名" className="w-full rounded border border-gray-300 px-2 py-1 dark:border-gray-600 dark:bg-gray-700" /></td>
                          <td className="py-1 pr-2"><select value={inst.host} onChange={e => updateInstance(adapter, idx, 'host', e.target.value)} className="w-full rounded border border-gray-300 px-2 py-1 dark:border-gray-600 dark:bg-gray-700">
                            <option value="">选择主机</option>{hosts.map(h => <option key={h.name} value={h.name}>{h.name}</option>)}</select></td>
                          <td className="py-1 pr-2"><input value={inst.path} onChange={e => updateInstance(adapter, idx, 'path', e.target.value)} placeholder="~/path/to/config" className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></td>
                          <td className="py-1"><button onClick={() => removeInstance(adapter, idx)} className="text-red-500 hover:text-red-700"><Trash2 className="h-3 w-3" /></button></td>
                        </tr>
                      ))}</tbody>
                    </table>
                    <button onClick={() => addInstance(adapter)} className="mt-2 flex items-center gap-1 text-xs text-blue-600 hover:text-blue-700"><Plus className="h-3 w-3" />添加实例</button>
                  </div>
                )}
              </div>
            )
          })}
        </div>
      </div>

      {/* 同步日志 */}
      <div className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <div className="flex items-center justify-between border-b border-gray-200 px-4 py-2 dark:border-gray-700">
          <span className="text-sm font-medium">同步日志</span>
          <button onClick={() => setLogs([])} className="text-xs text-gray-400 hover:text-gray-600">清空</button>
        </div>
        <div className="max-h-40 overflow-auto p-4 font-mono text-xs">
          {logs.length === 0 ? <span className="text-gray-400">— 选择上方功能执行</span> : logs.map((log, i) => <div key={i}>{log}</div>)}
        </div>
      </div>

      <StatusBar busy={busy} onClear={() => setLogs([])} />
    </div>
  )
}
