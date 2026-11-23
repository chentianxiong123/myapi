import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, act } from '@testing-library/react'
import Toast, { toast } from '@/components/Toast'

describe('Toast（提示）', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('渲染 Toast 组件', () => {
    render(<Toast />)
    expect(document.querySelector('.fixed')).toBeInTheDocument()
  })

  it('toast() 显示消息并自动消失', () => {
    vi.useFakeTimers()
    render(<Toast />)
    act(() => { toast('测试消息') })
    expect(screen.getByText('测试消息')).toBeInTheDocument()
    // 2.2s 后消失
    act(() => { vi.advanceTimersByTime(2300) })
    expect(screen.getByText('测试消息')).toHaveClass('opacity-0')
  })
})
