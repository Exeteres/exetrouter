//! Process-local reservations for stateless requests and connection-local chats.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub(crate) struct Candidate {
    pub id: i64,
    pub quota: Option<f64>,
    pub routing: crate::routing::Decision,
}

/// Do not invent weights for missing observations or mix economic tiers.
pub(crate) fn paced(candidates: &[Candidate]) -> bool {
    !candidates.is_empty()
        && candidates.iter().all(|c| {
            c.quota.is_some()
                && !c.routing.urgent
                && c.routing
                    .pacing_weight
                    .is_some_and(|w| w.is_finite() && w > 0.0)
        })
}

pub(crate) fn top_tier(candidates: &[Candidate]) -> Vec<Candidate> {
    let Some(best) = candidates
        .iter()
        .min_by(|a, b| a.routing.compare(&b.routing))
    else {
        return Vec::new();
    };
    candidates
        .iter()
        .filter(|c| c.routing.compare(&best.routing).is_eq())
        .cloned()
        .collect()
}

pub(crate) struct Recommendation {
    pub id: i64,
    pub best: bool,
    pub share: Option<f64>,
}

/// General-pool advisory, not a promise about a specific model or owned dialog.
pub(crate) fn recommendations(candidates: &[Candidate]) -> Vec<Recommendation> {
    let top = top_tier(candidates);
    if top.is_empty()
        || top
            .iter()
            .any(|c| c.quota.is_none() || (!c.routing.urgent && c.routing.short_window.is_none()))
    {
        return Vec::new();
    }
    let paced = paced(&top);
    let weight = |c: &Candidate| {
        if paced {
            c.routing.pacing_weight.expect("paced")
        } else {
            c.quota.expect("known")
        }
    };
    let best = top
        .iter()
        .max_by(|a, b| {
            weight(a)
                .total_cmp(&weight(b))
                .then_with(|| b.id.cmp(&a.id))
        })
        .expect("nonempty tier");
    let total: f64 = top.iter().map(weight).sum();
    candidates
        .iter()
        .map(|c| {
            let in_tier = c.routing.compare(&best.routing).is_eq();
            let share = if !in_tier {
                Some(0.0)
            } else if paced {
                Some(100.0 * weight(c) / total)
            } else if top.len() == 1 {
                Some(100.0)
            } else {
                None
            }; // Cache affinity can choose any tied urgent account.
            Recommendation {
                id: c.id,
                best: c.id == best.id,
                share,
            }
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct Pool {
    loads: Arc<Mutex<BTreeMap<i64, usize>>>,
}

pub(crate) struct Reservation {
    pub id: i64,
    loads: Arc<Mutex<BTreeMap<i64, usize>>>,
}

impl Pool {
    pub fn reserve(&self, candidates: &[Candidate]) -> Option<Reservation> {
        self.reserve_affinity(candidates, None)
    }

    pub fn reserve_affinity(
        &self,
        candidates: &[Candidate],
        affinity: Option<&str>,
    ) -> Option<Reservation> {
        let mut loads = self.loads.lock().expect("pool lock poisoned");
        let candidates = top_tier(candidates);
        if paced(&candidates) {
            // Weighted rendezvous keeps hints stable across restart, with no
            // session registry. Unhinted independent requests use randomness.
            let selected = candidates
                .iter()
                .map(|candidate| {
                    let uniform = if let Some(affinity) = affinity {
                        use sha2::{Digest, Sha256};
                        let mut hash = Sha256::new();
                        hash.update(b"exetrouter-account-pacing-v1\0");
                        hash.update(affinity.as_bytes());
                        hash.update(candidate.id.to_be_bytes());
                        let bits = u64::from_be_bytes(
                            hash.finalize()[..8].try_into().expect("eight bytes"),
                        ) >> 11;
                        (bits as f64 + 1.0) / ((1_u64 << 53) as f64 + 2.0)
                    } else {
                        use rand::Rng;
                        rand::thread_rng().gen_range(f64::MIN_POSITIVE..1.0)
                    };
                    (
                        -uniform.ln() / candidate.routing.pacing_weight.expect("paced"),
                        candidate.id,
                    )
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)))?;
            *loads.entry(selected.1).or_default() += 1;
            return Some(Reservation {
                id: selected.1,
                loads: self.loads.clone(),
            });
        }
        // Unknown quota is not an assumed zero or an assumed exhausted limit.
        // Use quota ranking only when every candidate has a fresh observation.
        let all_known = candidates.iter().all(|candidate| candidate.quota.is_some());
        let selected = candidates.iter().min_by(|a, b| {
            let priority = a.routing.compare(&b.routing);
            if priority != std::cmp::Ordering::Equal {
                return priority;
            }
            // Rendezvous hashing needs no unbounded session table and is stable
            // after restart. Eligibility has already excluded paused accounts.
            if let Some(affinity) = affinity {
                use sha2::{Digest, Sha256};
                let score = |id: i64| {
                    let mut hash = Sha256::new();
                    hash.update(b"exetrouter-account-affinity-v1\0");
                    hash.update(affinity.as_bytes());
                    hash.update(id.to_be_bytes());
                    hash.finalize()
                };
                return score(b.id).cmp(&score(a.id)).then_with(|| a.id.cmp(&b.id));
            }
            let quota = if all_known {
                // Candidate quota is USED percentage. Finish the more consumed
                // account within this economic tier instead of spreading use.
                b.quota
                    .expect("all known")
                    .total_cmp(&a.quota.expect("all known"))
            } else {
                std::cmp::Ordering::Equal
            };
            quota
                .then_with(|| {
                    loads
                        .get(&a.id)
                        .unwrap_or(&0)
                        .cmp(loads.get(&b.id).unwrap_or(&0))
                })
                .then_with(|| a.id.cmp(&b.id))
        })?;
        *loads.entry(selected.id).or_default() += 1;
        Some(Reservation {
            id: selected.id,
            loads: self.loads.clone(),
        })
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let mut loads = self.loads.lock().expect("pool lock poisoned");
        let count = loads.get_mut(&self.id).expect("reserved account");
        *count -= 1;
        if *count == 0 {
            loads.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_sessions_are_stable_distributed_and_urgency_overrides_weights() {
        let pool = Pool::default();
        let mut candidates = [
            Candidate {
                id: 1,
                quota: Some(40.0),
                routing: crate::routing::Decision {
                    short_window: Some(true),
                    pacing_weight: Some(10.0),
                    ..Default::default()
                },
            },
            Candidate {
                id: 2,
                quota: Some(20.0),
                routing: crate::routing::Decision {
                    short_window: Some(true),
                    pacing_weight: Some(20.0),
                    ..Default::default()
                },
            },
        ];
        let mut second = 0;
        for i in 0..6000 {
            let hint = format!("synthetic-session-{i}");
            let first = pool.reserve_affinity(&candidates, Some(&hint)).unwrap().id;
            assert_eq!(
                first,
                pool.reserve_affinity(&candidates, Some(&hint)).unwrap().id
            );
            second += usize::from(first == 2);
        }
        assert!((3800..4200).contains(&second), "second account: {second}");
        let targets = recommendations(&candidates);
        assert!(!targets[0].best);
        assert!(targets[1].best);
        assert!((targets[1].share.unwrap() - 200.0 / 3.0).abs() < 0.001);
        candidates[0].routing.urgent = true;
        for hint in [None, Some("synthetic-session")] {
            assert_eq!(pool.reserve_affinity(&candidates, hint).unwrap().id, 1);
        }
        assert_eq!(recommendations(&candidates)[0].share, Some(100.0));
        candidates[0].routing.urgent = false;
        candidates[0].routing.pacing_weight = None;
        assert!(!paced(&candidates));
        candidates[0].quota = None;
        assert!(recommendations(&candidates).is_empty());
    }

    #[test]
    fn unknown_quota_balances_reservations_and_releases_every_account() {
        let pool = Pool::default();
        let candidates = [
            Candidate {
                id: 1,
                quota: None,
                routing: Default::default(),
            },
            Candidate {
                id: 2,
                routing: Default::default(),
                quota: Some(50.0),
            },
        ];
        let reservations = (0..8)
            .map(|_| pool.reserve(&candidates).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            reservations.iter().map(|r| r.id).collect::<Vec<_>>(),
            [1, 2, 1, 2, 1, 2, 1, 2]
        );
        drop(reservations);
        assert!(pool.loads.lock().unwrap().is_empty());
        assert_eq!(pool.reserve(&candidates).unwrap().id, 1);
        assert!(pool.reserve(&[]).is_none());
    }

    #[test]
    fn automatic_routing_precedes_quota_load_and_soft_cache_affinity() {
        let pool = Pool::default();
        let candidates = [
            Candidate {
                id: 1,
                quota: Some(0.),
                routing: Default::default(),
            },
            Candidate {
                id: 2,
                quota: Some(90.),
                routing: crate::routing::Decision {
                    urgent: true,
                    ..Default::default()
                },
            },
        ];
        for hint in [None, Some("synthetic-cache")] {
            assert_eq!(pool.reserve_affinity(&candidates, hint).unwrap().id, 2);
        }
    }
    #[test]
    fn fresh_quotas_rank_before_load_and_equal_quotas_use_load_then_id() {
        let pool = Pool::default();
        let candidates = [
            Candidate {
                id: 1,
                routing: Default::default(),
                quota: Some(70.0),
            },
            Candidate {
                id: 2,
                routing: Default::default(),
                quota: Some(20.0),
            },
        ];
        let first = pool.reserve(&candidates).unwrap();
        let second = pool.reserve(&candidates).unwrap();
        assert_eq!((first.id, second.id), (1, 1));
        let tied = [
            Candidate {
                id: 1,
                routing: Default::default(),
                quota: Some(20.0),
            },
            Candidate {
                id: 2,
                routing: Default::default(),
                quota: Some(20.0),
            },
        ];
        assert_eq!(pool.reserve(&tied).unwrap().id, 2);
    }
}
