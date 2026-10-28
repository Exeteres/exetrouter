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
        let mut candidates = Vec::new();
        // Scan activity once per snapshot, not once per account.
        let mut clients = crate::usage::clients_by_account(conn, user, now)?;
        for row in rows.iter_mut() {
            if let Some(id) = row["id"].as_i64() {
                let preference = effective(conn, Some(user), id)?;
                let quota = crate::quota::summary(conn, id, now)?;
                row["preference"] = serde_json::to_value(&preference)?;
                let routing = crate::routing::decision(conn, id, now)?;
                let reached = routing.reached(&quota);
                let available = preference.enabled && !reached && quota.cooldown_until.is_none()
                    && conn.query_row("SELECT a.state='active' AND a.expires_at>?2+60 AND a.catalog_updated_at IS NOT NULL AND a.catalog_updated_at<=?2 AND a.catalog_updated_at>?2-60 AND EXISTS(SELECT 1 FROM oauth_models m WHERE m.account_id=a.id) AND NOT EXISTS(SELECT 1 FROM oauth_health h WHERE h.account_id=a.id AND h.generation=a.generation AND h.retry_at>?2) FROM oauth_accounts a WHERE a.id=?1",params![id,now],|r|r.get::<_,bool>(0))?;
                if available {
                    candidates.push(crate::pool::Candidate {
                        id,
                        quota: if quota.status == "observed" {
                            quota
                                .windows
                                .iter()
                                .map(|w| w.used_percent)
                                .reduce(f64::max)
                        } else {
                            None
                        },
                        routing: routing.clone(),
                    });
                }
                row["threshold_reached"] = serde_json::json!(reached);
                row["routing"] = serde_json::to_value(routing)?;
                row["routing"]["best"] = false.into();
                row["routing"]["observed_at"] = now.into();
                row["routing"]["valid_until"] = quota
                    .windows
                    .iter()
                    .fold(now.saturating_add(60), |until, window| {
                        until
                            .min(window.observed_at.saturating_add(60))
                            .min(window.reset_at.unwrap_or(i64::MAX))
                    })
                    .into();
                row["routing"]["target_share_percent"] = serde_json::Value::Null;
                row["clients"] = clients
                    .remove(&id)
                    .unwrap_or_else(|| crate::usage::empty_clients(now));
                let status:Option<String>=conn.query_row("SELECT status FROM automatic_reset_attempts WHERE account_id=?1 ORDER BY attempted_at DESC LIMIT 1",[id],|r|r.get(0)).ok();
                row["automatic_reset"] =
                    serde_json::json!({"status":status.unwrap_or_else(||"idle".into())});
            }
        }
        let recommendations = crate::pool::recommendations(&candidates);
        for row in rows {
            if let Some(recommendation) = recommendations
                .iter()
                .find(|r| row["id"].as_i64() == Some(r.id))
            {
                row["routing"]["best"] = recommendation.best.into();
                row["routing"]["target_share_percent"] = serde_json::json!(recommendation.share);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn best_and_shares_follow_fresh_capacity_and_personal_availability() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::store::init(&conn).unwrap();
        let user = crate::create_user(&conn, "synthetic").unwrap();
        let now = chrono::Utc::now().timestamp();
        for (id, used, days) in [(1, 40.0, 6), (2, 20.0, 4)] {
            conn.execute("INSERT INTO oauth_accounts(id,account_id,state,encrypted_credentials,expires_at,catalog_updated_at,created_at) VALUES(?1,?2,'active',X'00',?3,?4,0)",params![id,format!("synthetic-{id}"),now+3600,now]).unwrap();
            conn.execute("INSERT INTO oauth_models(account_id,model,display_name) VALUES(?1,'synthetic','Synthetic')",[id]).unwrap();
            crate::routing::save(&conn, id, 0, 0, now, Some(true), None).unwrap();
            conn.execute("INSERT INTO oauth_quota_windows(account_id,kind,used_percent,window_minutes,reset_at,observed_at,request_order) VALUES(?1,'secondary',?2,10080,?3,?4,0)",params![id,used,now+days*86400,now]).unwrap();
        }
        let snapshot = |conn: &Connection| {
            let mut value = serde_json::json!({"quota_accounts":[{"id":1},{"id":2}]});
            annotate(conn, user, &mut value).unwrap();
            value
        };
        let value = snapshot(&conn);
        assert_eq!(value["quota_accounts"][0]["routing"]["best"], false);
        assert_eq!(value["quota_accounts"][1]["routing"]["best"], true);
        let share = value["quota_accounts"][1]["routing"]["target_share_percent"]
            .as_f64()
            .unwrap();
        assert!((share - 200.0 / 3.0).abs() < 0.01);
        set_user(&mut conn, user, 2, Some(false), None).unwrap();
        let value = snapshot(&conn);
        assert_eq!(value["quota_accounts"][0]["routing"]["best"], true);
        assert_eq!(
            value["quota_accounts"][0]["routing"]["target_share_percent"],
            100.0
        );
        assert_eq!(value["quota_accounts"][1]["routing"]["best"], false);
        set_user(&mut conn, user, 2, Some(true), None).unwrap();
        for restriction in [
            format!("cooldown_until={}", now + 60),
            "state='reauth_required'".into(),
            format!("expires_at={now}"),
            "catalog_updated_at=NULL".into(),
            format!("catalog_updated_at={}", now - 60),
        ] {
            conn.execute(
                &format!("UPDATE oauth_accounts SET {restriction} WHERE id=2"),
                [],
            )
            .unwrap();
            assert_eq!(
                snapshot(&conn)["quota_accounts"][0]["routing"]["best"],
                true
            );
            conn.execute("UPDATE oauth_accounts SET state='active',cooldown_until=NULL,expires_at=?1,catalog_updated_at=?2 WHERE id=2",params![now+3600,now]).unwrap();
        }
        conn.execute("INSERT INTO oauth_health(account_id,scope,generation,failures,retry_at,reason,last_order) VALUES(2,'responses',0,1,?1,'unavailable',0)",[now+60]).unwrap();
        assert_eq!(
            snapshot(&conn)["quota_accounts"][0]["routing"]["best"],
            true
        );
        conn.execute("DELETE FROM oauth_health WHERE account_id=2", [])
            .unwrap();
        conn.execute(
            "UPDATE oauth_quota_windows SET used_percent=100 WHERE account_id=2",
            [],
        )
        .unwrap();
        assert_eq!(
            snapshot(&conn)["quota_accounts"][0]["routing"]["best"],
            true
        );
        conn.execute("UPDATE oauth_quota_windows SET observed_at=?1", [now - 60])
            .unwrap();
        assert!(snapshot(&conn)["quota_accounts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["routing"]["best"] == false));
    }
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
