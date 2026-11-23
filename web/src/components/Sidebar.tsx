import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  LayoutDashboard, List, ScrollText, Settings, Server,
  Play, Sun, Moon, Languages, ChevronsLeft, ChevronsRight, Layers, Network, KeyRound, MessageSquare, Globe, Plug,
  Menu, X,
} from 'lucide-react'
import { useTheme } from '@/contexts/ThemeContext'

const navItems = [
  { id: 'dashboard', label: 'nav.dashboard', icon: LayoutDashboard },
  { id: 'tokens', label: 'nav.tokens', icon: KeyRound },
  { id: 'models', label: 'nav.models', icon: List },
  { id: 'providers', label: 'nav.providers', icon: Server },
  { id: 'proxypools', label: 'nav.proxypools', icon: Globe },
  { id: 'mcp', label: 'nav.mcp', icon: Plug },
  { id: 'logs', label: 'nav.logs', icon: ScrollText },
  { id: 'chat', label: 'nav.chat', icon: MessageSquare },
  { id: 'sync', label: 'nav.sync', icon: Play },
  { id: 'deploy', label: 'nav.deploy', icon: Network },
  { id: 'config', label: 'nav.config', icon: Settings },
]

interface Props {
  active: string
  onSelect: (id: string) => void
}

export default function Sidebar({ active, onSelect }: Props) {
  const [collapsed, setCollapsed] = useState(false)
  const [mobileOpen, setMobileOpen] = useState(false)
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

  function select(id: string) {
    onSelect(id)
    setMobileOpen(false)
  }

  const NavContent = ({ showLabel, showCollapse }: { showLabel: boolean; showCollapse: boolean }) => (
    <>
      <nav className="flex-1 space-y-1 overflow-y-auto p-2">
        {navItems.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            onClick={() => select(id)}
            title={i18n.t(label)}
            className={`flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm ${
              active === id
                ? 'bg-blue-50 font-semibold text-blue-600 dark:bg-blue-900/20'
                : 'text-gray-600 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700'
            }`}
          >
            <Icon className="h-4 w-4 shrink-0" />
            {showLabel && <span>{i18n.t(label)}</span>}
          </button>
        ))}
      </nav>

      <div className="space-y-1 border-t border-gray-200 p-2 dark:border-gray-700">
        <button onClick={toggleTheme}
          className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-gray-600 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700">
          {theme === 'dark' ? <Sun className="h-4 w-4 shrink-0" /> : <Moon className="h-4 w-4 shrink-0" />}
          {showLabel && <span>{theme === 'dark' ? '浅色' : '深色'}</span>}
        </button>
        <button onClick={toggleLanguage}
          className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-gray-600 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700">
          <Languages className="h-4 w-4 shrink-0" />
          {showLabel && <span>{i18n.language === 'zh' ? '中文' : 'English'}</span>}
        </button>
        {showCollapse && (
          <button onClick={() => setCollapsed(!collapsed)}
            className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm text-gray-600 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700">
            {collapsed ? <ChevronsRight className="h-4 w-4 shrink-0" /> : <ChevronsLeft className="h-4 w-4 shrink-0" />}
            {showLabel && <span>收起</span>}
          </button>
        )}
      </div>
    </>
  )

  return (
    <>
      {/* 移动端顶栏（<md）*/}
      <header className="fixed inset-x-0 top-0 z-40 flex h-14 items-center gap-3 border-b border-gray-200 bg-white px-3 md:hidden dark:border-gray-700 dark:bg-gray-800">
        <button onClick={() => setMobileOpen(true)} className="rounded-lg p-1.5 text-gray-600 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700">
          <Menu className="h-5 w-5" />
        </button>
        <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-blue-600 text-xs text-white">MY</div>
        <span className="font-bold">myapi</span>
      </header>

      {/* 移动端抽屉 + 遮罩 */}
      {mobileOpen && (
        <div className="fixed inset-0 z-50 md:hidden">
          <div className="absolute inset-0 bg-black/50" onClick={() => setMobileOpen(false)} />
          <aside className="absolute inset-y-0 left-0 flex w-64 flex-col border-r border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
            <div className="flex h-14 items-center justify-between border-b border-gray-200 px-4 dark:border-gray-700">
              <div className="flex items-center gap-2">
                <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-blue-600 text-xs text-white">MY</div>
                <span className="font-bold">myapi</span>
              </div>
              <button onClick={() => setMobileOpen(false)} className="rounded-lg p-1.5 text-gray-600 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700">
                <X className="h-5 w-5" />
              </button>
            </div>
            <NavContent showLabel showCollapse={false} />
          </aside>
        </div>
      )}

      {/* 桌面侧边栏（≥md）*/}
      <aside
        className={`hidden h-screen flex-col border-r border-gray-200 bg-white transition-all md:flex dark:border-gray-700 dark:bg-gray-800 ${
          collapsed ? 'md:w-16' : 'md:w-56'
        }`}
      >
        <div className="flex h-14 items-center gap-2 border-b border-gray-200 px-4 dark:border-gray-700">
          <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-blue-600 text-xs text-white">MY</div>
          {!collapsed && <span className="font-bold">myapi</span>}
        </div>
        <NavContent showLabel={!collapsed} showCollapse />
      </aside>
    </>
  )
}
