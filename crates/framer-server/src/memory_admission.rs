//! Check new inference requests once. Never recheck an accepted body.
use axum::{body::Body, extract::{Request, State}, middleware::Next,
    response::{IntoResponse, Response}, http::StatusCode, Json};
use std::{path::Path, sync::{Arc, atomic::{AtomicU64, Ordering}}};

const REQUEST_RESERVE: u64 = 64 * 1024 * 1024;
const BROWSER_STARTUP_RESERVE: u64 = 1024 * 1024 * 1024;
type Sample = Arc<dyn Fn() -> Option<MemorySample> + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq)]
struct MemorySample {
    headroom: i128,
    scope: &'static str,
    used: i128,
    total: i128,
}

impl MemorySample {
    fn refusal(self, reserved: u64) -> String {
        let gib = 1024_f64.powi(3);
        let mib = 1024_f64.powi(2);
        format!("Low available RAM: insufficient memory below the 90% admission watermark for a new request. {}: {:.2} of {:.2} GiB; \
            admission headroom {:.2} MiB, reserved {:.2} MiB, new request needs {:.2} MiB. Retry shortly.",
            self.scope, self.used as f64 / gib, self.total as f64 / gib,
            self.headroom as f64 / mib, reserved as f64 / mib, REQUEST_RESERVE as f64 / mib)
    }
}

pub(crate) struct Admission {
    reserved: AtomicU64,
    sample: Sample,
}

impl Admission {
    pub(crate) fn browser_deferral(&self) -> Option<String> {
        let reserved = self.reserved.load(Ordering::SeqCst);
        let sample = (self.sample)()?;
        if sample.headroom >= i128::from(reserved) + i128::from(BROWSER_STARTUP_RESERVE) {
            return None;
        }
        Some(format!("Low available RAM: no resident browser was available for this request, and a new browser needs 1024.00 MiB. {}: {:.2} of {:.2} GiB; admission headroom {:.2} MiB, reserved {:.2} MiB. Retry shortly.",
            sample.scope, sample.used as f64 / 1024_f64.powi(3),
            sample.total as f64 / 1024_f64.powi(3),
            sample.headroom as f64 / 1024_f64.powi(2), reserved as f64 / 1024_f64.powi(2)))
    }

    #[cfg(test)]
    pub(crate) fn fixed(headroom: Option<i128>) -> Self {
        Self { reserved: AtomicU64::new(0), sample: Arc::new(move || headroom.map(|headroom|
            MemorySample { headroom, scope: "fixture RAM", used: 0, total: 1024 * 1024 * 1024 })) }
    }
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
    if let Some(sample) = (gate.sample)().filter(|sample|
        sample.headroom < i128::from(reserved) + i128::from(REQUEST_RESERVE)) {
        framer_core::observability::Event::new("memory.request_refused")
            .field("cause", sample.refusal(reserved))
            .field("scope", sample.scope)
            .field("used_bytes", sample.used.clamp(0, u64::MAX as i128) as u64)
            .field("total_bytes", sample.total.clamp(0, u64::MAX as i128) as u64)
            .field("headroom_bytes", sample.headroom.clamp(i64::MIN as i128, i64::MAX as i128) as i64)
            .field("reserved_bytes", reserved)
            .field("requested_bytes", REQUEST_RESERVE)
            .outcome("deferred").emit();
        return crate::operation_ownership::completed_response((
            StatusCode::SERVICE_UNAVAILABLE, [("retry-after", "5")],
            Json(serde_json::json!({"error": {"type": "server_overloaded",
                "code": "memory_pressure", "message": sample.refusal(reserved)}}))
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

fn host_headroom(text: &str) -> Option<MemorySample> {
    let value = |key| text.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        (fields.next()? == key).then(|| fields.next()?.parse::<u64>().ok()).flatten()
    }).map(|kb| i128::from(kb) * 1024);
    let (available, total) = (value("MemAvailable:")?, value("MemTotal:")?);
    Some(MemorySample { headroom: available - total / 10,
        scope: "host RAM used/unavailable", used: total - available, total })
}

fn headroom(proc: &Path, root: &Path) -> Option<MemorySample> {
    let mut available = std::fs::read_to_string(proc.join("meminfo")).ok()
        .and_then(|text| host_headroom(&text));
    if let Ok(groups) = std::fs::read_to_string(proc.join("self/cgroup")) {
        if let Some(group) = groups.lines().find_map(|line| line.strip_prefix("0::")) {
            let mut node = root.join(group.trim_start_matches('/'));
            while node.starts_with(root) {
                if let (Some(current), Some(limit)) =
                    (number(&node.join("memory.current")), number(&node.join("memory.max"))) {
                    let remaining = limit * 9 / 10 - current;
                    if available.is_none_or(|old| remaining < old.headroom) {
                        available = Some(MemorySample { headroom: remaining,
                            scope: "limiting cgroup RAM used", used: current, total: limit });
                    }
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
