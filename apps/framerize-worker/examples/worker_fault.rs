//! Deliberately broken, separately built peer. Never a production worker mode.
use framer_worker_wire::{frame, message::{self as wire, Message}, WireError};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

async fn send(output: &mut (impl AsyncWrite + Unpin), message: Message) -> Result<(), WireError> {
    frame::write(output, &frame::encode(&message)?).await
}
async fn next(input: &mut (impl AsyncRead + Unpin)) -> Result<Message, WireError> {
    Ok(frame::read::<_, Message>(input).await?.ok_or(WireError::Protocol)?.message)
}
async fn accepted(input: &mut (impl AsyncRead + Unpin), op: u64, frontier: u64)
    -> Result<(), WireError> {
    loop {
        if let Message::Credit { op: found, accepted_event_seq, .. } = next(input).await? {
            if found == op && accepted_event_seq >= frontier { return Ok(()); }
        }
    }
}
async fn wait_loss(input: &mut (impl AsyncRead + Unpin)) -> Result<(), WireError> {
    while next(input).await.is_ok() {}
    Ok(())
}
async fn data(output: &mut (impl AsyncWrite + Unpin), op: u64, seq: u64,
    event: wire::DataKind, bytes: &[u8], last: bool) -> Result<(), WireError> {
    send(output, Message::Data { op, seq, event, bytes: wire::Segment::new(bytes.to_vec())?, last }).await
}
async fn run(input: &mut (impl AsyncRead + Unpin), output: &mut (impl AsyncWrite + Unpin))
    -> Result<(), WireError> {
    send(output, Message::Hello { wire: 1, build: wire::Text::new("fault-fixture".into())?,
        routes: vec![wire::Route::FreeResponses] }).await?;
    if !matches!(next(input).await?, Message::Ready { generation } if generation != 0) {
        return Err(WireError::Protocol);
    }
    let Message::Start { op, model, .. } = next(input).await? else { return Err(WireError::Protocol); };
    inject(input, output, op, model.as_str()).await
}
async fn inject(input: &mut (impl AsyncRead + Unpin), output: &mut (impl AsyncWrite + Unpin),
    op: u64, mode: &str) -> Result<(), WireError> {
    if mode == "fixture-before-head" { return Ok(()); }
    if mode == "fixture-oversize" {
        output.write_all(&((framer_worker_wire::FRAME_LIMIT + 1) as u32).to_be_bytes()).await?;
        return wait_loss(input).await;
    }
    if mode == "fixture-duplicate" {
        let bytes = format!("{{\"kind\":\"head\",\"op\":{op},\"op\":{op},\"status\":200,\"content_type\":\"text/event-stream\"}}");
        output.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
        output.write_all(bytes.as_bytes()).await?;
        return wait_loss(input).await;
    }
    if mode == "fixture-unheaded" {
        data(output, op, 0, wire::DataKind::Opening, b"", true).await?;
        return wait_loss(input).await;
    }
    let head_op = if mode == "fixture-foreign-op" { op + 1 } else { op };
    send(output, Message::Head { op: head_op, status: 200,
        content_type: wire::Text::new("text/event-stream".into())? }).await?;
    inject_after_head(input, output, op, mode).await
}
async fn inject_after_head(input: &mut (impl AsyncRead + Unpin),
    output: &mut (impl AsyncWrite + Unpin), op: u64, mode: &str) -> Result<(), WireError> {
    use wire::DataKind::{Opening, Content};
    match mode {
        "fixture-foreign-op" => wait_loss(input).await,
        "fixture-over-credit" => {
            data(output, op, 0, Opening, b"", true).await?;
            data(output, op, 1, Opening, b"", true).await?;
            data(output, op, 2, Content, b"", true).await?;
            wait_loss(input).await
        }
        "fixture-mid-event" => {
            data(output, op, 0, Opening, b"event: response.created\ndata: {", false).await?;
            accepted(input, op, 0).await
        }
        "fixture-boundary" => {
            data(output, op, 0, Opening, b"event: response.created\ndata: {}\n\n", true).await?;
            accepted(input, op, 1).await
        }
        "fixture-after-terminal" => after_terminal(input, output, op).await,
        "fixture-pending-forward" => pending_forward(input, output, op).await,
        _ => Err(WireError::Protocol),
    }
}
async fn invoke_number(input: &mut (impl AsyncRead + Unpin)) -> Result<u32, WireError> {
    loop {
        if let Message::Invoke { n, request, .. } = next(input).await? {
            if request.last { return Ok(n); }
        }
    }
}
async fn forward(output: &mut (impl AsyncWrite + Unpin), op: u64, n: u32) -> Result<(), WireError> {
    send(output, Message::Forward { op, n, fwd: 1, dialect: wire::Dialect::OpenRouterResponses,
        step: wire::Text::new("translate".into())?, step_index: 0,
        payload: wire::Part { offset: 0, last: true, bytes: wire::Segment::new(b"{}".to_vec())? } }).await
}
async fn pending_forward(input: &mut (impl AsyncRead + Unpin),
    output: &mut (impl AsyncWrite + Unpin), op: u64) -> Result<(), WireError> {
    let n = invoke_number(input).await?;
    forward(output, op, n).await?;
    loop {
        if matches!(next(input).await?, Message::Cancel { op: found } if found == op) {
            return Ok(());
        }
    }
}
async fn after_terminal(input: &mut (impl AsyncRead + Unpin),
    output: &mut (impl AsyncWrite + Unpin), op: u64) -> Result<(), WireError> {
    let n = invoke_number(input).await?;
    forward(output, op, n).await?;
    loop {
        if matches!(next(input).await?, Message::ForwardReply { body, .. } if body.last) { break; }
    }
    send(output, Message::Result { op, n, diag_dropped: 0, outcome: wire::Outcome::Success {
        summary: wire::Summary { chunks: 1, metered_chunks: 0, input_tokens: 1, output_tokens: 1,
            finish: wire::FinishReason::Completed } } }).await?;
    loop {
        if matches!(next(input).await?, Message::Finish { .. }) { break; }
    }
    data(output, op, 0, wire::DataKind::Opening, b"event: response.created\ndata: {}\n\n", true).await?;
    data(output, op, 1, wire::DataKind::Opening, b"event: response.in_progress\ndata: {}\n\n", true).await?;
    let mut disposed = 0;
    while disposed < 2 {
        if let Message::Credit { returned, .. } = next(input).await? { disposed += returned.slots; }
    }
    data(output, op, 2, wire::DataKind::Terminal, b"event: response.completed\ndata: {}\n\n", true).await?;
    accepted(input, op, 3).await
}

#[cfg(unix)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    use std::os::fd::{FromRawFd, OwnedFd};
    use tokio::net::unix::pipe::{Receiver, Sender};
    let input = Receiver::from_owned_fd(unsafe { OwnedFd::from_raw_fd(0) });
    let output = Sender::from_owned_fd(unsafe { OwnedFd::from_raw_fd(1) });
    match (input, output) {
        (Ok(mut input), Ok(mut output)) => {
            if run(&mut input, &mut output).await.is_ok() { std::process::ExitCode::SUCCESS }
            else { std::process::ExitCode::FAILURE }
        }
        _ => std::process::ExitCode::FAILURE,
    }
}
#[cfg(not(unix))]
fn main() -> std::process::ExitCode { std::process::ExitCode::FAILURE }
