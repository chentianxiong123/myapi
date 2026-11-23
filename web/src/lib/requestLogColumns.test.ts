import { describe, it, expect } from 'vitest'
import { cn } from '@/lib/utils'
import {
  REQUEST_LOG_COLUMNS,
  resolveRequestLogColumns,
  setRequestLogColumn,
  TOGGLEABLE_REQUEST_LOG_COLUMNS,
  STATUS_CODE_OPTIONS,
} from '@/lib/requestLogColumns'

describe('cn（class 合并）', () => {
  it('合并 clsx + tailwind-merge', () => {
    expect(cn('px-2', 'py-1')).toBe('px-2 py-1')
  })
  it('tailwind-merge 覆盖冲突类', () => {
    expect(cn('px-2', 'px-4')).toBe('px-4')
  })
  it('处理条件类', () => {
    expect(cn('text-sm', false && 'hidden', 'font-bold')).toBe('text-sm font-bold')
  })
})

describe('REQUEST_LOG_COLUMNS（日志列定义）', () => {
  it('含 time 且 time 锁定（不可切换）', () => {
    const time = REQUEST_LOG_COLUMNS.find(c => c.id === 'time')
    expect(time).toBeDefined()
    expect(time?.locked).toBe(true)
  })
  it('key_name 列存在（区分使用者）', () => {
    const keyName = REQUEST_LOG_COLUMNS.find(c => c.id === 'key_name')
    expect(keyName).toBeDefined()
    expect(keyName?.label).toBeTruthy()
  })
  it('TOGGLEABLE 不含 time（locked）', () => {
    expect(TOGGLEABLE_REQUEST_LOG_COLUMNS.some(c => c.id === 'time')).toBe(false)
  })
  it('STATUS_CODE_OPTIONS 含常见状态码', () => {
    expect(STATUS_CODE_OPTIONS).toContain(200)
    expect(STATUS_CODE_OPTIONS).toContain(500)
  })
})

describe('resolveRequestLogColumns（overrides → 列）', () => {
  it('无 overrides 返回默认可见列，且首列为 time', () => {
    const cols = resolveRequestLogColumns(undefined)
    expect(cols[0]).toBe('time')
    expect(new Set(cols).size).toBe(cols.length)
  })
  it('显式覆盖：model 强制显示/隐藏', () => {
    expect(resolveRequestLogColumns({ model: true })).toContain('model')
    expect(resolveRequestLogColumns({ model: false })).not.toContain('model')
  })
  it('time 始终保留（locked，不受 override 影响）', () => {
    expect(resolveRequestLogColumns({ time: false })).toContain('time')
  })
})

describe('setRequestLogColumn（改一列）', () => {
  it('设为非默认 → 写入 override', () => {
    const col = REQUEST_LOG_COLUMNS.find(c => c.id === 'model')!
    const next = setRequestLogColumn(undefined, 'model', !col.defaultVisible)
    expect(next.model).toBe(!col.defaultVisible)
  })
  it('设为默认 → 删除 override', () => {
    const col = REQUEST_LOG_COLUMNS.find(c => c.id === 'model')!
    const next = setRequestLogColumn({ model: !col.defaultVisible }, 'model', col.defaultVisible)
    expect(next.model).toBeUndefined()
  })
  it('time（locked）不可改', () => {
    expect(setRequestLogColumn(undefined, 'time', false)).toEqual({})
  })
})
