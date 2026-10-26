use super::*;
use std::time::Duration;
use tokio::time::timeout;

async fn evaluate(reply: Value, expression: String, long: bool) -> Result<Value, String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let expected = expression.clone();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let frame = socket.next().await.unwrap().unwrap();
        let command: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
        assert_eq!(command["method"], "Runtime.evaluate");
        assert_eq!(command["params"]["expression"], expected);
        assert_eq!(command["params"]["awaitPromise"], true);
        assert_eq!(command["params"]["returnByValue"], true);
        socket.send(Message::Text(json!({"id": command["id"], "result": reply})
            .to_string().into())).await.unwrap();
        assert!(socket.next().await.unwrap().unwrap().is_close());
    });
    let (ws, _) = connect_async(format!("ws://{address}")).await.unwrap();
    let mut instance = CdpInstance { child: None, ws, next_id: 0 };
    let result = timeout(Duration::from_secs(2), async {
        if long { instance.eval_long(expression).await }
        else { instance.eval(expression).await }
    }).await;
    let closed = timeout(Duration::from_secs(2), instance.ws.close(None)).await;
    let joined = timeout(Duration::from_secs(2), peer).await;
    closed.unwrap().unwrap();
    joined.unwrap().unwrap();
    result.unwrap()
}

fn exception_reply() -> Value {
    let error = json!({"type": "object", "subtype": "error",
        "description": "synthetic-cookie-and-expression-must-stay-private"});
    json!({"result": error, "exceptionDetails": {
        "exceptionId": 1, "text": "Uncaught", "lineNumber": 0,
        "columnNumber": 0, "exception": error
    }})
}

#[tokio::test]
async fn device_seed_eval_propagates_javascript_exception_without_payload() {
    let error = evaluate(exception_reply(), seed_device_snippet("fixture-device"), false)
        .await.unwrap_err();
    assert_eq!(error, "cdp JavaScript evaluation failed");
}

#[tokio::test]
async fn long_eval_rejects_javascript_exception_even_with_a_value() {
    let mut reply = exception_reply();
    reply["result"]["value"] = json!({"status": 200, "body": "untrusted partial value"});
    let error = evaluate(reply, "(async () => { throw new Error('fixture'); })()".into(), true)
        .await.unwrap_err();
    assert_eq!(error, "cdp JavaScript evaluation failed");
}

#[tokio::test]
async fn successful_values_and_undefined_preserve_existing_results() {
    let cases = [
        (json!({"type":"string", "value":"fixture-device"}), json!("fixture-device")),
        (json!({"type":"object", "value":{"body":"complete"}}), json!({"body":"complete"})),
        (json!({"type":"object", "subtype":"null", "value":null}), Value::Null),
        (json!({"type":"undefined"}), Value::Null),
    ];
    for long in [false, true] {
        for (index, (remote, expected)) in cases.iter().enumerate() {
            let mut reply = json!({"result": remote});
            if index == 3 { reply["exceptionDetails"] = Value::Null; }
            let value = evaluate(reply, "fixture()".into(), long).await.unwrap();
            assert_eq!(&value, expected);
        }
    }
}
