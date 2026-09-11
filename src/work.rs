//! Non-reserving, merge-friendly work ID proposals.
//!
//! New IDs contain 60 OS-random bits, rendered as twelve lowercase Crockford
//! base32 characters after `AST-`. They are independent of time, counters, source
//! content, and existing IDs. Existing IDs only reject occupied candidates.
//! Collisions remain possible, especially between concurrent branch allocations:
//! callers must still validate uniqueness when persisting or merging records.

use std::collections::BTreeSet;

const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
pub const MAX_OCCUPIED_IDS: usize = 4_096;
pub const MAX_OCCUPIED_BYTES: usize = 16_777_216;
pub const MAX_GENERATION_ATTEMPTS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkIdError {
    OccupiedInputLimit,
    Entropy(getrandom::Error),
    CollisionsExhausted,
}

impl WorkIdError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::OccupiedInputLimit => "WORK_ID_INPUT_LIMIT",
            Self::Entropy(_) => "WORK_ID_ENTROPY_FAILED",
            Self::CollisionsExhausted => "WORK_ID_COLLISIONS_EXHAUSTED",
        }
    }
}

impl std::fmt::Display for WorkIdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OccupiedInputLimit => {
                formatter.write_str("occupied work ID input exceeds its count or byte limit")
            }
            Self::Entropy(source) => write!(formatter, "OS randomness unavailable: {source}"),
            Self::CollisionsExhausted => formatter
                .write_str("all bounded work ID generation attempts collided with occupied IDs"),
        }
    }
}

impl std::error::Error for WorkIdError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Entropy(source) => Some(source),
            _ => None,
        }
    }
}

/// Propose an ID absent from this observed set, without reserving or persisting it.
/// Historical IDs need no migration and are compared exactly as supplied.
/// At most sixteen candidates are drawn; OS entropy failure never falls back to
/// a clock, deterministic seed, counter, or weaker randomness source.
pub fn generate_id<'a>(occupied: impl IntoIterator<Item = &'a str>) -> Result<String, WorkIdError> {
    generate_with(occupied, getrandom::fill)
}

fn generate_with<'a>(
    occupied: impl IntoIterator<Item = &'a str>,
    mut fill: impl FnMut(&mut [u8]) -> Result<(), getrandom::Error>,
) -> Result<String, WorkIdError> {
    let mut ids = BTreeSet::new();
    let mut bytes = 0usize;
    for (index, id) in occupied.into_iter().enumerate() {
        bytes = bytes.saturating_add(id.len());
        if index >= MAX_OCCUPIED_IDS || bytes > MAX_OCCUPIED_BYTES {
            return Err(WorkIdError::OccupiedInputLimit);
        }
        ids.insert(id);
    }
    for _ in 0..MAX_GENERATION_ATTEMPTS {
        let mut entropy = [0u8; 8];
        fill(&mut entropy).map_err(WorkIdError::Entropy)?;
        let candidate = encode(entropy);
        if !ids.contains(candidate.as_str()) {
            return Ok(candidate);
        }
    }
    Err(WorkIdError::CollisionsExhausted)
}

fn encode(entropy: [u8; 8]) -> String {
    // Twelve disjoint five-bit groups use exactly the low sixty uniform bits.
    let bits = u64::from_be_bytes(entropy);
    let mut result = String::with_capacity(16);
    result.push_str("AST-");
    for digit in (0..12).rev() {
        result.push(ALPHABET[((bits >> (digit * 5)) & 31) as usize] as char);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_uses_exactly_sixty_bits_and_crockford_digits() {
        assert_eq!(encode([0; 8]), "AST-000000000000");
        assert_eq!(encode([255; 8]), "AST-zzzzzzzzzzzz");
        assert_eq!(encode(1u64.to_be_bytes()), "AST-000000000001");
        assert_eq!(encode(32u64.to_be_bytes()), "AST-000000000010");
        assert_eq!(
            encode((0xf000_0000_0000_0000u64).to_be_bytes()),
            "AST-000000000000"
        );
        for (value, expected) in b"0123456789abcdefghjkmnpqrstvwxyz".iter().enumerate() {
            assert_eq!(
                encode((value as u64).to_be_bytes()),
                format!("AST-00000000000{}", *expected as char)
            );
        }
        for position in 0..12 {
            let mut expected = b"AST-000000000000".to_vec();
            expected[15 - position] = b'1';
            assert_eq!(
                encode((1u64 << (5 * position)).to_be_bytes()).as_bytes(),
                expected
            );
        }
    }

    #[test]
    fn known_collision_retries_with_fresh_entropy_without_changing_existing_ids() {
        let occupied = ["AST-001", "historical-id", "AST-000000000000"];
        let mut draws = 0u64;
        let result = generate_with(occupied, |bytes| {
            bytes.copy_from_slice(&draws.to_be_bytes());
            draws += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(result, "AST-000000000001");
        assert_eq!(draws, 2);
        assert_eq!(occupied, ["AST-001", "historical-id", "AST-000000000000"]);
    }

    #[test]
    fn repeated_collisions_stop_at_the_attempt_bound() {
        let mut draws = 0;
        let result = generate_with(["AST-000000000000"], |bytes| {
            draws += 1;
            bytes.fill(0);
            Ok(())
        });
        assert_eq!(result, Err(WorkIdError::CollisionsExhausted));
        assert_eq!(draws, MAX_GENERATION_ATTEMPTS);
    }

    #[test]
    fn entropy_failure_propagates_immediately_with_original_cause() {
        let mut draws = 0;
        let result = generate_with([], |_| {
            draws += 1;
            Err(getrandom::Error::UNSUPPORTED)
        });
        assert_eq!(
            result,
            Err(WorkIdError::Entropy(getrandom::Error::UNSUPPORTED))
        );
        assert_eq!(draws, 1);
        assert!(std::error::Error::source(&result.unwrap_err()).is_some());
    }

    #[test]
    fn failure_after_collision_does_not_reuse_entropy_or_invent_a_fallback() {
        let mut draws = 0;
        let result = generate_with(["AST-000000000000"], |bytes| {
            draws += 1;
            if draws == 1 {
                bytes.fill(0);
                Ok(())
            } else {
                Err(getrandom::Error::UNEXPECTED)
            }
        });
        assert_eq!(
            result,
            Err(WorkIdError::Entropy(getrandom::Error::UNEXPECTED))
        );
        assert_eq!(draws, 2);
    }

    #[test]
    fn occupied_input_limits_are_enforced_before_requesting_entropy() {
        let too_many = std::iter::repeat_n("AST-001", MAX_OCCUPIED_IDS + 1);
        let result = generate_with(too_many, |_| panic!("entropy must not be requested"));
        assert_eq!(result, Err(WorkIdError::OccupiedInputLimit));
        let oversized = "x".repeat(MAX_OCCUPIED_BYTES + 1);
        let result = generate_with([oversized.as_str()], |_| {
            panic!("entropy must not be requested")
        });
        assert_eq!(result, Err(WorkIdError::OccupiedInputLimit));
    }

    #[test]
    fn identical_entropy_is_independent_of_unrelated_occupied_ids() {
        let fill = |bytes: &mut [u8]| {
            bytes.copy_from_slice(&1234u64.to_be_bytes());
            Ok(())
        };
        assert_eq!(
            generate_with([], fill).unwrap(),
            generate_with(["AST-001", "AST-999", "legacy"], fill).unwrap()
        );
    }
}
