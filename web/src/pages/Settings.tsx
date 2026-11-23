import { useTranslation } from 'react-i18next'
import { Sun, Moon, Monitor, Languages, Database, Info } from 'lucide-react'
import { useTheme } from '@/contexts/ThemeContext'

export default function Settings() {
  const { theme, setTheme } = useTheme()
  const { i18n } = useTranslation()

  return (
    <div className="mx-auto max-w-2xl space-y-6">
      <h2 className="text-lg font-semibold">设置</h2>

      {/* 外观 */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <h3 className="mb-4 text-sm font-semibold">外观</h3>
        
        {/* 主题 */}
        <div className="mb-4">
          <label className="mb-2 block text-xs text-gray-500">主题</label>
          <div className="flex gap-2">
            {[
              { value: 'light', label: '浅色', icon: Sun },
              { value: 'dark', label: '深色', icon: Moon },
              { value: 'system', label: '系统', icon: Monitor },
            ].map(({ value, label, icon: Icon }) => (
              <button
                key={value}
                onClick={() => setTheme(value as any)}
                className={`flex items-center gap-1.5 rounded-lg border px-3 py-2 text-sm ${
                  theme === value
                    ? 'border-blue-600 bg-blue-50 text-blue-600 dark:bg-blue-900/20'
                    : 'border-gray-300 hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700'
                }`}
              >
                <Icon className="h-4 w-4" />
                {label}
              </button>
            ))}
          </div>
        </div>

        {/* 语言 */}
        <div>
          <label className="mb-2 block text-xs text-gray-500">语言</label>
          <div className="flex gap-2">
            {[
              { value: 'zh', label: '中文' },
              { value: 'en', label: 'English' },
            ].map(({ value, label }) => (
              <button
                key={value}
                onClick={() => { i18n.changeLanguage(value); localStorage.setItem('language', value) }}
                className={`rounded-lg border px-3 py-2 text-sm ${
                  i18n.language === value
                    ? 'border-blue-600 bg-blue-50 text-blue-600 dark:bg-blue-900/20'
                    : 'border-gray-300 hover:bg-gray-50 dark:border-gray-600 dark:hover:bg-gray-700'
                }`}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
      </div>

      {/* 关于 */}
      <div className="rounded-xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <h3 className="mb-4 flex items-center gap-2 text-sm font-semibold">
          <Info className="h-4 w-4" />关于
        </h3>
        <div className="space-y-2 text-sm">
          <div className="flex justify-between">
            <span className="text-gray-500">版本</span>
            <span className="font-mono">0.1.0</span>
          </div>
          <div className="flex justify-between">
            <span className="text-gray-500">项目</span>
            <span>myapi · 个人网关 + 下发器</span>
          </div>
          <div className="flex justify-between">
            <span className="text-gray-500">理念</span>
            <span>一体化（网关 + 下发器）</span>
          </div>
        </div>
      </div>
    </div>
  )
}
