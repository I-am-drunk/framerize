//! Check new inference requests once. Never recheck an accepted body.
use axum::{body::Body, extract::{Request, State}, middleware::Next,
    response::{IntoResponse, Response}, http::StatusCode, Json};
use std::{path::Path, sync::{Arc, atomic::{AtomicU64, Ordering}}};

const REQUEST_RESERVE: u64 = 64 * 1024 * 1024;
type Sample = Arc<dyn Fn() -> Option<i128> + Send + Sync>;

pub(crate) struct Admission {
    reserved: AtomicU64,
    sample: Sample,
}

impl Default for Admission {
    fn default() -> Self {
        Self { reserved: AtomicU64::new(0), sample: Arc::new(||
            headroom(Path::new("/proc"), Path::new("/sys/fs/cgroup"))) }
    }
}

struct Reservation(Arc<Admission>);
fn inference(request: &Request) -> bool {
    request.method() == axum::http::Method::POST && matches!(
        request.uri().path().strip_prefix("/paid").unwrap_or(request.uri().path()),
        "/api/run" | "/anthropic/v1/messages" | "/openai/v1/responses"
        | "/openai/v1/chat/completions")
}

pub(crate) async fn middleware(State(gate): State<Arc<Admission>>,
    request: Request, next: Next) -> Response {
    if !inference(&request) { return next.run(request).await; }
    let reserved = gate.reserved.fetch_add(REQUEST_RESERVE, Ordering::SeqCst);
    let permit = Reservation(gate.clone());
    if let Some(free) = (gate.sample)().filter(|free|
        *free < i128::from(reserved) + i128::from(REQUEST_RESERVE)) {
        framer_core::observability::Event::new("memory.request_refused")
            .field("headroom_bytes", free.clamp(i64::MIN as i128, i64::MAX as i128) as i64)
            .field("reserved_bytes", reserved)
            .field("requested_bytes", REQUEST_RESERVE)
            .outcome("deferred").emit();
        return crate::operation_ownership::completed_response((
            StatusCode::SERVICE_UNAVAILABLE, [("retry-after", "5")],
            Json(serde_json::json!({"error": {"type": "server_overloaded",
                "code": "memory_pressure", "message": "Low available RAM: new request deferred. Retry shortly."}}))
        ).into_response(), true);
    }
    let (parts, body) = next.run(request).await.into_parts();
    Response::from_parts(parts, Body::new(AdmittedBody { body, _permit: permit }))
}

struct AdmittedBody { body: Body, _permit: Reservation }

impl http_body::Body for AdmittedBody {
    type Data = axum::body::Bytes;
    type Error = axum::Error;
    fn poll_frame(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>)
        -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        std::pin::Pin::new(&mut self.body).poll_frame(cx)
    }
    fn is_end_stream(&self) -> bool { self.body.is_end_stream() }
    fn size_hint(&self) -> http_body::SizeHint { self.body.size_hint() }
}

fn number(path: &Path) -> Option<i128> {
    std::fs::read_to_string(path).ok()?.trim().parse::<u64>().ok().map(i128::from)
}

fn host_headroom(text: &str) -> Option<i128> {
    let value = |key| text.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        (fields.next()? == key).then(|| fields.next()?.parse::<u64>().ok()).flatten()
    }).map(|kb| i128::from(kb) * 1024);
    Some(value("MemAvailable:")? - value("MemTotal:")? / 10)
}

fn headroom(proc: &Path, root: &Path) -> Option<i128> {
    let mut available = std::fs::read_to_string(proc.join("meminfo")).ok()
        .and_then(|text| host_headroom(&text));
    if let Ok(groups) = std::fs::read_to_string(proc.join("self/cgroup")) {
        if let Some(group) = groups.lines().find_map(|line| line.strip_prefix("0::")) {
            let mut node = root.join(group.trim_start_matches('/'));
            while node.starts_with(root) {
                if let (Some(current), Some(limit)) =
                    (number(&node.join("memory.current")), number(&node.join("memory.max"))) {
                    let remaining = limit * 9 / 10 - current;
                    available = Some(available.map_or(remaining, |old| old.min(remaining)));
                }
                if node == root || !node.pop() { break; }
            }
        }
    }
    available
}

#[cfg(test)]
#[path = "memory_admission_tests.rs"]
mod tests;

impl Drop for Reservation {
    fn drop(&mut self) {
        self.0.reserved.fetch_sub(REQUEST_RESERVE, Ordering::SeqCst);
    }
}
