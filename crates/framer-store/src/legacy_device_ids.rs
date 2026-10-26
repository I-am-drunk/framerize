//! Persist the legacy device default before any account can be read.
use framer_core::account::ManagedAccount;
use rusqlite::{types::Value as SqlValue, Connection, OptionalExtension, TransactionBehavior};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

pub(super) fn persist(conn: &mut Connection) -> Result<(), String> {
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    {
        let mut select = transaction.prepare(
            "SELECT rowid, json FROM accounts WHERE ?1 IS NULL OR rowid > ?1
             ORDER BY rowid LIMIT 1",
        ).map_err(|error| error.to_string())?;
        let mut previous: Option<i64> = None;
        loop {
            // Finish each SELECT before updating its row. Retain only one record.
            let row = select.query_row([previous], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, SqlValue>(1)?))
            }).optional().map_err(|error| error.to_string())?;
            let Some((rowid, value)) = row else { break };
            previous = Some(rowid);
            let SqlValue::Text(json) = value else { continue };
            if let Some(updated) = with_default(&json)? {
                let changed = transaction.execute("UPDATE accounts SET json = ?1 WHERE rowid = ?2",
                    rusqlite::params![updated, rowid]).map_err(|error| error.to_string())?;
                if changed != 1 {
                    return Err("legacy device identity was not persisted".into());
                }
            }
        }
    }
    transaction.commit().map_err(|error| error.to_string())
}

fn with_default(json: &str) -> Result<Option<String>, String> {
    let Ok(fields) = serde_json::from_str::<BTreeMap<String, &RawValue>>(json) else {
        return Ok(None);
    };
    if fields.contains_key("castle_device_id") {
        return Ok(None);
    }
    // Invalid records retain their existing get/list error behavior.
    let Ok(account) = serde_json::from_str::<ManagedAccount>(json) else {
        return Ok(None);
    };
    let device = serde_json::to_string(&account.castle_device_id)
        .map_err(|error| error.to_string())?;
    let end = json.trim_end().len() - 1;
    // Keep every original field byte intact, including unknown numeric values.
    Ok(Some(format!("{},\"castle_device_id\":{}{}", &json[..end], device, &json[end..])))
}
