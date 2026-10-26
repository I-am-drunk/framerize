use serde::{Deserialize, Serialize};
use crate::credit::Capacity;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route { FreeResponses }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Message {
    Hello { wire: u16, build: String, routes: Vec<Route> },
    Ready { generation: u64 },
    Credit {
        op: u64,
        returned: Capacity,
        /// Exclusive frontier. Zero means that no event has been accepted.
        accepted_event_seq: u64,
    },
}
