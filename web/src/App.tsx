import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { LayoutDashboard, List, ScrollText, Settings, Server, Play, Sun, Moon, Languages } from 'lucide-react'
import { useTheme } from '@/contexts/ThemeContext'
import Dashboard from './pages/Dashboard'
import Config from './pages/Config'
import Providers from './pages/Providers'
import Models from './pages/Models'
import Logs from './pages/Logs'
import Sync from './pages/Sync'
import SettingsPage from './pages/Settings'
import Toast from './components/Toast'

const tabs = [
  { id: 'dashboard', label: 'nav.dashboard', icon: LayoutDashboard },
  { id: 'models', label: 'nav.models', icon: List },
  { id: 'providers', label: 'nav.providers', icon: Server },
  { id: 'logs', label: 'nav.logs', icon: ScrollText },
  { id: 'sync', label: 'nav.sync', icon: Play },
  { id: 'config', label: 'nav.config', icon: Settings },
]

export default function App() {
  const [tab, setTab] = useState('dashboard')
  const { theme, setTheme } = useTheme()
  const { i18n } = useTranslation()

  function toggleTheme() {
    setTheme(theme === 'dark' ? 'light' : 'dark')
  }

  function toggleLanguage() {
    const next = i18n.language === 'zh' ? 'en' : 'zh'
    i18n.changeLanguage(next)
    localStorage.setItem('language', next)
  }

  return (
    <div className="min-h-screen">
      <Toast />
      <header className="sticky top-0 z-20 border-b border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
        <div className="flex items-center gap-4 px-6 py-3">
          <div className="flex items-center gap-2 font-bold">
            <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-blue-600 text-xs text-white">CPA</div>
            <span>cpa-sync</span>
          </div>
          <div className="flex-1" />
          <button onClick={toggleTheme} className="rounded p-1.5 hover:bg-gray-100 dark:hover:bg-gray-700">
            {theme === 'dark' ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
          </button>
          <button onClick={toggleLanguage} className="flex items-center gap-1 rounded p-1.5 hover:bg-gray-100 dark:hover:bg-gray-700">
            <Languages className="h-4 w-4" />
            <span className="text-xs">{i18n.language === 'zh' ? '中' : 'EN'}</span>
          </button>
          <nav className="flex gap-1 rounded-lg bg-gray-100 p-1 dark:bg-gray-700">
            {tabs.map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                onClick={() => setTab(id)}
                className={`flex items-center gap-1.5 rounded-md px-3 py-1.5 text-sm ${
                  tab === id
                    ? 'bg-white font-semibold text-blue-600 shadow dark:bg-gray-800'
                    : 'text-gray-500 hover:text-gray-900 dark:hover:text-gray-100'
                }`}
              >
                <Icon className="h-4 w-4" />
                <span>{i18n.t(label)}</span>
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
        {tab === 'sync' && <Sync />}
        {tab === 'config' && <Config />}
      </main>
    </div>
  )
}
