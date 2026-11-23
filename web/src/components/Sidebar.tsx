import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  LayoutDashboard, List, ScrollText, Settings, Server,
  Play, Sun, Moon, Languages, ChevronsLeft, ChevronsRight, Layers
} from 'lucide-react'
import { useTheme } from '@/contexts/ThemeContext'

const navItems = [
  { id: 'dashboard', label: 'nav.dashboard', icon: LayoutDashboard },
  { id: 'models', label: 'nav.models', icon: List },
  { id: 'providers', label: 'nav.providers', icon: Server },
  { id: 'logs', label: 'nav.logs', icon: ScrollText },
  { id: 'sync', label: 'nav.sync', icon: Play },
  { id: 'config', label: 'nav.config', icon: Settings },
]

interface Props {
  active: string
  onSelect: (id: string) => void
}

export default function Sidebar({ active, onSelect }: Props) {
  const [collapsed, setCollapsed] = useState(false)
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
    <aside
      className={`flex h-screen flex-col border-r border-gray-200 bg-white transition-all dark:border-gray-700 dark:bg-gray-800 ${
        collapsed ? 'w-16' : 'w-56'
      }`}
    >
      {/* Logo */}
      <div className="flex h-14 items-center gap-2 border-b border-gray-200 px-4 dark:border-gray-700">
        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-blue-600 text-xs text-white">CPA</div>
        {!collapsed && <span className="font-bold">cpa-sync</span>}
      </div>

      {/* 导航 */}
      <nav className="flex-1 space-y-1 overflow-y-auto p-2">
        {navItems.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            onClick={() => onSelect(id)}
            title={i18n.t(label)}
            className={`flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm ${
              active === id
                ? 'bg-blue-50 font-semibold text-blue-600 dark:bg-blue-900/20'
                : 'text-gray-600 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700'
            }`}
          >
            <Icon className="h-4 w-4 shrink-0" />
            {!collapsed && <span>{i18n.t(label)}</span>}
          </button>
        ))}
      </nav>

      {/* 底部工具 */}
      <div className="space-y-1 border-t border-gray-200 p-2 dark:border-gray-700">
        <button onClick={toggleTheme}
          className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-gray-600 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700">
          {theme === 'dark' ? <Sun className="h-4 w-4 shrink-0" /> : <Moon className="h-4 w-4 shrink-0" />}
          {!collapsed && <span>{theme === 'dark' ? '浅色' : '深色'}</span>}
        </button>
        <button onClick={toggleLanguage}
          className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-gray-600 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700">
          <Languages className="h-4 w-4 shrink-0" />
          {!collapsed && <span>{i18n.language === 'zh' ? '中文' : 'English'}</span>}
        </button>
        <button onClick={() => setCollapsed(!collapsed)}
          className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-gray-600 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700">
          {collapsed ? <ChevronsRight className="h-4 w-4 shrink-0" /> : <ChevronsLeft className="h-4 w-4 shrink-0" />}
          {!collapsed && <span>收起</span>}
        </button>
      </div>
    </aside>
  )
}
