use super::*;
use crate::message::Message;

#[tokio::test]
async fn fragmented_transport_preserves_adjacent_complete_frames() {
    let (mut writer, mut reader) = tokio::io::duplex(1);
    let first = encode(&Message::Ready { generation: 7 }).unwrap();
    let second = encode(&Message::Ready { generation: 8 }).unwrap();
    assert_eq!(first.capacity(), first.len());
    assert_eq!(second.capacity(), second.len());
    let send = async {
        write(&mut writer, &first).await.unwrap();
        write(&mut writer, &second).await.unwrap();
        writer.shutdown().await.unwrap();
    };
    let receive = async {
        for (generation, bytes) in [(7, first.len()), (8, second.len())] {
            let frame = read::<_, Message>(&mut reader).await.unwrap().unwrap();
            assert_eq!(frame.message, Message::Ready { generation });
            assert_eq!(frame.wire_bytes, bytes as u64);
        }
        assert!(read::<_, Message>(&mut reader).await.unwrap().is_none());
    };
    tokio::join!(send, receive);
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Nested { #[allow(dead_code)] next: Option<Box<Nested>> }

#[tokio::test]
async fn recursion_limit_stays_enabled() {
    let json = format!("{}null{}", "{\"next\":".repeat(160), "}".repeat(160));
    let input = framed(json.as_bytes());
    assert!(matches!(read::<_, Nested>(&mut input.as_slice()).await, Err(WireError::Json(_))));
}

#[tokio::test]
async fn length_is_rejected_before_any_payload_is_read() {
    let mut bytes = ((FRAME_LIMIT + 1) as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(b"unread");
    let mut input = bytes.as_slice();
    assert!(matches!(read::<_, Message>(&mut input).await, Err(WireError::FrameLimit)));
    assert_eq!(input, b"unread");
    assert!(matches!(read::<_, Message>(&mut [0u8; 4].as_slice()).await,
        Err(WireError::FrameLimit)));
    let mut truncated = 10u32.to_be_bytes().to_vec();
    truncated.extend_from_slice(b"abc");
    assert!(matches!(read::<_, Message>(&mut truncated.as_slice()).await,
        Err(WireError::Io(_))));
}

#[test]
fn encoding_refuses_an_oversized_frame() {
    let message = Message::Hello {
        wire: 1, build: "x".repeat(FRAME_LIMIT), routes: vec![],
    };
    assert!(matches!(encode(&message), Err(WireError::FrameLimit)));
}

fn framed(json: &[u8]) -> Vec<u8> {
    let mut bytes = (json.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(json);
    bytes
}

#[tokio::test]
async fn strict_dtos_reject_duplicate_unknown_and_trailing_input() {
    for json in [
        r#"{"kind":"ready","generation":1,"generation":2}"#,
        r#"{"kind":"ready","kind":"ready","generation":1}"#,
        r#"{"kind":"ready","generation":1,"extra":0}"#,
        r#"{"kind":"unknown","generation":1}"#,
        r#"{"kind":"ready","generation":1} {}"#,
        r#"{"kind":"credit","op":1,"accepted_event_seq":0,"returned":{"wire_bytes":0,"slots":0,"slots":1,"downstream_bytes":0}}"#,
    ] {
        let input = framed(json.as_bytes());
        assert!(read::<_, Message>(&mut input.as_slice()).await.is_err());
    }
}

#[tokio::test]
async fn clean_eof_is_distinct_from_a_truncated_frame() {
    assert!(read::<_, Message>(&mut [].as_slice()).await.unwrap().is_none());
    for input in [vec![0], vec![0, 0, 0], framed(b"{\"kind\":")] {
        assert!(read::<_, Message>(&mut input.as_slice()).await.is_err());
    }
}
