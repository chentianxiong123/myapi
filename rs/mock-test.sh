#!/bin/bash
# myapi 网关 mock 测试（win11，全部 mock，不真调上游）
# 验证：编译 → 起服务 → /v1/models / /v1/chat/completions → 停服务
set +e
cd /d C:\Users\a1\myapi\rs 2>/dev/null || cd "C:/Users/a1/myapi/rs"

echo "========== 1. cargo build（debug，mock 验证用）=========="
cargo build 2>&1 | tail -5
if [ ! -f target/debug/myapi.exe ]; then
  echo "BUILD FAILED: 无 myapi.exe"
  exit 1
fi
echo "BUILD OK: target/debug/myapi.exe"

echo ""
echo "========== 2. 起服务（后台，端口 8380）=========="
./target/debug/myapi.exe > /tmp/myapi.log 2>&1 &
SRV_PID=$!
sleep 3
echo "服务 PID: $SRV_PID"
if ! curl -s http://127.0.0.1:8380/api/version > /dev/null 2>&1; then
  echo "服务没起来，日志："
  cat /tmp/myapi.log
  kill $SRV_PID 2>/dev/null
  exit 1
fi
echo "服务已起 ✓"

echo ""
echo "========== 3. 测 /v1/models（应返回 mock 模型清单）=========="
echo "--- GET /v1/models ---"
curl -s http://127.0.0.1:8380/v1/models
echo ""

echo ""
echo "========== 4. 测 /v1/chat/completions（mock 上游 127.0.0.1:9999 不存在 → 应 502）=========="
echo "--- POST /v1/chat/completions model=mock-model ---"
curl -s -X POST http://127.0.0.1:8380/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"mock-model","messages":[{"role":"user","content":"hi"}]}'
echo ""

echo ""
echo "========== 5. 测不存在的模型（应 404 无可用通道）=========="
echo "--- POST model=nonexistent ---"
curl -s -X POST http://127.0.0.1:8380/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"nonexistent","messages":[{"role":"user","content":"hi"}]}'
echo ""

echo ""
echo "========== 6. 停服务 =========="
kill $SRV_PID 2>/dev/null
echo "服务已停"
echo ""
echo "========== mock 测试完成 =========="
