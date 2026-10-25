use super::*;
use clap::Parser;

#[derive(Parser)]
struct TestPoolCli {
    #[command(flatten)]
    pool: PoolArgs,
}

#[test]
fn capacity_parser_accepts_both_server_boundaries() {
    for (raw, expected) in [("1", 1), ("64", 64)] {
        let parsed = TestPoolCli::try_parse_from(["pool", "configure", "--active-capacity", raw]).unwrap();
        let (_, _, body) = request_for(parsed.pool.command.unwrap()).unwrap();
        assert_eq!(body, Some(json!({"active_capacity": expected})));
    }
}

#[test]
fn capacity_parser_rejects_values_outside_server_range() {
    for raw in ["0", "65", "4294967295"] {
        assert!(TestPoolCli::try_parse_from(["pool", "configure", "--active-capacity", raw]).is_err());
    }
}

#[test]
fn capacity_only_does_not_replace_eligibility() {
    let request = request_for(PoolCommand::Configure { active_capacity: Some(6), eligible: None }).unwrap();
    assert_eq!(request, (Method::POST, "/api/pool/config", Some(json!({"active_capacity": 6}))));
}

#[test]
fn eligibility_only_does_not_replace_capacity() {
    let request = request_for(PoolCommand::Configure { active_capacity: None, eligible: Some("all".into()) }).unwrap();
    assert_eq!(request, (Method::POST, "/api/pool/config", Some(json!({"eligible_accounts": "all"}))));
}

#[test]
fn explicit_selection_and_capacity_share_one_patch() {
    let id = "00000000-0000-0000-0000-000000000001";
    let (_, _, body) = request_for(PoolCommand::Configure {
        active_capacity: Some(6), eligible: Some(id.into()),
    }).unwrap();
    assert_eq!(body, Some(json!({"active_capacity": 6, "eligible_accounts": [id]})));
}

#[test]
fn none_is_an_explicit_empty_selection() {
    assert_eq!(config_patch(None, Some("none")).unwrap(), json!({"eligible_accounts": []}));
}

#[test]
fn blank_eligibility_cannot_silently_pause_admission() {
    for raw in ["", " ", "\t\r\n"] {
        let error = config_patch(None, Some(raw)).unwrap_err();
        assert!(error.contains("use none to pause admission"));
    }
}

#[test]
fn empty_configure_is_rejected_before_request() {
    assert!(request_for(PoolCommand::Configure { active_capacity: None, eligible: None }).is_err());
}

#[test]
fn duplicate_accounts_are_rejected_before_request() {
    let id = "00000000-0000-0000-0000-000000000001";
    assert!(config_patch(None, Some(&format!("{id},{id}"))).is_err());
}

#[test]
fn account_operations_send_only_the_account_identity() {
    let id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
    for (command, path) in [
        (PoolCommand::Activate { account_id: id }, "/api/pool/activate"),
        (PoolCommand::Park { account_id: id }, "/api/pool/park"),
    ] {
        assert_eq!(request_for(command).unwrap(),
            (Method::POST, path, Some(json!({"account_id": id.to_string()}))));
    }
}
