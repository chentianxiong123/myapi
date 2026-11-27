//! 单元测试（从内联 mod tests 整段拆出）
#![allow(unused_imports)]
use super::*;

#[test]
fn chat_sse_to_anthropic_text() {
    let mut st = AnthropicSseState::new();
    let mut out = Vec::new();
    out.extend(st.push(r#"{"choices":[{"delta":{"content":"你好"},"index":0}],"model":"m"}"#));
    out.extend(st.push(r#"{"choices":[{"delta":{"content":"，世界"},"index":0}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#));
    out.extend(st.finish());
    let joined = out.join("\n");
    assert!(joined.contains("message_start"));
    assert!(joined.contains("content_block_start"));
    assert!(joined.contains("text_delta"));
    assert!(joined.contains("你好"));
    assert!(joined.contains("message_delta"));
    assert!(joined.contains("message_stop"));
    // 三桶 usage：input=10, output=5（无 cache）
    assert!(joined.contains("\"input_tokens\":10"));
    assert!(joined.contains("\"output_tokens\":5"));
}

#[test]
fn chat_sse_to_anthropic_tool_call() {
    let mut st = AnthropicSseState::new();
    let mut out = Vec::new();
    out.extend(st.push(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"search","arguments":""}}]},"index":0}]}"#));
    out.extend(st.push(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"q\":\"r\"}"}}]},"finish_reason":"tool_calls","index":0}]}"#));
    out.extend(st.finish());
    let joined = out.join("\n");
    assert!(joined.contains("tool_use"));
    assert!(joined.contains("search"));
    assert!(joined.contains("message_delta"));
    assert!(joined.contains("\"stop_reason\":\"tool_use\""));
}

#[test]
fn chat_sse_to_responses_text() {
    let mut st = ResponsesSseState::new();
    let mut out = Vec::new();
    out.extend(st.push(r#"{"choices":[{"delta":{"content":"hi"},"index":0}],"model":"m"}"#));
    out.extend(st.finish());
    let joined = out.join("\n");
    assert!(joined.contains("response.created"));
    assert!(joined.contains("response.output_text.delta"));
    assert!(joined.contains("\"delta\":\"hi\""));
    assert!(joined.contains("response.output_text.done"));
    assert!(joined.contains("response.completed"));
    assert!(joined.contains("\"status\":\"completed\""));
}

#[test]
fn chat_sse_to_responses_reasoning_not_lost() {
    // 思考模式：上游 chat 流式 reasoning_content → responses 流式 reasoning 项（不丢）
    let mut st = ResponsesSseState::new();
    let mut out = Vec::new();
    out.extend(st.push(r#"{"choices":[{"delta":{"reasoning_content":"思考中"},"index":0}],"model":"m"}"#));
    out.extend(st.push(r#"{"choices":[{"delta":{"content":"答案"},"index":0}]}"#));
    out.extend(st.finish());
    let joined = out.join("\n");
    // reasoning 项事件齐全：added + delta + done
    assert!(joined.contains("response.output_item.added"), "缺 output_item.added");
    assert!(joined.contains("response.reasoning_summary_text.added"), "缺 reasoning_summary_text.added");
    assert!(joined.contains("response.reasoning_summary_text.delta"), "缺 reasoning_summary_text.delta");
    assert!(joined.contains("\"delta\":\"思考中\""), "思考内容丢失");
    assert!(joined.contains("response.reasoning_summary_text.done"), "缺 reasoning_summary_text.done");
    assert!(joined.contains("response.output_item.done"), "缺 reasoning item done");
    // 正文仍完整
    assert!(joined.contains("\"delta\":\"答案\""), "正文丢失");
}