//! 单元测试（从内联 mod tests 整段拆出）
#![allow(unused_imports)]
use super::*;

#[test]
fn anthropic_basic_to_chat() {
    let body = json!({
        "model": "claude-3-5-sonnet",
        "system": "你是助手",
        "messages": [{"role": "user", "content": "你好"}],
        "max_tokens": 1000,
        "stream": true,
    });
    let out = anthropic_to_openai(&body).unwrap();
    assert_eq!(out["model"], "claude-3-5-sonnet");
    assert_eq!(out["messages"][0]["role"], "system");
    assert_eq!(out["messages"][0]["content"], "你是助手");
    assert_eq!(out["messages"][1]["role"], "user");
    assert_eq!(out["messages"][1]["content"], "你好");
    assert_eq!(out["max_tokens"], 1000);
    assert_eq!(out["stream"], true);
}

#[test]
fn anthropic_tool_use_to_chat() {
    let body = json!({
        "model": "m",
        "messages": [{"role": "assistant", "content": [
            {"type": "tool_use", "id": "call_1", "name": "search", "input": {"q": "rust"}}
        ]}],
        "max_tokens": 100,
    });
    let out = anthropic_to_openai(&body).unwrap();
    let msg = &out["messages"][0];
    assert_eq!(msg["tool_calls"][0]["id"], "call_1");
    assert_eq!(msg["tool_calls"][0]["function"]["name"], "search");
    assert!(msg["tool_calls"][0]["function"]["arguments"].as_str().unwrap().contains("rust"));
}

#[test]
fn anthropic_tool_result_to_chat() {
    let body = json!({
        "model": "m",
        "messages": [{"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "call_1", "content": "结果"}
        ]}],
        "max_tokens": 100,
    });
    let out = anthropic_to_openai(&body).unwrap();
    assert_eq!(out["messages"][0]["role"], "tool");
    assert_eq!(out["messages"][0]["tool_call_id"], "call_1");
    assert_eq!(out["messages"][0]["content"], "结果");
}

#[test]
fn chat_response_to_anthropic() {
    let body = json!({
        "id": "chatcmpl-1",
        "model": "m",
        "choices": [{"message": {"role": "assistant", "content": "你好"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5},
    });
    let out = openai_to_anthropic(&body).unwrap();
    assert_eq!(out["role"], "assistant");
    assert_eq!(out["content"][0]["type"], "text");
    assert_eq!(out["content"][0]["text"], "你好");
    assert_eq!(out["stop_reason"], "end_turn");
    assert_eq!(out["usage"]["input_tokens"], 10);
    assert_eq!(out["usage"]["output_tokens"], 5);
}

#[test]
fn chat_tool_calls_to_anthropic() {
    let body = json!({
        "choices": [{"message": {"role": "assistant", "content": null,
            "tool_calls": [{"id": "call_1", "type": "function",
                "function": {"name": "search", "arguments": "{\"q\":\"rust\"}"}}]},
            "finish_reason": "tool_calls"}],
    });
    let out = openai_to_anthropic(&body).unwrap();
    assert_eq!(out["stop_reason"], "tool_use");
    assert_eq!(out["content"][0]["type"], "tool_use");
    assert_eq!(out["content"][0]["name"], "search");
    assert_eq!(out["content"][0]["input"]["q"], "rust");
}

#[test]
fn inject_usage_on_stream() {
    let mut body = json!({"model": "m", "stream": true, "messages": []});
    inject_stream_include_usage(&mut body);
    assert_eq!(body["stream_options"]["include_usage"], true);
}

#[test]
fn responses_message_to_chat() {
    let body = json!({
        "model": "gpt-5",
        "instructions": "你是助手",
        "input": [{"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": "你好"}]}],
        "max_output_tokens": 1000,
    });
    let out = responses_to_chat(&body).unwrap();
    assert_eq!(out["messages"][0]["role"], "system");
    assert_eq!(out["messages"][0]["content"], "你是助手");
    assert_eq!(out["messages"][1]["role"], "user");
    assert_eq!(out["messages"][1]["content"], "你好");
    assert_eq!(out["max_tokens"], 1000);
}

#[test]
fn responses_reasoning_to_chat() {
    // 思考模式：responses `reasoning: {effort}` → chat `reasoning_effort`
    let body = json!({
        "model": "gpt-5",
        "reasoning": {"effort": "high"},
        "input": [{"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": "hi"}]}],
    });
    let out = responses_to_chat(&body).unwrap();
    assert_eq!(out["reasoning_effort"], "high", "responses reasoning.effort 应转 chat reasoning_effort");
}

#[test]
fn responses_function_call_to_chat() {
    let body = json!({
        "model": "gpt-5",
        "input": [
            {"type": "function_call", "call_id": "call_1", "name": "search",
             "arguments": "{\"q\":\"rust\"}"},
            {"type": "function_call_output", "call_id": "call_1", "output": "结果"},
        ],
    });
    let out = responses_to_chat(&body).unwrap();
    assert_eq!(out["messages"][0]["role"], "assistant");
    assert_eq!(out["messages"][0]["tool_calls"][0]["function"]["name"], "search");
    assert_eq!(out["messages"][1]["role"], "tool");
    assert_eq!(out["messages"][1]["tool_call_id"], "call_1");
    assert_eq!(out["messages"][1]["content"], "结果");
}

#[test]
fn chat_response_to_responses() {
    let body = json!({
        "id": "chatcmpl-1", "model": "m", "created": 123,
        "choices": [{"message": {"role": "assistant", "content": "你好"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5},
    });
    let out = chat_to_responses(&body).unwrap();
    assert_eq!(out["object"], "response");
    assert_eq!(out["status"], "completed");
    assert_eq!(out["output"][0]["type"], "message");
    assert_eq!(out["output"][0]["content"][0]["text"], "你好");
    assert_eq!(out["usage"]["input_tokens"], 10);
}

#[test]
fn chat_tool_calls_to_responses() {
    let body = json!({
        "choices": [{"message": {"role": "assistant", "content": null,
            "tool_calls": [{"id": "call_1", "type": "function",
                "function": {"name": "search", "arguments": "{\"q\":\"rust\"}"}}]},
            "finish_reason": "tool_calls"}],
    });
    let out = chat_to_responses(&body).unwrap();
    assert_eq!(out["output"][0]["type"], "function_call");
    assert_eq!(out["output"][0]["name"], "search");
    assert_eq!(out["output"][0]["call_id"], "call_1");
}

#[test]
fn chat_to_anthropic_request_basic() {
    let body = json!({
        "model": "m", "max_tokens": 100, "temperature": 0.5,
        "messages": [
            {"role": "system", "content": "你是助手"},
            {"role": "user", "content": "你好"},
            {"role": "assistant", "content": "好的",
             "tool_calls": [{"id": "call_1", "type": "function",
                 "function": {"name": "search", "arguments": "{\"q\":\"r\"}"}}]},
            {"role": "tool", "tool_call_id": "call_1", "content": "结果"},
        ],
        "tools": [{"type": "function", "function": {"name": "search",
            "description": "搜索", "parameters": {"type": "object", "properties": {}}}}],
    });
    let out = chat_to_anthropic_request(&body).unwrap();
    assert_eq!(out["system"], "你是助手");
    assert_eq!(out["messages"][0]["role"], "user");
    assert_eq!(out["messages"][0]["content"][0]["type"], "text");
    assert_eq!(out["messages"][1]["role"], "assistant");
    assert_eq!(out["messages"][1]["content"][1]["type"], "tool_use");
    assert_eq!(out["messages"][1]["content"][1]["name"], "search");
    assert_eq!(out["messages"][2]["role"], "user");
    assert_eq!(out["messages"][2]["content"][0]["type"], "tool_result");
    assert_eq!(out["max_tokens"], 100);
    assert_eq!(out["tools"][0]["type"], "tool");
    assert_eq!(out["tools"][0]["name"], "search");
}

#[test]
fn anthropic_response_to_chat() {
    let body = json!({
        "id": "msg_1", "model": "claude-x",
        "content": [
            {"type": "text", "text": "你好"},
            {"type": "tool_use", "id": "call_1", "name": "search", "input": {"q": "r"}},
        ],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 3},
    });
    let out = anthropic_to_openai_response(&body).unwrap();
    assert_eq!(out["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(out["choices"][0]["message"]["content"], "你好");
    assert_eq!(out["choices"][0]["message"]["tool_calls"][0]["function"]["name"], "search");
    assert_eq!(out["usage"]["prompt_tokens"], 13);
    assert_eq!(out["usage"]["completion_tokens"], 5);
}

#[test]
fn anthropic_response_thinking_to_reasoning() {
    let body = json!({
        "content": [{"type": "thinking", "thinking": "思考中"}],
        "stop_reason": "end_turn",
    });
    let out = anthropic_to_openai_response(&body).unwrap();
    assert_eq!(out["choices"][0]["message"]["reasoning_content"], "思考中");
}

#[test]
fn thinking_object_enabled_maps_by_budget() {
    // 学 cc-switch：Anthropic thinking 是**对象**（{type, budget_tokens}），不是数组。
    // enabled + budget 分段：<4000 low / <16000 medium / 其余 high。
    assert_eq!(resolve_reasoning_effort(&json!({"thinking": {"type": "enabled", "budget_tokens": 2048}})), Some("low"));
    assert_eq!(resolve_reasoning_effort(&json!({"thinking": {"type": "enabled", "budget_tokens": 8192}})), Some("medium"));
    assert_eq!(resolve_reasoning_effort(&json!({"thinking": {"type": "enabled", "budget_tokens": 24576}})), Some("high"));
}

#[test]
fn thinking_adaptive_maps_xhigh() {
    // adaptive（Claude 5.x 默认）→ xhigh（最大推理）
    assert_eq!(resolve_reasoning_effort(&json!({"thinking": {"type": "adaptive"}})), Some("xhigh"));
}

#[test]
fn thinking_disabled_or_missing_is_none() {
    assert_eq!(resolve_reasoning_effort(&json!({"thinking": {"type": "disabled"}})), None);
    assert_eq!(resolve_reasoning_effort(&json!({})), None);
}

#[test]
fn output_config_effort_wins_and_floor_when_thinking_off() {
    // Priority 1：output_config.effort 优先（thinking on 时原样保留）
    assert_eq!(resolve_reasoning_effort(&json!({"thinking": {"type": "enabled"}, "output_config": {"effort": "high"}})), Some("high"));
    assert_eq!(resolve_reasoning_effort(&json!({"thinking": {"type": "enabled"}, "output_config": {"effort": "xhigh"}})), Some("xhigh"));
    // Claude 5.x 省略 thinking = adaptive 默认（不 floor）
    assert_eq!(resolve_reasoning_effort(&json!({"model": "claude-opus-5", "output_config": {"effort": "high"}})), Some("high"));
    // 显式 thinking: disabled + effort → floor low
    assert_eq!(resolve_reasoning_effort(&json!({"thinking": {"type": "disabled"}, "output_config": {"effort": "high"}})), Some("low"));
    // 非 Claude 模型省略 thinking（=off）+ effort → floor low
    assert_eq!(resolve_reasoning_effort(&json!({"model": "gpt-5", "output_config": {"effort": "high"}})), Some("low"));
}

#[test]
fn effort_max_clamped_by_model() {
    // max 仅支持模型的模型保留，否则回退 xhigh（均带 thinking on）
    assert_eq!(resolve_reasoning_effort(&json!({"model": "gpt-5.6", "thinking": {"type": "enabled"}, "output_config": {"effort": "max"}})), Some("max"));
    assert_eq!(resolve_reasoning_effort(&json!({"model": "gpt-5", "thinking": {"type": "enabled"}, "output_config": {"effort": "max"}})), Some("xhigh"));
}

#[test]
fn gpt_pro_effort_clamped() {
    // gpt-5-pro 强制 high；其他 gpt-pro 最低 medium（带 thinking on）
    assert_eq!(resolve_reasoning_effort(&json!({"model": "gpt-5-pro", "thinking": {"type": "enabled"}, "output_config": {"effort": "low"}})), Some("high"));
    assert_eq!(resolve_reasoning_effort(&json!({"model": "gpt-4-pro", "thinking": {"type": "enabled"}, "output_config": {"effort": "low"}})), Some("medium"));
}

#[test]
fn chat_effort_to_anthropic_thinking_budget() {
    // 学 cc-switch effort_to_thinking_budget：2048/8192/16384/24576
    let out = chat_to_anthropic_request(&json!({"model": "m", "reasoning_effort": "high"})).unwrap();
    assert_eq!(out["thinking"]["type"], "enabled");
    assert_eq!(out["thinking"]["budget_tokens"], 16384);
    let out2 = chat_to_anthropic_request(&json!({"model": "m", "reasoning_effort": "xhigh"})).unwrap();
    assert_eq!(out2["thinking"]["budget_tokens"], 24576);
}