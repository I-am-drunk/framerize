use super::*;
pub(super) const BODY_LIMIT: usize = 64 * 1024;

/// Both request bodies are fixed before browser work starts. Only the fresh
/// top-level turn identifier may differ; the host never invents metadata.
pub struct ComparisonPlan {
    pub(super) project: ProjectId,
    pub(super) path: String,
    pub(super) baseline: Value,
    pub(super) native: Value,
}

fn turn(value: &Value) -> bool {
    value.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 256
        && !s.chars().any(char::is_control))
}
fn bounded(value: &Value) -> bool {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > BODY_LIMIT.saturating_sub(self.0) {
                return Err(std::io::Error::other(FAILURE));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    serde_json::to_writer(Count(0), value).is_ok()
}
impl ComparisonPlan {
    pub fn new(project: ProjectId, path: &str, baseline: Value, native: Value)
        -> Result<Self, &'static str> {
        if !matches!(path, "/ai/v3/responses/" | "/ai/v3/messages/")
            || project.0.is_empty() || project.0.len() > 256
            || !project.0.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || !bounded(&baseline) || !bounded(&native) {
            return Err(FAILURE);
        }
        let (Some(a), Some(b)) = (baseline.as_object(), native.as_object()) else {
            return Err(FAILURE);
        };
        if a.len() != b.len() || a.contains_key("castleToken")
            || a.iter().any(|(key, value)| key != "turnId" && b.get(key) != Some(value))
            || baseline["projectId"] != project.0 || baseline["request"]["stream"] != false
            || !turn(&baseline["turnId"]) || !turn(&native["turnId"])
            || baseline["turnId"] == native["turnId"] {
            return Err(FAILURE);
        }
        Ok(Self { project, path: path.into(), baseline, native })
    }
}
