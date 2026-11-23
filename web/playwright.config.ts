import { defineConfig, devices } from '@playwright/test'

export default defineConfig({
  // E2E 测试目录（与单元测试 src/**/*.test.tsx 分开，Vitest 不收集）
  testDir: './e2e',
  // 超时
  timeout: 30_000,
  expect: { timeout: 5_000 },
  // 失败重跑
  retries: process.env.CI ? 1 : 0,
  // 并行（CI 单进程，本地并行）
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    // baseURL 与 Vite dev server 一致
    baseURL: 'http://localhost:5173',
    // 失败留痕
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: 'off',
  },
  projects: [
    { name: 'chromium-desktop', use: { ...devices['Desktop Chrome'] } },
    // 移动端：chromium 内核 + iPhone 14 视口（不装 webkit，只需 chromium）
    {
      name: 'mobile',
      use: {
        ...devices['iPhone 14'],
        // 覆盖为 chromium（iPhone 14 默认 webkit 需额外下载）
        browserName: 'chromium',
      },
    },
  ],
  // 自动启动 dev server（复用已有 5173 则不重复启动）
  webServer: {
    command: 'npm run dev -- --host 0.0.0.0',
    url: 'http://localhost:5173',
    reuseExistingServer: !process.env.CI,
    stdout: 'ignore',
    stderr: 'ignore',
    timeout: 60_000,
  },
})
