/**
 * 请求日志的列：顺序即表格从左到右。`locked` 的列不能关（时间列是打开详情的入口）。
 * 学 cc-switch requestLogColumns。
 */
export const REQUEST_LOG_COLUMNS = [
  { id: 'time', label: '时间', defaultVisible: true, locked: true },
  { id: 'app', label: '应用', defaultVisible: true },
  { id: 'provider', label: 'Provider', defaultVisible: true },
  { id: 'model', label: '模型', defaultVisible: true },
  { id: 'stream', label: '流', defaultVisible: true },
  { id: 'input', label: '输入', defaultVisible: true },
  { id: 'output', label: '输出', defaultVisible: true },
  { id: 'cache_read', label: '缓存命中', defaultVisible: true },
  { id: 'cache_write', label: '缓存写入', defaultVisible: false },
  { id: 'hit_rate', label: '命中率', defaultVisible: false },
  { id: 'speed', label: '速度', defaultVisible: true },
  { id: 'ttft', label: '首字', defaultVisible: true },
  { id: 'duration', label: '总耗时', defaultVisible: false },
  { id: 'status_code', label: '状态码', defaultVisible: false },
] as const satisfies readonly {
  id: string
  label: string
  defaultVisible: boolean
  locked?: boolean
}[]

export type RequestLogColumnId = (typeof REQUEST_LOG_COLUMNS)[number]['id']

const isLocked = (column: (typeof REQUEST_LOG_COLUMNS)[number]) =>
  'locked' in column && column.locked

/** 按表格顺序列出要显示的列；认不出的 id 忽略 */
export function resolveRequestLogColumns(
  overrides: Record<string, boolean> | undefined,
): RequestLogColumnId[] {
  return REQUEST_LOG_COLUMNS.filter((column) => {
    if (isLocked(column)) return true
    const override = overrides?.[column.id]
    return typeof override === 'boolean' ? override : column.defaultVisible
  }).map((column) => column.id)
}

/** 改一列：和默认值相同就把这条覆盖删掉，只留真正改过的 */
export function setRequestLogColumn(
  overrides: Record<string, boolean> | undefined,
  id: RequestLogColumnId,
  visible: boolean,
): Record<string, boolean> {
  const column = REQUEST_LOG_COLUMNS.find((c) => c.id === id)
  const next = { ...overrides }
  if (!column || isLocked(column)) return next
  if (visible === column.defaultVisible) {
    delete next[id]
  } else {
    next[id] = visible
  }
  return next
}

/** 菜单里能开关的列（不含时间列） */
export const TOGGLEABLE_REQUEST_LOG_COLUMNS = REQUEST_LOG_COLUMNS.filter(
  (column) => !isLocked(column),
)

/** 状态码筛选选项（学 cc-switch STATUS_CODE_OPTIONS） */
export const STATUS_CODE_OPTIONS = [200, 400, 401, 429, 500] as const
