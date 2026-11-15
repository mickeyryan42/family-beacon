//! Compact "what I already have" summary sent in `Hello`.
//!
//! Most phones hold a few dozen envelopes; for those the summary is the exact
//! list of ids, so nothing is ever skipped by mistake. A busy neighborhood
//! phone may hold thousands, and listing every id would make each greeting
//! tens of kilobytes over Bluetooth. Above [`EXACT_LIMIT`] ids the summary is
//! a Bloom filter at a 1% false-positive rate, about 1.2 bytes per id.
//!
//! A false positive means the peer wrongly thinks we already have something
//! and skips it for this meeting. Each `Hello` uses a fresh seed, so the same
//! envelope is very unlikely to be skipped twice in a row.

use serde::{Deserialize, Serialize};

use crate::ids::EnvelopeId;

/// Up to this many ids are sent exactly (roughly 2.5 KB as JSON).
pub const EXACT_LIMIT: usize = 64;
/// Bits per id for ~1% false positives (-ln(0.01) / ln(2)^2 ≈ 9.6).
const BITS_PER_ID: usize = 10;
const HASHES: u8 = 7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IdSummary {
    Exact {
        ids: Vec<EnvelopeId>,
    },
    Bloom {
        #[serde(with = "crate::b64")]
        bits: Vec<u8>,
        hashes: u8,
        seed: u64,
    },
}

fn mix(mut x: u64) -> u64 {
    // splitmix64 finalizer: cheap, well distributed, identical on every platform.
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn positions(
    id: &EnvelopeId,
    bit_len: usize,
    hashes: u8,
    seed: u64,
) -> impl Iterator<Item = usize> {
    let v = id.0.as_u128();
    let (lo, hi) = (v as u64, (v >> 64) as u64);
    let h1 = mix(lo ^ seed) ^ mix(hi);
    let h2 = mix(hi ^ seed.rotate_left(17)) | 1;
    let bit_len = bit_len.max(1) as u64;
    (0..u64::from(hashes)).map(move |i| (h1.wrapping_add(i.wrapping_mul(h2)) % bit_len) as usize)
}

impl IdSummary {
    pub fn build<'a>(
        ids: impl IntoIterator<Item = &'a EnvelopeId>,
        count: usize,
        seed: u64,
    ) -> Self {
        if count <= EXACT_LIMIT {
            let mut ids: Vec<EnvelopeId> = ids.into_iter().copied().collect();
            ids.sort();
            return IdSummary::Exact { ids };
        }
        let bit_len = (count * BITS_PER_ID).next_multiple_of(8);
        let mut bits = vec![0u8; bit_len / 8];
        for id in ids {
            for p in positions(id, bit_len, HASHES, seed) {
                bits[p / 8] |= 1 << (p % 8);
            }
        }
        IdSummary::Bloom {
            bits,
            hashes: HASHES,
            seed,
        }
    }

    pub fn empty() -> Self {
        IdSummary::Exact { ids: Vec::new() }
    }

    /// `false` means definitely not held; `true` means held (exact) or
    /// probably held (Bloom).
    pub fn might_contain(&self, id: &EnvelopeId) -> bool {
        match self {
            IdSummary::Exact { ids } => ids.binary_search(id).is_ok(),
            IdSummary::Bloom { bits, hashes, seed } => {
                !bits.is_empty()
                    && positions(id, bits.len() * 8, *hashes, *seed)
                        .all(|p| bits.get(p / 8).is_some_and(|b| b & (1 << (p % 8)) != 0))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_sets_are_exact() {
        let ids: Vec<_> = (0..EXACT_LIMIT).map(|_| EnvelopeId::new()).collect();
        let s = IdSummary::build(&ids, ids.len(), 1);
        assert!(matches!(s, IdSummary::Exact { .. }));
        assert!(ids.iter().all(|id| s.might_contain(id)));
        assert!((0..10_000).all(|_| !s.might_contain(&EnvelopeId::new())));
    }

    #[test]
    fn large_sets_contain_everything_they_were_built_from() {
        let ids: Vec<_> = (0..500).map(|_| EnvelopeId::new()).collect();
        let s = IdSummary::build(&ids, ids.len(), 42);
        assert!(matches!(s, IdSummary::Bloom { .. }));
        assert!(ids.iter().all(|id| s.might_contain(id)));
    }

    #[test]
    fn bloom_false_positive_rate_is_low_and_size_is_compact() {
        let ids: Vec<_> = (0..1000).map(|_| EnvelopeId::new()).collect();
        let IdSummary::Bloom { bits, .. } = IdSummary::build(&ids, ids.len(), 7) else {
            panic!("expected a Bloom summary");
        };
        assert!(bits.len() <= 1300, "summary is {} bytes", bits.len());

        let s = IdSummary::build(&ids, ids.len(), 7);
        let false_positives = (0..10_000)
            .filter(|_| s.might_contain(&EnvelopeId::new()))
            .count();
        assert!(
            false_positives < 300,
            "{false_positives} false positives in 10k"
        );
    }

    #[test]
    fn empty_summary_contains_nothing() {
        assert!(!IdSummary::empty().might_contain(&EnvelopeId::new()));
    }
}
