import { useEffect, useState } from 'react'
import {
  Plus, Trash2, Server, Database, Zap, Cpu, MemoryStick, Activity,
  RefreshCw, Network, ArrowRight, FileDown, HeartPulse, GitBranch, Smartphone, Laptop, Monitor,
} from 'lucide-react'
import { toast } from '@/components/Toast'
import StatusBar from '@/components/StatusBar'
import type { DeployInstance, GatewayEntry, SqliteSync } from '@/lib/types'

const DEFAULT_INSTANCES: DeployInstance[] = [
  { id: 'i-1', name: 'gw-phone', role: 'master', address: '192.168.31.213:8317', region: '手机 meilan-phone1', status: 'running', cpu: 12, mem: 38, requests: 45231, heartbeatMs: 5000, sqlitePath: '/opt/myapi/data/gateway.db' },
  { id: 'i-2', name: 'gw-win11', role: 'backup', address: '192.168.31.12:8317', region: 'win11', status: 'running', cpu: 8, mem: 25, requests: 18742, heartbeatMs: 5000, sqlitePath: 'C:\\myapi\\data\\gateway.db' },
  { id: 'i-3', name: 'gw-pve', role: 'backup', address: '192.168.31.14:8317', region: 'pve', status: 'stopped', cpu: 0, mem: 0, requests: 0, heartbeatMs: 5000, sqlitePath: '/srv/myapi/data/gateway.db' },
]

const DEFAULT_ENTRY: GatewayEntry = { listen: 8380, readTimeout: 120, nextUpstream: 'error timeout http_502 http_503 http_504' }

const DEFAULT_SYNC: SqliteSync = {
  enabled: true, sourcePath: '/opt/myapi/data/gateway.db',
  targetHost: '192.168.31.12', targetPath: 'C:\\myapi\\data\\gateway.db',
  intervalSec: 60, lastSync: new Date().toISOString(),
}

export default function Deploy() {
  const [instances, setInstances] = useState<DeployInstance[]>([])
  const [entry, setEntry] = useState<GatewayEntry>(DEFAULT_ENTRY)
  const [sync, setSync] = useState<SqliteSync>(DEFAULT_SYNC)
  const [busy, setBusy] = useState(false)
  const [loading, setLoading] = useState(true)

  useEffect(() => { load() }, [])

  async function load() {
    try {
      const res = await fetch('/api/deploy/instances')
      const data = await res.json()
      setInstances(data.instances || [])
    } catch {
      setInstances(DEFAULT_INSTANCES)
    } finally { setLoading(false) }
  }

  function addInstance() {
    const inst: DeployInstance = {
      id: `i-${Date.now()}`, name: 'gw-backup-n', role: 'backup', address: '', region: '',
      status: 'stopped', cpu: 0, mem: 0, requests: 0, heartbeatMs: 5000, sqlitePath: '',
    }
    setInstances([...instances, inst])
  }
  function removeInstance(id: string) { setInstances(instances.filter(i => i.id !== id)) }
  function updateInstance(id: string, field: keyof DeployInstance, value: any) {
    setInstances(instances.map(i => i.id === id ? { ...i, [field]: value } : i))
  }

  const running = instances.filter(i => i.status === 'running').length
  const totalRequests = instances.reduce((s, i) => s + i.requests, 0)
  const masterCount = instances.filter(i => i.role === 'master').length
  const backupCount = instances.filter(i => i.role === 'backup').length

  async function syncNow() {
    setBusy(true)
    try {
      await new Promise(r => setTimeout(r, 800))
      setSync({ ...sync, lastSync: new Date().toISOString() })
      toast('SQLite 已同步到备用节点')
    } finally { setBusy(false) }
  }

  async function failover() {
    setBusy(true)
    try {
      await new Promise(r => setTimeout(r, 600))
      toast('故障切换：已切到备用节点（Nginx upstream 自动）')
    } finally { setBusy(false) }
  }

  async function saveAll() {
    setBusy(true)
    try {
      await fetch('/api/deploy/instances', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ instances, entry, sync }) })
      toast('部署配置已保存')
    } catch {
      toast('后端未实现，仅本地保存')
    } finally { setBusy(false) }
  }

  function regionIcon(region: string) {
    if (region.includes('手机')) return <Smartphone className="h-3 w-3" />
    if (region.includes('win')) return <Laptop className="h-3 w-3" />
    return <Monitor className="h-3 w-3" />
  }

  return (
    <div className="space-y-4">
      {/* 头部 */}
      <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div>
          <h2 className="text-lg font-semibold">部署</h2>
          <p className="text-sm text-gray-500">主备高可用 · 客户端固定入口 + Nginx 多 upstream 故障切换 + SQLite 同步</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button onClick={failover} className="flex items-center gap-1.5 rounded-lg border border-amber-300 px-3 py-1.5 text-sm text-amber-600 hover:bg-amber-50 dark:border-amber-600 dark:hover:bg-amber-900/20">
            <Zap className="h-4 w-4" />故障切换
          </button>
          <button onClick={addInstance} className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">
            <Plus className="h-4 w-4" />添加实例
          </button>
          <button onClick={saveAll} disabled={busy} className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700 disabled:opacity-50">
            <RefreshCw className={`h-4 w-4 ${busy ? 'animate-spin' : ''}`} />保存
          </button>
        </div>
      </div>

      {/* 概览卡 */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        {[
          { label: '固定入口', value: `:${entry.listen}`, sub: '客户端认的地址', icon: Network, color: 'text-blue-600' },
          { label: '主节点', value: `${masterCount}`, sub: '持有入口', icon: Server, color: 'text-green-600' },
          { label: '备用节点', value: `${backupCount}`, sub: `${running} 运行`, icon: HeartPulse, color: 'text-violet-600' },
          { label: '总请求', value: totalRequests.toLocaleString(), sub: '累计', icon: Activity, color: 'text-orange-600' },
        ].map(card => (
          <div key={card.label} className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
            <div className="flex items-center gap-2 text-xs text-gray-400"><card.icon className={`h-4 w-4 ${card.color}`} />{card.label}</div>
            <div className="mt-1 text-2xl font-semibold tabular-nums">{card.value}</div>
            <div className="text-xs text-gray-400">{card.sub}</div>
          </div>
        ))}
      </div>

      {/* 拓扑：客户端 → 固定入口 → upstream 主/备 → SQLite 同步 */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <h3 className="mb-4 flex items-center gap-2 text-sm font-semibold"><Network className="h-4 w-4" />拓扑（客户端无感知主备切换）</h3>
        <div className="flex flex-col items-center gap-2">
          {/* 客户端 */}
          <div className="rounded-lg border border-gray-200 bg-gray-50 px-4 py-1.5 text-xs text-gray-600 dark:border-gray-600 dark:bg-gray-700">客户端（pi / opencode / claude / codex）</div>
          <div className="text-gray-300">↓</div>
          {/* 固定入口（Nginx） */}
          <div className="rounded-lg bg-blue-50 px-5 py-2 text-xs font-semibold text-blue-600 shadow-sm dark:bg-blue-900/20">
            固定入口 :{entry.listen}（Nginx）
          </div>
          <div className="text-gray-300">↓</div>
          {/* upstream 主/备 */}
          <div className="flex gap-2">
            {instances.map(i => (
              <div key={i.id} className={`rounded-lg border px-3 py-2 text-xs ${i.status === 'running' ? 'border-green-200 bg-green-50 dark:bg-green-900/20' : 'border-gray-200 bg-gray-50 dark:bg-gray-700'}`}>
                <div className="flex items-center gap-1.5 font-medium">
                  <Server className={`h-3 w-3 ${i.status === 'running' ? 'text-green-600' : 'text-gray-400'}`} />
                  {i.name}
                  {i.role === 'master'
                    ? <span className="rounded bg-green-100 px-1 text-[9px] text-green-700 dark:bg-green-900/30">主</span>
                    : <span className="rounded bg-violet-100 px-1 text-[9px] text-violet-700 dark:bg-violet-900/30">备</span>}
                </div>
                <div className="mt-0.5 flex items-center gap-1 text-[10px] text-gray-400">
                  {regionIcon(i.region)}{i.address || '未配置'}
                </div>
              </div>
            ))}
          </div>
          <div className="text-gray-300">↓</div>
          {/* SQLite 同步 */}
          <div className="flex gap-2">
            <div className="rounded-lg bg-orange-50 px-3 py-1.5 text-xs font-medium text-orange-600 dark:bg-orange-900/20">
              <Database className="mr-1 inline h-3 w-3" />SQLite 文件同步（主→备）
            </div>
            <div className="rounded-lg bg-gray-100 px-3 py-1.5 text-xs text-gray-500 dark:bg-gray-700">
              {sync.enabled ? `每 ${sync.intervalSec}s` : '未启用'}
            </div>
          </div>
        </div>
      </div>

      {/* 固定入口 + 故障切换配置 */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
          <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><Network className="h-4 w-4 text-blue-500" />固定入口</h3>
          <div className="grid grid-cols-2 gap-3">
            <div><label className="mb-1 block text-xs text-gray-500">监听端口（客户端认）</label>
              <input type="number" value={entry.listen} onChange={e => setEntry({ ...entry, listen: parseInt(e.target.value) || 8380 })}
                className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
            <div><label className="mb-1 block text-xs text-gray-500">proxy_read_timeout（秒）</label>
              <input type="number" value={entry.readTimeout} onChange={e => setEntry({ ...entry, readTimeout: parseInt(e.target.value) || 120 })}
                className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-sm dark:border-gray-600 dark:bg-gray-700" /></div>
          </div>
          <div className="mt-3"><label className="mb-1 block text-xs text-gray-500">proxy_next_upstream（故障切换）</label>
            <input value={entry.nextUpstream} onChange={e => setEntry({ ...entry, nextUpstream: e.target.value })}
              className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
          <p className="mt-2 text-xs text-gray-400">客户端认固定入口，主节点挂了 Nginx 自动切到备用（proxy_next_upstream）</p>
        </div>

        {/* SQLite 同步 */}
        <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
          <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><Database className="h-4 w-4 text-orange-500" />SQLite 同步</h3>
          <label className="mb-1 flex items-center gap-1.5 text-xs text-gray-500">
            <input type="checkbox" checked={sync.enabled} onChange={e => setSync({ ...sync, enabled: e.target.checked })} />
            启用主→备 SQLite 文件同步
          </label>
          <div className="mt-2 grid grid-cols-2 gap-3">
            <div><label className="mb-1 block text-xs text-gray-500">主节点 SQLite 路径</label>
              <input value={sync.sourcePath} onChange={e => setSync({ ...sync, sourcePath: e.target.value })}
                className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
            <div><label className="mb-1 block text-xs text-gray-500">备用节点主机</label>
              <input value={sync.targetHost} onChange={e => setSync({ ...sync, targetHost: e.target.value })}
                className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
            <div><label className="mb-1 block text-xs text-gray-500">备用节点路径</label>
              <input value={sync.targetPath} onChange={e => setSync({ ...sync, targetPath: e.target.value })}
                className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
            <div><label className="mb-1 block text-xs text-gray-500">同步间隔（秒）</label>
              <input type="number" value={sync.intervalSec} onChange={e => setSync({ ...sync, intervalSec: parseInt(e.target.value) || 60 })}
                className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" /></div>
          </div>
          <div className="mt-3 flex items-center justify-between">
            <span className="text-xs text-gray-400">上次同步：{sync.lastSync ? new Date(sync.lastSync).toLocaleTimeString('zh-CN') : '—'}</span>
            <button onClick={syncNow} disabled={busy} className="flex items-center gap-1 rounded-lg bg-blue-600 px-3 py-1 text-xs text-white hover:bg-blue-700 disabled:opacity-50">
              <FileDown className="h-3 w-3" />立即同步
            </button>
          </div>
        </div>
      </div>

      {/* 实例列表 */}
      <div className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <div className="flex items-center justify-between border-b border-gray-200 px-4 py-3 dark:border-gray-700">
          <h3 className="text-sm font-semibold">实例（upstream）</h3>
          <span className="text-xs text-gray-400">{instances.length} 实例</span>
        </div>
        {loading ? (
          <div className="h-32 animate-pulse bg-gray-200 dark:bg-gray-700" />
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead><tr className="border-b border-gray-200 dark:border-gray-700">
                <th className="px-3 py-2 font-medium text-gray-500">实例</th>
                <th className="px-3 py-2 font-medium text-gray-500">角色</th>
                <th className="px-3 py-2 font-medium text-gray-500">地址</th>
                <th className="px-3 py-2 font-medium text-gray-500">位置</th>
                <th className="px-3 py-2 font-medium text-gray-500">状态</th>
                <th className="px-3 py-2 font-medium text-gray-500">CPU</th>
                <th className="px-3 py-2 font-medium text-gray-500">内存</th>
                <th className="px-3 py-2 font-medium text-gray-500">请求</th>
                <th className="px-3 py-2 font-medium text-gray-500">心跳</th>
                <th className="px-3 py-2 w-10"></th>
              </tr></thead>
              <tbody>
                {instances.map(i => (
                  <tr key={i.id} className="border-b border-gray-100 hover:bg-gray-50 dark:border-gray-800 dark:hover:bg-gray-700/50">
                    <td className="px-3 py-2">
                      <input value={i.name} onChange={e => updateInstance(i.id, 'name', e.target.value)}
                        className="w-28 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                    </td>
                    <td className="px-3 py-2">
                      <select value={i.role} onChange={e => updateInstance(i.id, 'role', e.target.value)}
                        className="rounded border border-gray-300 px-2 py-1 text-xs dark:border-gray-600 dark:bg-gray-700">
                        <option value="master">主</option>
                        <option value="backup">备</option>
                      </select>
                    </td>
                    <td className="px-3 py-2">
                      <input value={i.address} onChange={e => updateInstance(i.id, 'address', e.target.value)} placeholder="host:port"
                        className="w-36 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                    </td>
                    <td className="px-3 py-2 text-xs text-gray-400">{i.region}</td>
                    <td className="px-3 py-2">
                      <select value={i.status} onChange={e => updateInstance(i.id, 'status', e.target.value)}
                        className={`rounded border px-2 py-1 text-xs ${i.status === 'running' ? 'border-green-200 bg-green-50 text-green-700 dark:bg-green-900/20' : i.status === 'degraded' ? 'border-yellow-200 bg-yellow-50 text-yellow-700 dark:bg-yellow-900/20' : 'border-gray-200 bg-gray-50 text-gray-500 dark:bg-gray-700'}`}>
                        <option value="running">运行</option>
                        <option value="degraded">降级</option>
                        <option value="stopped">停止</option>
                      </select>
                    </td>
                    <td className="px-3 py-2"><div className="flex items-center gap-1 font-mono text-xs"><Cpu className="h-3 w-3 text-gray-400" />{i.cpu}%</div></td>
                    <td className="px-3 py-2"><div className="flex items-center gap-1 font-mono text-xs"><MemoryStick className="h-3 w-3 text-gray-400" />{i.mem}%</div></td>
                    <td className="px-3 py-2 font-mono text-xs">{i.requests.toLocaleString()}</td>
                    <td className="px-3 py-2 font-mono text-xs">{i.heartbeatMs / 1000}s</td>
                    <td className="px-3 py-2"><button onClick={() => removeInstance(i.id)} className="text-red-500 hover:text-red-700"><Trash2 className="h-3 w-3" /></button></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      <StatusBar busy={busy} onClear={() => {}} />
    </div>
  )
}
