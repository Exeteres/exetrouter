//! Automatic account economics. Percentages from different plans are never added.
use crate::{quota::Summary, reset::Credits, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

pub(crate) const DAY: i64 = 86_400;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Decision {
    pub urgent: bool,
    pub reason: String,
    pub deadline: Option<i64>,
    pub short_window: Option<bool>,
    pub expiring_resets: Option<usize>,
    pub next_credit_expiry: Option<i64>,
    pub switch_at: u8,
    /// Percentage points per day, not absolute capacity across different plans.
    #[serde(default)]
    pub pacing_weight: Option<f64>,
}

pub(crate) fn fresh(at: i64, now: i64) -> bool {
    (0..60).contains(&now.saturating_sub(at))
}

pub(crate) fn weekly(quota: &Summary) -> Option<&crate::quota::Window> {
    quota
        .windows
        .iter()
        .find(|w| w.status == "current" && w.window_minutes == Some(10080))
}

pub(crate) fn decide(
    quota: &Summary,
    short: Option<bool>,
    credits: Option<&Credits>,
    now: i64,
) -> Decision {
    let credits = credits.filter(|c| fresh(c.observed_at, now));
    let expiry = credits
        .filter(|c| c.details_available && c.available_count > 0)
        .and_then(|c| {
            c.credits
                .iter()
                .filter_map(|c| c.expires_at)
                .filter(|at| *at > now)
                .min()
        });
    let weekly = weekly(quota).filter(|w| fresh(w.observed_at, now));
    let free = weekly
        .filter(|w| w.remaining_percent > 0.0)
        .and_then(|w| w.reset_at)
        .filter(|at| *at > now);
    let credit_deadline = expiry.filter(|at| *at <= now.saturating_add(3 * DAY));
    let weekly_deadline = free.filter(|at| *at <= now.saturating_add(DAY));
    let deadline = credit_deadline.into_iter().chain(weekly_deadline).min();
    let urgent = deadline.is_some();
    Decision {
        urgent,
        reason: if deadline.is_some() && deadline == credit_deadline {
            "reset_expiring"
        } else if urgent {
            "weekly_reset_soon"
        } else if short == Some(true) {
            "short_window_first"
        } else if short == Some(false) {
            "reserve"
        } else {
            "metadata_unknown"
        }
        .into(),
        deadline,
        short_window: short,
        expiring_resets: credits.filter(|c| c.details_available).map(|c| {
            if c.available_count <= 0 {
                0
            } else {
                c.credits
                    .iter()
                    .filter(|c| {
                        c.expires_at
                            .is_some_and(|at| at > now && at <= now.saturating_add(3 * DAY))
                    })
                    .count()
            }
        }),
        next_credit_expiry: credit_deadline,
        switch_at: 0,
        pacing_weight: if !urgent && quota.status == "observed" {
            weekly.zip(free).and_then(|(window, reset)| {
                let weight = window.remaining_percent / ((reset - now) as f64 / DAY as f64);
                (weight.is_finite() && weight > 0.0).then_some(weight)
            })
        } else {
            None
        },
    }
}

impl Decision {
    /// Lower ordering wins. Strategic reset inventory never affects selection.
    pub(crate) fn compare(&self, other: &Self) -> Ordering {
        other
            .urgent
            .cmp(&self.urgent)
            .then_with(|| {
                if self.urgent && other.urgent {
                    self.deadline.cmp(&other.deadline)
                } else {
                    Ordering::Equal
                }
            })
            .then_with(|| {
                self.next_credit_expiry
                    .unwrap_or(i64::MAX)
                    .cmp(&other.next_credit_expiry.unwrap_or(i64::MAX))
            })
            .then_with(|| {
                other
                    .expiring_resets
                    .unwrap_or(0)
                    .cmp(&self.expiring_resets.unwrap_or(0))
            })
            .then_with(|| short_rank(self.short_window).cmp(&short_rank(other.short_window)))
    }
    pub(crate) fn reached(&self, quota: &Summary) -> bool {
        quota
            .windows
            .iter()
            .any(|w| w.status == "current" && w.reset_at.is_some() && w.remaining_percent <= 0.0)
    }
}
fn short_rank(value: Option<bool>) -> u8 {
    match value {
        Some(true) => 0,
        Some(false) => 1,
        None => 2,
    }
}

pub(crate) fn metadata(
    conn: &Connection,
    account: i64,
    now: i64,
) -> Result<(Option<bool>, Option<Credits>)> {
    let row = conn.query_row("SELECT m.observed_at,m.short_window,m.credits FROM account_routing_metadata m JOIN oauth_accounts a ON a.id=m.account_id AND a.generation=m.generation WHERE m.account_id=?1", [account], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,Option<bool>>(1)?,r.get::<_,Option<String>>(2)?))).optional()?;
    match row {
        Some((at, short, credits)) if fresh(at, now) => Ok((
            short,
            credits.map(|s| serde_json::from_str(&s)).transpose()?,
        )),
        _ => Ok((None, None)),
    }
}
pub(crate) fn decision(conn: &Connection, account: i64, now: i64) -> Result<Decision> {
    let (short, credits) = metadata(conn, account, now)?;
    Ok(decide(
        &crate::quota::summary(conn, account, now)?,
        short,
        credits.as_ref(),
        now,
    ))
}
pub(crate) fn save(
    conn: &Connection,
    account: i64,
    generation: i64,
    order: i64,
    now: i64,
    short: Option<bool>,
    credits: Option<&Credits>,
) -> Result<()> {
    let credits = credits.map(serde_json::to_string).transpose()?;
    conn.execute("INSERT INTO account_routing_metadata(account_id,generation,request_order,observed_at,short_window,credits) SELECT ?1,?2,?3,?4,?5,?6 WHERE EXISTS(SELECT 1 FROM oauth_accounts WHERE id=?1 AND generation=?2 AND state='active') ON CONFLICT(account_id) DO UPDATE SET generation=excluded.generation,request_order=excluded.request_order,observed_at=excluded.observed_at,short_window=excluded.short_window,credits=excluded.credits WHERE excluded.request_order>=account_routing_metadata.request_order", params![account,generation,order,now,short,credits])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_window_precedes_reset_count_but_urgency_precedes_short_window() {
        let short = Decision {
            short_window: Some(true),
            expiring_resets: Some(0),
            ..Default::default()
        };
        let reserve = Decision {
            short_window: Some(false),
            expiring_resets: Some(0),
            ..Default::default()
        };
        assert_eq!(short.compare(&reserve), Ordering::Less);
        let urgent = Decision {
            urgent: true,
            deadline: Some(100),
            ..reserve
        };
        assert_eq!(urgent.compare(&short), Ordering::Less);
        let earlier = Decision {
            deadline: Some(99),
            ..urgent.clone()
        };
        assert_eq!(earlier.compare(&urgent), Ordering::Less);
    }
    #[test]
    fn freshness_does_not_turn_unknown_into_zero() {
        assert!(!fresh(101, 100));
        assert!(!fresh(40, 100));
        assert!(fresh(41, 100));
    }
    fn quota(now: i64, reset: i64) -> Summary {
        Summary {
            status: "observed".into(),
            cooldown_until: None,
            cooldown_source: None,
            windows: vec![crate::quota::Window {
                kind: "secondary".into(),
                used_percent: 50.0,
                remaining_percent: 50.0,
                window_minutes: Some(10080),
                reset_at: Some(reset),
                observed_at: now,
                status: "current".into(),
            }],
        }
    }
    #[test]
    fn urgency_uses_credit_expiry_or_weekly_refresh_not_short_window_reset() {
        let now = 1_800_000_000;
        let mut q = quota(now, now + DAY + 1);
        assert!(!decide(&q, Some(true), None, now).urgent);
        q.windows[0].reset_at = Some(now + DAY);
        let decision = decide(&q, Some(false), None, now);
        assert!(decision.urgent);
        assert_eq!(decision.switch_at, 0);
        q.windows[0].status = "stale".into();
        assert!(!decide(&q, Some(false), None, now).urgent);
        q.windows[0].status = "current".into();
        q.windows[0].window_minutes = Some(300);
        assert!(!decide(&q, Some(true), None, now).urgent);
        let mut credits:Credits=serde_json::from_value(serde_json::json!({"available_count":1,"credits":[{"title":"Full reset","expires_at":now+3*DAY}],"details_available":true,"observed_at":now})).unwrap();
        assert!(decide(&q, Some(false), Some(&credits), now).urgent);
        credits.observed_at = now - 60;
        assert!(!decide(&q, Some(false), Some(&credits), now).urgent);
        assert_eq!(decide(&q, None, Some(&credits), now).expiring_resets, None);
    }
    #[test]
    fn pacing_uses_fresh_remaining_weekly_quota_without_changing_session_tiers() {
        let now = 1_800_000_000;
        let a = decide(&quota(now, now + 5 * DAY), Some(true), None, now);
        let b = decide(&quota(now, now + 2 * DAY), Some(true), None, now);
        assert_eq!(a.pacing_weight, Some(10.0));
        assert_eq!(b.pacing_weight, Some(25.0));
        assert_eq!(a.compare(&b), Ordering::Equal);
        assert!(!b.urgent);
        let mut stale = quota(now - 60, now + 2 * DAY);
        assert_eq!(decide(&stale, Some(true), None, now).pacing_weight, None);
        stale.windows[0].remaining_percent = 0.0;
        assert_eq!(decide(&stale, Some(true), None, now).pacing_weight, None);
        let mut depleted = quota(now, now + 2 * DAY);
        depleted.windows[0].remaining_percent = 0.0;
        assert_eq!(decide(&depleted, Some(true), None, now).pacing_weight, None);
    }
    #[test]
    fn metadata_is_fenced_by_generation_order_and_freshness() {
        let conn = Connection::open_in_memory().unwrap();
        crate::store::init(&conn).unwrap();
        conn.execute("INSERT INTO oauth_accounts(account_id,state,encrypted_credentials,expires_at,created_at) VALUES('synthetic','active',X'00',9999,0)",[]).unwrap();
        save(&conn, 1, 0, 2, 100, Some(true), None).unwrap();
        save(&conn, 1, 0, 1, 100, Some(false), None).unwrap();
        assert_eq!(metadata(&conn, 1, 101).unwrap().0, Some(true));
        assert_eq!(metadata(&conn, 1, 160).unwrap().0, None);
        conn.execute("UPDATE oauth_accounts SET generation=1", [])
            .unwrap();
        assert_eq!(metadata(&conn, 1, 101).unwrap().0, None);
    }

    #[test]
    fn strategic_inventory_never_changes_priority_and_expiry_precedes_expiring_count() {
        let now = 1_800_000_000;
        let q = quota(now, now + 6 * DAY);
        let credits = |expiries: Vec<i64>| -> Credits {
            serde_json::from_value(serde_json::json!({"available_count":expiries.len(),"credits":expiries.into_iter().map(|at|serde_json::json!({"title":"Full reset","expires_at":at})).collect::<Vec<_>>(),"details_available":true,"observed_at":now})).unwrap()
        };
        let strategic = credits(vec![now + 30 * DAY; 10]);
        let empty = credits(vec![]);
        assert_eq!(
            decide(&q, Some(true), Some(&strategic), now).compare(&decide(
                &q,
                Some(true),
                Some(&empty),
                now
            )),
            Ordering::Equal
        );
        let earlier = credits(vec![now + DAY]);
        let later = credits(vec![now + 2 * DAY; 5]);
        assert_eq!(
            decide(&q, Some(false), Some(&earlier), now).compare(&decide(
                &q,
                Some(true),
                Some(&later),
                now
            )),
            Ordering::Less
        );
        let more = credits(vec![now + DAY; 2]);
        assert_eq!(
            decide(&q, Some(false), Some(&more), now).compare(&decide(
                &q,
                Some(false),
                Some(&earlier),
                now
            )),
            Ordering::Less
        );
        assert_eq!(
            decide(&q, None, Some(&empty), now).compare(&decide(
                &q,
                Some(false),
                Some(&empty),
                now
            )),
            Ordering::Greater
        );
    }
}
