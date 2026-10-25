//! Reversible availability only. Routing economics cannot be overridden.
use crate::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Decode retired wire fields only to explicitly reject old-client mutations.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switch_at: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switch_at_short: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switch_at_weekly: Option<serde_json::Value>,
}
impl RoutingArgs {
    pub fn validate(&self) -> Result<()> {
        if [
            &self.priority,
            &self.switch_at,
            &self.switch_at_short,
            &self.switch_at_weekly,
        ]
        .iter()
        .any(|v| v.is_some())
        {
            return Err("Account routing is automatic; manual priorities and switching rules are no longer supported".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preference {
    pub enabled: bool,
    pub locked: bool,
}
impl Default for Preference {
    fn default() -> Self {
        Self {
            enabled: true,
            locked: false,
        }
    }
}
pub fn effective(conn: &Connection, user: Option<i64>, account: i64) -> Result<Preference> {
    Ok(conn.query_row("SELECT COALESCE(p.enabled,1) AND CASE WHEN COALESCE(p.locked,0)=1 THEN 1 ELSE COALESCE(u.enabled,1) END,COALESCE(p.locked,0) FROM oauth_accounts a LEFT JOIN account_policy p ON p.account_id=a.id LEFT JOIN account_preferences u ON u.account_id=a.id AND u.user_id=?1 WHERE a.id=?2",params![user,account],|r|Ok(Preference{enabled:r.get(0)?,locked:r.get(1)?}))?)
}
pub fn set_user(
    conn: &mut Connection,
    user: i64,
    account: i64,
    enabled: Option<bool>,
    priority: Option<i32>,
) -> Result<Preference> {
    set_user_rules(
        conn,
        user,
        account,
        enabled,
        priority,
        &RoutingArgs::default(),
    )
}
pub fn set_user_rules(
    conn: &mut Connection,
    user: i64,
    account: i64,
    enabled: Option<bool>,
    priority: Option<i32>,
    patch: &RoutingArgs,
) -> Result<Preference> {
    patch.validate()?;
    if priority.is_some() {
        return Err("Account routing is automatic; manual priority is no longer supported".into());
    }
    let tx = conn.transaction()?;
    if effective(&tx, Some(user), account)?.locked {
        return Err("Account settings are locked by the server operator".into());
    }
    if let Some(enabled) = enabled {
        tx.execute("INSERT INTO account_preferences(user_id,account_id,enabled) VALUES(?1,?2,?3) ON CONFLICT(user_id,account_id) DO UPDATE SET enabled=excluded.enabled",params![user,account,enabled])?;
    }
    let result = effective(&tx, Some(user), account)?;
    tx.commit()?;
    Ok(result)
}
pub fn set_policy(
    conn: &mut Connection,
    account: i64,
    enabled: Option<bool>,
    priority: Option<i32>,
    locked: Option<bool>,
) -> Result<Preference> {
    set_policy_rules(
        conn,
        account,
        enabled,
        priority,
        locked,
        &RoutingArgs::default(),
    )
}
pub fn set_policy_rules(
    conn: &mut Connection,
    account: i64,
    enabled: Option<bool>,
    priority: Option<i32>,
    locked: Option<bool>,
    patch: &RoutingArgs,
) -> Result<Preference> {
    patch.validate()?;
    if priority.is_some() {
        return Err("Account routing is automatic; manual priority is no longer supported".into());
    }
    let tx = conn.transaction()?;
    let previous = effective(&tx, None, account)?;
    tx.execute("INSERT INTO account_policy(account_id,enabled,locked) VALUES(?1,?2,?3) ON CONFLICT(account_id) DO UPDATE SET enabled=excluded.enabled,locked=excluded.locked",params![account,enabled.unwrap_or(previous.enabled),locked.unwrap_or(previous.locked)])?;
    let result = effective(&tx, None, account)?;
    tx.commit()?;
    Ok(result)
}
pub fn annotate(conn: &Connection, user: i64, value: &mut serde_json::Value) -> Result<()> {
    value["capabilities"]["automatic_account_routing"] = serde_json::json!(1);
    value["capabilities"]["automatic_reset_credits"] = serde_json::json!(1);
    let now = chrono::Utc::now().timestamp();
    if let Some(rows) = value["quota_accounts"].as_array_mut() {
        for row in rows {
            if let Some(id) = row["id"].as_i64() {
                row["preference"] = serde_json::to_value(effective(conn, Some(user), id)?)?;
                let routing = crate::routing::decision(conn, id, now)?;
                row["threshold_reached"] =
                    serde_json::json!(routing.reached(&crate::quota::summary(conn, id, now)?));
                row["routing"] = serde_json::to_value(routing)?;
                let status:Option<String>=conn.query_row("SELECT status FROM automatic_reset_attempts WHERE account_id=?1 ORDER BY attempted_at DESC LIMIT 1",[id],|r|r.get(0)).ok();
                row["automatic_reset"] =
                    serde_json::json!({"status":status.unwrap_or_else(||"idle".into())});
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_routing_cannot_be_overridden_but_availability_and_locks_survive() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::store::init(&conn).unwrap();
        crate::create_user(&conn, "synthetic").unwrap();
        conn.execute("INSERT INTO oauth_accounts(account_id,state,encrypted_credentials,expires_at,created_at) VALUES('synthetic','active',X'00',9999,0)",[]).unwrap();
        conn.execute(
            "INSERT INTO account_policy(account_id,priority,switch_at) VALUES(1,255,100)",
            [],
        )
        .unwrap();
        assert!(effective(&conn, Some(1), 1).unwrap().enabled);
        assert!(set_user(&mut conn, 1, 1, None, Some(10)).is_err());
        let legacy: RoutingArgs =
            serde_json::from_value(serde_json::json!({"switch_at":20})).unwrap();
        assert!(set_policy_rules(&mut conn, 1, None, None, None, &legacy).is_err());
        assert!(
            !set_user(&mut conn, 1, 1, Some(false), None)
                .unwrap()
                .enabled
        );
        assert!(
            set_policy(&mut conn, 1, Some(true), None, Some(true))
                .unwrap()
                .locked
        );
        assert!(effective(&conn, Some(1), 1).unwrap().enabled);
        assert!(set_user(&mut conn, 1, 1, Some(false), None).is_err());
        set_policy(&mut conn, 1, None, None, Some(false)).unwrap();
        assert!(!effective(&conn, Some(1), 1).unwrap().enabled);
    }
}
