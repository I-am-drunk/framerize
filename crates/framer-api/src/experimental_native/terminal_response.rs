//! Source-only terminal Responses reducer. Raw client SSE uses byte passthrough.
use super::consumer::Reducer;
use super::sse_decoder::Event;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition { Completed, Incomplete, Failed, ErrorEvent }

/// Complete response object, or complete top-level event for ErrorEvent.
/// No field allowlist, text flattening or model substitution.
pub struct Terminal {
    pub disposition: Disposition,
    pub value: Value,
}

impl std::fmt::Debug for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminal").field("disposition", &self.disposition)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Encoded bytes of one event, checked before parsing. JSON allocation
    /// overhead is additional; this is not a physical RAM limit.
    pub event_bytes: usize,
    pub events: u64,
    pub identity_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    InvalidLimits, EventBytes, EventCount, Json, EventShape, EventName,
    ResponseShape, ResponseIdentity, TerminalStatus, InvalidCompleted,
    MissingTerminal, PrematureDone, AfterDone, AfterTerminal,
}

pub struct ResponsesReducer {
    limits: Limits,
    events: u64,
    identity: Option<String>,
    terminal: Option<Terminal>,
    retained_wire_bytes: usize,
    done: bool,
    failure: Option<Failure>,
}

impl ResponsesReducer {
    pub fn new(limits: Limits) -> Result<Self, Failure> {
        if limits.event_bytes == 0 || limits.events == 0 || limits.identity_bytes == 0
            || limits.identity_bytes > limits.event_bytes
        {
            return Err(Failure::InvalidLimits);
        }
        Ok(Self {
            limits, events: 0, identity: None, terminal: None,
            retained_wire_bytes: 0, done: false, failure: None,
        })
    }

    /// Encoded event bytes represented by the retained terminal, not heap size.
    pub fn retained_wire_bytes(&self) -> usize { self.retained_wire_bytes }
    pub fn events(&self) -> u64 { self.events }

    fn bind_id(&mut self, value: &Value) -> Result<(), Failure> {
        let id = value.as_str().filter(|id| !id.is_empty()
            && id.len() <= self.limits.identity_bytes).ok_or(Failure::ResponseIdentity)?;
        match &self.identity {
            Some(expected) if expected != id => Err(Failure::ResponseIdentity),
            Some(_) => Ok(()),
            None => { self.identity = Some(id.to_owned()); Ok(()) }
        }
    }

    fn take_terminal(&mut self, value: &mut Value, disposition: Disposition)
        -> Result<Value, Failure>
    {
        if disposition == Disposition::ErrorEvent { return Ok(value.take()); }
        let response = value.as_object_mut().and_then(|v| v.remove("response"))
            .filter(Value::is_object).ok_or(Failure::ResponseShape)?;
        self.bind_id(response.get("id").ok_or(Failure::ResponseIdentity)?)?;
        if !response.get("output").is_some_and(Value::is_array) {
            return Err(Failure::ResponseShape);
        }
        let expected = match disposition {
            Disposition::Completed => "completed",
            Disposition::Incomplete => "incomplete",
            Disposition::Failed => "failed",
            Disposition::ErrorEvent => unreachable!(),
        };
        if response.get("status").and_then(Value::as_str) != Some(expected) {
            return Err(Failure::TerminalStatus);
        }
        if disposition == Disposition::Completed
            && (!response.get("error").is_none_or(Value::is_null)
                || !response.get("incomplete_details").is_none_or(Value::is_null))
        {
            return Err(Failure::InvalidCompleted);
        }
        Ok(response)
    }

    fn on_event(&mut self, event: Event<'_>) -> Result<(), Failure> {
        if self.events >= self.limits.events { return Err(Failure::EventCount); }
        self.events += 1;
        if event.data.len() > self.limits.event_bytes { return Err(Failure::EventBytes); }
        if self.done { return Err(Failure::AfterDone); }
        if (event.data.is_empty() || event.data == "[DONE]") && event.event != "message" {
            return Err(Failure::EventName);
        }
        // Empty keepalives consume event budget but retain no payload.
        if event.data.is_empty() { return Ok(()); }
        if event.data == "[DONE]" {
            if self.terminal.is_none() { return Err(Failure::PrematureDone); }
            self.done = true;
            return Ok(());
        }
        if self.terminal.is_some() { return Err(Failure::AfterTerminal); }
        let mut value: Value = serde_json::from_str(event.data).map_err(|_| Failure::Json)?;
        let kind = value.get("type").and_then(Value::as_str)
            .filter(|kind| !kind.is_empty()).ok_or(Failure::EventShape)?;
        if event.event != "message" && event.event != kind { return Err(Failure::EventName); }
        if let Some(id) = value.get("response_id") { self.bind_id(id)?; }
        let disposition = match kind {
            "response.created" | "response.in_progress" => {
                let response = value.get("response").filter(|v| v.is_object())
                    .ok_or(Failure::ResponseShape)?;
                self.bind_id(response.get("id").ok_or(Failure::ResponseIdentity)?)?;
                return Ok(());
            }
            "response.completed" => Disposition::Completed,
            "response.incomplete" => Disposition::Incomplete,
            "response.failed" => Disposition::Failed,
            "error" => Disposition::ErrorEvent,
            // A terminal-only reducer does not retain or reserialize deltas.
            _ => return Ok(()),
        };
        let result = self.take_terminal(&mut value, disposition)?;
        self.terminal = Some(Terminal { disposition, value: result });
        self.retained_wire_bytes = event.data.len();
        Ok(())
    }
}

impl Reducer for ResponsesReducer {
    type Output = Terminal;
    type Error = Failure;

    fn event(&mut self, event: Event<'_>) -> Result<(), Failure> {
        if let Some(error) = self.failure { return Err(error); }
        let result = self.on_event(event);
        if let Err(error) = result {
            // Failure is sticky. A caller cannot ignore an error, then obtain
            // the earlier completed result via finish().
            self.failure = Some(error);
            self.identity = None;
            self.terminal = None;
            self.retained_wire_bytes = 0;
        }
        result
    }

    fn finish(mut self) -> Result<Terminal, Failure> {
        if let Some(error) = self.failure { return Err(error); }
        self.terminal.take().ok_or(Failure::MissingTerminal)
    }
}
