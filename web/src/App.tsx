import { useState } from 'react'
import { LayoutDashboard, List, ScrollText, Settings, Server } from 'lucide-react'
import Dashboard from './pages/Dashboard'
import Logs from './pages/Logs'
import Config from './pages/Config'
import Providers from './pages/Providers'
import Models from './pages/Models'
import Toast from './components/Toast'

const tabs = [
  { id: 'dashboard', label: '仪表盘', icon: LayoutDashboard },
  { id: 'models', label: '模型', icon: List },
  { id: 'providers', label: 'Provider', icon: Server },
  { id: 'logs', label: '日志', icon: ScrollText },
  { id: 'config', label: '配置', icon: Settings },
]

export default function App() {
  const [tab, setTab] = useState('dashboard')

  return (
    <div className="min-h-screen">
      <Toast />
      <header className="sticky top-0 z-20 border-b border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <div className="flex items-center gap-4 px-6 py-3">
          <div className="flex items-center gap-2 font-bold">
            <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-blue-600 text-xs text-white">CPA</div>
            <span>cpa-sync · 个人网关</span>
          </div>
          <div className="flex-1" />
          <nav className="flex gap-1 rounded-lg bg-gray-100 p-1 dark:bg-gray-700">
            {tabs.map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                onClick={() => setTab(id)}
                className={`flex items-center gap-1.5 rounded-md px-3 py-1.5 text-sm ${
                  tab === id ? 'bg-white font-semibold text-blue-600 shadow dark:bg-gray-800' : 'text-gray-500 hover:text-gray-900 dark:hover:text-gray-100'
                }`}
              >
                <Icon className="h-4 w-4" />
                {label}
              </button>
            ))}
          </nav>
        </div>
      </header>
      <main className="p-6">
        {tab === 'dashboard' && <Dashboard />}
        {tab === 'models' && <Models />}
        {tab === 'providers' && <Providers />}
        {tab === 'logs' && <Logs />}
        {tab === 'config' && <Config />}
      </main>
    </div>
  )
}
