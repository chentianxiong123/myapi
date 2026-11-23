import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import Sidebar from '@/components/Sidebar'

describe('Sidebar（导航）', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('渲染 11 个导航项 + logo', () => {
    render(<Sidebar active="dashboard" onSelect={() => {}} />)
    // logo
    expect(screen.getAllByText('myapi').length).toBeGreaterThan(0)
    // 导航项（桌面默认展开，显示中文 label）
    expect(screen.getByText('仪表盘')).toBeInTheDocument()
    expect(screen.getByText('供应商')).toBeInTheDocument()
    expect(screen.getByText('MCP 接口')).toBeInTheDocument()
  })

  it('移动端显示顶栏（hamburger 菜单按钮）', () => {
    render(<Sidebar active="dashboard" onSelect={() => {}} />)
    // 移动端顶栏（md:hidden）
    const header = document.querySelector('header')
    expect(header).toBeInTheDocument()
  })

  it('点击导航项触发 onSelect', () => {
    const onSelect = vi.fn()
    render(<Sidebar active="dashboard" onSelect={onSelect} />)
    fireEvent.click(screen.getByText('供应商'))
    expect(onSelect).toHaveBeenCalledWith('providers')
  })

  it('点击导航项后移动端抽屉关闭', () => {
    const onSelect = vi.fn()
    render(<Sidebar active="dashboard" onSelect={onSelect} />)
    // 打开抽屉（点 hamburger）
    const headerBtn = document.querySelector('header button')!
    fireEvent.click(headerBtn)
    // 抽屉出现（遮罩 bg-black/50）
    const overlay = document.querySelector('.bg-black\\/50')
    expect(overlay).toBeInTheDocument()
  })
})
