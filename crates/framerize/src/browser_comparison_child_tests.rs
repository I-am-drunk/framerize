use super::*;
use framer_core::ids::AccountId;
use pair::ComparisonWindow;
use tokio::{io::AsyncWriteExt, net::UnixStream};

fn wire() -> serde_json::Value {
    json!({"version":1,"nonce":"a".repeat(32),"account":AccountId::new(),
        "user":"fixture-user","team":"fixture-team","project":"fixture-project",
        "chat_session":"fixture-session","route":"responses",
        "payload":{"model":"fixture-model","stream":false,"input":"fixture"},
        "context":{"feature_category":"fixture","feature_name":"fixture","step":"fixture",
            "turn_id":"native-turn","user_prompt":"fixture"},"baseline_turn":"browser-turn",
        "token_limit":32,"timeout_ms":30_000,"proxy":null,
        "state_parent":"/unused-fixture","host_script":"/unused-fixture/host.py"})
}

#[tokio::test]
async fn private_plan_rejects_unbounded_and_ambiguous_requests() {
    let parse = |v| serde_json::from_value::<plan::WirePlan>(v).unwrap().specification();
    assert!(parse(wire()).is_ok());
    for (field,value) in [("timeout_ms",json!(60_001)),("route",json!("unknown")),
        ("baseline_turn",json!("native-turn")),("nonce",json!("bad")),
        ("token_limit",json!(513))] {
        let mut v=wire();v[field]=value;assert!(parse(v).is_err());
    }
    let mut v=wire();v["payload"]["stream"]=json!(true);assert!(parse(v).is_err());
    let mut v=wire();v["unexpected"]=json!(true);
    assert!(serde_json::from_value::<plan::WirePlan>(v).is_err());
}

#[tokio::test]
async fn private_frames_require_complete_bounded_json() {
    for (size,body) in [(0u32,b"".as_slice()),(98_305,b"".as_slice()),(3,b"{}".as_slice())] {
        let (a,mut b)=UnixStream::pair().unwrap();
        b.write_u32(size).await.unwrap();b.write_all(body).await.unwrap();
        b.shutdown().await.unwrap();
        assert!(ipc::Channel(a).receive::<serde_json::Value>().await.is_err());
    }
    let (a,b)=UnixStream::pair().unwrap();
    ipc::Channel(b).send(&json!({"fixture":true})).await.unwrap();
    assert_eq!(ipc::Channel(a).receive::<serde_json::Value>().await.unwrap(),json!({"fixture":true}));
    assert!(ipc::Channel::inherited(-1).is_err());
}

fn parent_window(channel: ipc::Channel) -> ParentWindow {
    let plan: plan::WirePlan=serde_json::from_value(wire()).unwrap();
    let spec=plan.specification().unwrap();
    ParentWindow { nonce:plan.nonce,selected:Mutex::new(Some(Selection {
        identity:spec.identity,context:spec.context,proxy:None })),
        store:Arc::new(PrivateStore(Mutex::new(None))),channel:tokio::sync::Mutex::new(channel) }
}

#[tokio::test]
async fn parent_nonce_refusal_cannot_populate_private_store() {
    let (a,b)=UnixStream::pair().unwrap();let window=parent_window(ipc::Channel(a));
    let peer=async {
        let mut channel=ipc::Channel(b);
        let ready:serde_json::Value=channel.receive().await.unwrap();assert_eq!(ready["command"],"ready");
        channel.send(&json!({"command":"start","nonce":"wrong","session_cookie":"session=private-fixture"})).await.unwrap();
    };
    let (result,())=tokio::join!(window.admit(),peer);
    assert!(matches!(result,Err(Failure::Admission)));
    assert!(window.store.0.lock().unwrap().is_none());
}

#[tokio::test]
async fn parent_admission_and_session_are_consumed_once() {
    let (a,b)=UnixStream::pair().unwrap();let window=parent_window(ipc::Channel(a));
    let peer=async {
        let mut channel=ipc::Channel(b);
        let ready:serde_json::Value=channel.receive().await.unwrap();
        channel.send(&json!({"command":"start","nonce":ready["nonce"],
            "session_cookie":"session=private-fixture"})).await.unwrap();
    };
    let (result,())=tokio::join!(window.admit(),peer);let selected=result.unwrap();
    assert!(matches!(window.admit().await,Err(Failure::Admission)));
    let session=window.store.load(&selected.identity.user).await.unwrap().unwrap();
    assert_eq!(session.cookie_header,"session=private-fixture");
    assert!(window.store.load(&selected.identity.user).await.is_err());
    assert!(window.store.save(&session).await.is_err());
    assert!(window.store.delete(&selected.identity.user).await.is_err());
}

#[tokio::test]
async fn result_requires_matching_parent_commit_acknowledgement() {
    for nonce in ["wrong".to_owned(),"a".repeat(32)] {
        let expected=nonce.len()==32;
        let (a,b)=UnixStream::pair().unwrap();let window=parent_window(ipc::Channel(a));
        let value=pair::Report { failure:Some(Failure::Dispatch),..pair::Report::default() };
        let peer=async {
            let mut channel=ipc::Channel(b);
            let v:serde_json::Value=channel.receive().await.unwrap();
            assert_eq!(v["command"],"result");assert_eq!(v["report"]["failure"],"Dispatch");
            assert!(v["report"].get("session_cookie").is_none());
            channel.send(&json!({"command":"recorded","nonce":nonce})).await.unwrap();
        };
        let (result,())=tokio::join!(window.record(&value),peer);assert_eq!(result.is_ok(),expected);
        assert!(!locally_complete(&value));
    }
}
