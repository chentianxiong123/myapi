import '@testing-library/jest-dom/vitest'
import { vi } from 'vitest'
import { initReactI18next } from 'react-i18next'
import i18n from 'i18next'

// --- DOM API mocks（jsdom 缺失）---
// matchMedia（Tailwind/主题/断点依赖）
Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: vi.fn().mockImplementation((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
})

// ResizeObserver（Recharts 依赖）
class ResizeObserverMock {
  observe = vi.fn()
  unobserve = vi.fn()
  disconnect = vi.fn()
}
window.ResizeObserver = ResizeObserverMock

// IntersectionObserver
class IntersectionObserverMock {
  root = null
  rootMargin = ''
  thresholds = []
  observe = vi.fn()
  unobserve = vi.fn()
  disconnect = vi.fn()
  takeRecords = vi.fn()
}
window.IntersectionObserver = IntersectionObserverMock

// getBoundingClientRect（Recharts 测量）
// scrollIntoView
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = vi.fn()
}

// --- i18n 初始化（测试用 zh 语言包）---
import zh from '@/i18n/locales/zh.json'
import en from '@/i18n/locales/en.json'

// 避免测试间重复初始化
if (!i18n.isInitialized) {
  void i18n.use(initReactI18next).init({
    resources: { zh: { translation: zh }, en: { translation: en } },
    lng: 'zh',
    fallbackLng: 'zh',
    interpolation: { escapeValue: false },
  })
}
