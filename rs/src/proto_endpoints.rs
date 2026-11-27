//! proto_endpoints（精确拆分）
use super::*;

/// 流式响应：读上游 chat SSE 流 → 逐行转换 → 目标协议 SSE 流（学 cc-switch streaming 管道）
pub(crate) async fn proto_stream_reply<S: protocol_stream::SseTransformer + Send + 'static>(
    resp: axum::response::Response,
    mut state: S,
) -> axum::response::Response {
    let status = resp.status();
    if !status.is_success() {
        let (parts, body) = resp.into_parts();
        let bytes = http_body_util::BodyExt::collect(body).await.map(|b| b.to_bytes()).unwrap_or_default();
        let mut r = axum::response::Response::builder().status(status);
        if let Some(ct) = parts.headers.get("content-type") { r = r.header("content-type", ct); }
        return r.body(axum::body::Body::from(bytes)).unwrap_or_else(|_| proto_error(502, "错误响应构造失败"));
    }
    let (parts, body) = resp.into_parts();
    use http_body_util::BodyExt;
    let mut chunks = body.into_data_stream();
    let mut buffer = String::new();
    let mut finished = false;
    let stream = futures_util::stream::poll_fn(move |cx| -> std::task::Poll<Option<Result<bytes::Bytes, std::convert::Infallible>>> {
        loop {
            if finished { return std::task::Poll::Ready(None); }
            if let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data == "[DONE]" {
                        let out = state.finish();
                        finished = true;
                        if !out.is_empty() {
                            return std::task::Poll::Ready(Some(Ok(bytes::Bytes::from(out.join("")))));
                        }
                        return std::task::Poll::Ready(None);
                    }
                    let out = state.push(data);
                    if !out.is_empty() {
                        return std::task::Poll::Ready(Some(Ok(bytes::Bytes::from(out.join("")))));
                    }
                }
                continue;
            }
            match futures_util::StreamExt::poll_next_unpin(&mut chunks, cx) {
                std::task::Poll::Ready(Some(Ok(b))) => {
                    buffer.push_str(&String::from_utf8_lossy(&b));
                }
                std::task::Poll::Ready(Some(Err(_))) | std::task::Poll::Ready(None) => {
                    let out = state.finish();
                    finished = true;
                    if !out.is_empty() {
                        return std::task::Poll::Ready(Some(Ok(bytes::Bytes::from(out.join("")))));
                    }
                    return std::task::Poll::Ready(None);
                }
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    });
    let mut r = axum::response::Response::builder().status(200).header("content-type", "text/event-stream");
    for (k, v) in parts.headers.iter() {
        if k != "content-type" && k != "content-length" { r = r.header(k, v); }
    }
    r.body(axum::body::Body::from_stream(stream)).unwrap_or_else(|_| proto_error(502, "流式响应构造失败"))
}

/// 读 chat 响应 body → 转目标协议 → 返回；错误时透传原始 body
pub(crate) async fn proto_reply<F>(resp: axum::response::Response, f: F) -> axum::response::Response
where F: Fn(&Value) -> Result<Value, String> {
    let status = resp.status();
    let (parts, body) = resp.into_parts();
    let bytes = http_body_util::BodyExt::collect(body).await.map(|b| b.to_bytes()).unwrap_or_default();
    if status.is_success() {
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(v) => match f(&v) {
                Ok(converted) => {
                    let mut r = axum::response::Response::builder().status(status);
                    if let Some(ct) = parts.headers.get("content-type") { r = r.header("content-type", ct); }
                    return r.body(axum::body::Body::from(serde_json::to_vec(&converted).unwrap_or_default()))
                        .unwrap_or_else(|_| proto_error(500, "响应构造失败"));
                }
                Err(e) => return proto_error(502, &e),
            },
            Err(e) => return proto_error(502, &format!("响应解析失败: {e}")),
        }
    }
    let mut r = axum::response::Response::builder().status(status);
    if let Some(ct) = parts.headers.get("content-type") { r = r.header("content-type", ct); }
    r.body(axum::body::Body::from(bytes)).unwrap_or_else(|_| proto_error(502, "错误响应构造失败"))
}

pub(crate) fn proto_error(code: u16, msg: &str) -> axum::response::Response {
    (
        axum::http::StatusCode::from_u16(code).unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
        axum::Json(serde_json::json!({"error": {"message": msg, "type": "api_error"}})),
    ).into_response()
}