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
