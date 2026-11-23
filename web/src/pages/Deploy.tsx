import { useEffect, useState } from 'react'
import {
  Plus, Trash2, Server, Database, Zap, Cpu, MemoryStick, Activity,
  RefreshCw, Scale, Network, ArrowUp, ArrowDown,
} from 'lucide-react'
import { toast } from '@/components/Toast'
import StatusBar from '@/components/StatusBar'
import type { DeployInstance, SharedStorage } from '@/lib/types'

const DEFAULT_INSTANCES: DeployInstance[] = [
  { id: 'i-1', name: 'node-master-1', nodeType: 'master', port: 3000, status: 'running', address: '192.168.31.213:3000', region: 'home', cpu: 12, mem: 38, requests: 45231, syncFrequency: 600, frontendBaseUrl: 'http://192.168.31.213:3000', redisConn: 'redis://192.168.31.213:6379', sqlDsn: 'oneapi:123456@tcp(db:3306)/one-api' },
  { id: 'i-2', name: 'node-slave-1', nodeType: 'slave', port: 3001, status: 'running', address: '192.168.31.12:3001', region: 'win11', cpu: 8, mem: 25, requests: 18742, syncFrequency: 60, frontendBaseUrl: 'http://192.168.31.12:3001', redisConn: 'redis://192.168.31.213:6379', sqlDsn: '' },
  { id: 'i-3', name: 'node-slave-2', nodeType: 'slave', port: 3002, status: 'stopped', address: '192.168.31.14:3002', region: 'pve', cpu: 0, mem: 0, requests: 0, syncFrequency: 60, frontendBaseUrl: 'http://192.168.31.14:3002', redisConn: 'redis://192.168.31.213:6379', sqlDsn: '' },
]

const DEFAULT_STORAGE: SharedStorage = {
  redis: { enabled: true, conn: 'redis://192.168.31.213:6379', cluster: false },
  db: { enabled: true, dsn: 'oneapi:123456@tcp(db:3306)/one-api', type: 'mysql' },
  syncFrequency: 600,
}

export default function Deploy() {
  const [instances, setInstances] = useState<DeployInstance[]>([])
  const [storage, setStorage] = useState<SharedStorage>(DEFAULT_STORAGE)
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
      id: `i-${Date.now()}`, name: 'node-slave-n', nodeType: 'slave', port: 3000 + instances.length,
      status: 'stopped', address: '', region: '', cpu: 0, mem: 0, requests: 0,
      syncFrequency: 60, frontendBaseUrl: '', redisConn: storage.redis.conn, sqlDsn: '',
    }
    setInstances([...instances, inst])
  }
  function removeInstance(id: string) { setInstances(instances.filter(i => i.id !== id)) }
  function updateInstance(id: string, field: keyof DeployInstance, value: any) {
    setInstances(instances.map(i => i.id === id ? { ...i, [field]: value } : i))
  }

  const running = instances.filter(i => i.status === 'running').length
  const totalRequests = instances.reduce((s, i) => s + i.requests, 0)
  const masterCount = instances.filter(i => i.nodeType === 'master').length
  const slaveCount = instances.filter(i => i.nodeType === 'slave').length

  async function scaleUp() { toast('模拟扩容：+1 从节点') }
  async function scaleDown() { toast('模拟缩容：-1 从节点') }

  async function saveAll() {
    setBusy(true)
    try {
      await fetch('/api/deploy/instances', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ instances, storage }) })
      toast('部署配置已保存')
    } catch {
      toast('后端未实现，仅本地保存')
    } finally { setBusy(false) }
  }

  return (
    <div className="space-y-4">
      {/* 头部 */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold">部署</h2>
          <p className="text-sm text-gray-500">水平扩展 · 主从节点 + 共享 Redis/DB（学 new-api NODE_TYPE=slave）</p>
        </div>
        <div className="flex gap-2">
          <button onClick={scaleDown} className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">
            <ArrowDown className="h-4 w-4" />缩容
          </button>
          <button onClick={scaleUp} className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700">
            <ArrowUp className="h-4 w-4" />扩容
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
          { label: '实例数', value: `${instances.length}`, sub: `${running} 运行`, icon: Server, color: 'text-blue-600' },
          { label: '主节点', value: `${masterCount}`, sub: '写', icon: Zap, color: 'text-green-600' },
          { label: '从节点', value: `${slaveCount}`, sub: '读/转发', icon: Network, color: 'text-violet-600' },
          { label: '总请求', value: totalRequests.toLocaleString(), sub: '累计', icon: Activity, color: 'text-orange-600' },
        ].map(card => (
          <div key={card.label} className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800">
            <div className="flex items-center gap-2 text-xs text-gray-400"><card.icon className={`h-4 w-4 ${card.color}`} />{card.label}</div>
            <div className="mt-1 text-2xl font-semibold tabular-nums">{card.value}</div>
            <div className="text-xs text-gray-400">{card.sub}</div>
          </div>
        ))}
      </div>

      {/* 拓扑示意（学 new-api 主从 + Redis + DB） */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><Network className="h-4 w-4" />拓扑（水平扩展）</h3>
        <div className="flex flex-col items-center gap-2">
          {/* 客户端 */}
          <div className="rounded-lg bg-gray-100 px-4 py-1.5 text-xs text-gray-500 dark:bg-gray-700">客户端</div>
          <div className="text-gray-300">↓</div>
          {/* 负载均衡 */}
          <div className="rounded-lg bg-blue-50 px-4 py-1.5 text-xs font-medium text-blue-600 dark:bg-blue-900/20">负载均衡（Nginx/K8s）</div>
          <div className="text-gray-300">↓</div>
          {/* 实例 */}
          <div className="flex gap-2">
            {instances.map(i => (
              <div key={i.id} className={`rounded-lg border px-3 py-1.5 text-xs ${i.status === 'running' ? 'border-green-200 bg-green-50 dark:bg-green-900/20' : 'border-gray-200 bg-gray-50 dark:bg-gray-700'}`}>
                <div className="flex items-center gap-1 font-medium">
                  <Server className={`h-3 w-3 ${i.status === 'running' ? 'text-green-600' : 'text-gray-400'}`} />
                  {i.name}
                  {i.nodeType === 'master' ? <span className="rounded bg-green-100 px-1 text-[9px] text-green-700 dark:bg-green-900/30">主</span> : <span className="rounded bg-violet-100 px-1 text-[9px] text-violet-700 dark:bg-violet-900/30">从</span>}
                </div>
                <div className="mt-0.5 text-[10px] text-gray-400">{i.port}</div>
              </div>
            ))}
          </div>
          <div className="text-gray-300">↓</div>
          {/* 共享存储 */}
          <div className="flex gap-2">
            <div className="rounded-lg bg-orange-50 px-3 py-1.5 text-xs font-medium text-orange-600 dark:bg-orange-900/20">
              <Database className="mr-1 inline h-3 w-3" />共享 DB ({storage.db.type})
            </div>
            <div className="rounded-lg bg-red-50 px-3 py-1.5 text-xs font-medium text-red-600 dark:bg-red-900/20">
              <Zap className="mr-1 inline h-3 w-3" />共享 Redis
            </div>
          </div>
        </div>
      </div>

      {/* 实例列表 */}
      <div className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <div className="flex items-center justify-between border-b border-gray-200 px-4 py-3 dark:border-gray-700">
          <h3 className="text-sm font-semibold">实例列表</h3>
          <span className="text-xs text-gray-400">{instances.length} 实例</span>
        </div>
        {loading ? (
          <div className="h-32 animate-pulse bg-gray-200 dark:bg-gray-700" />
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead><tr className="border-b border-gray-200 dark:border-gray-700">
                <th className="px-3 py-2 font-medium text-gray-500">实例</th>
                <th className="px-3 py-2 font-medium text-gray-500">节点类型</th>
                <th className="px-3 py-2 font-medium text-gray-500">地址</th>
                <th className="px-3 py-2 font-medium text-gray-500">状态</th>
                <th className="px-3 py-2 font-medium text-gray-500">CPU</th>
                <th className="px-3 py-2 font-medium text-gray-500">内存</th>
                <th className="px-3 py-2 font-medium text-gray-500">请求</th>
                <th className="px-3 py-2 font-medium text-gray-500">SYNC</th>
                <th className="px-3 py-2 w-10"></th>
              </tr></thead>
              <tbody>
                {instances.map(i => (
                  <tr key={i.id} className="border-b border-gray-100 hover:bg-gray-50 dark:border-gray-800 dark:hover:bg-gray-700/50">
                    <td className="px-3 py-2">
                      <input value={i.name} onChange={e => updateInstance(i.id, 'name', e.target.value)}
                        className="w-32 rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                    </td>
                    <td className="px-3 py-2">
                      <select value={i.nodeType} onChange={e => updateInstance(i.id, 'nodeType', e.target.value)}
                        className="rounded border border-gray-300 px-2 py-1 text-xs dark:border-gray-600 dark:bg-gray-700">
                        <option value="master">主</option>
                        <option value="slave">从</option>
                      </select>
                    </td>
                    <td className="px-3 py-2 font-mono text-xs text-gray-500">{i.address || '未配置'}</td>
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
                    <td className="px-3 py-2 font-mono text-xs">{i.syncFrequency}s</td>
                    <td className="px-3 py-2"><button onClick={() => removeInstance(i.id)} className="text-red-500 hover:text-red-700"><Trash2 className="h-3 w-3" /></button></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* 共享存储配置 */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><Database className="h-4 w-4" />共享存储（多实例关键）</h3>
        <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
          <div className="rounded-lg border border-gray-200 p-3 dark:border-gray-700">
            <div className="mb-2 flex items-center gap-2 text-sm font-medium"><Zap className="h-4 w-4 text-red-500" />Redis（缓存同步）</div>
            <label className="mb-1 block text-xs text-gray-500">REDIS_CONN_STRING</label>
            <input value={storage.redis.conn} onChange={e => setStorage({ ...storage, redis: { ...storage.redis, conn: e.target.value } })}
              className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
            <label className="mb-1 mt-2 flex items-center gap-1.5 text-xs text-gray-500">
              <input type="checkbox" checked={storage.redis.enabled} onChange={e => setStorage({ ...storage, redis: { ...storage.redis, enabled: e.target.checked } })} />
              启用 Redis（多实例需启用）
            </label>
          </div>
          <div className="rounded-lg border border-gray-200 p-3 dark:border-gray-700">
            <div className="mb-2 flex items-center gap-2 text-sm font-medium"><Database className="h-4 w-4 text-orange-500" />数据库（共享）</div>
            <label className="mb-1 block text-xs text-gray-500">SQL_DSN</label>
            <input value={storage.db.dsn} onChange={e => setStorage({ ...storage, db: { ...storage.db, dsn: e.target.value } })}
              className="w-full rounded border border-gray-300 px-2 py-1 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
            <label className="mb-1 mt-2 block text-xs text-gray-500">类型</label>
            <select value={storage.db.type} onChange={e => setStorage({ ...storage, db: { ...storage.db, type: e.target.value as any } })}
              className="w-full rounded border border-gray-300 px-2 py-1 text-xs dark:border-gray-600 dark:bg-gray-700">
              <option value="mysql">MySQL</option>
              <option value="postgres">PostgreSQL</option>
              <option value="sqlite">SQLite（单机）</option>
            </select>
          </div>
        </div>
        <div className="mt-3">
          <label className="mb-1 block text-xs text-gray-500">SYNC_FREQUENCY（从节点定期从 DB 同步配置，秒）</label>
          <input type="number" value={storage.syncFrequency} onChange={e => setStorage({ ...storage, syncFrequency: parseInt(e.target.value) || 60 })}
            className="w-40 rounded border border-gray-300 px-2 py-1 text-xs dark:border-gray-600 dark:bg-gray-700" />
        </div>
      </div>

      <StatusBar busy={busy} onClear={() => {}} />
    </div>
  )
}
