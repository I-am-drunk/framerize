// Frozen framing-only baselines. See source-pins.sha256 and SOURCE.md.
// JSON interpretation after these operations is deliberately omitted.

pub fn nativechain_data(text: &str) -> Vec<&str> {
    let mut observed = Vec::new();
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data: ") else { continue };
        if data == "[DONE]" {
            continue;
        }
        observed.push(data);
    }
    observed
}

pub fn server_data(body: &str) -> Vec<String> {
    body
        .split("\n\n")
        .filter_map(|block| {
            let data: String = block
                .lines()
                .filter_map(|l| l.strip_prefix("data:").map(|d| d.trim_start()))
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() || data == "[DONE]" {
                return None;
            }
            Some(data)
        })
        .collect()
}
