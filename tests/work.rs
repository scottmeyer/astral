use ostk_gpt_cache::work::{MAX_OCCUPIED_IDS, WorkIdError, generate_id};

#[test]
fn os_generated_proposal_has_the_documented_shape_and_preserves_legacy_ids() {
    let occupied = ["AST-001", "AST-010", "legacy-work-item", "AST-000000000000"];
    let id = generate_id(occupied).expect("OS randomness available on supported test host");
    assert_eq!(id.len(), 16);
    let suffix = id.strip_prefix("AST-").expect("work prefix");
    assert_eq!(suffix.len(), 12);
    assert!(
        suffix
            .bytes()
            .all(|byte| b"0123456789abcdefghjkmnpqrstvwxyz".contains(&byte))
    );
    assert!(!occupied.contains(&id.as_str()));
    assert_eq!(
        occupied,
        ["AST-001", "AST-010", "legacy-work-item", "AST-000000000000"]
    );
}

#[test]
fn public_api_bounds_even_duplicate_occupied_entries() {
    let result = generate_id(std::iter::repeat_n("AST-001", MAX_OCCUPIED_IDS + 1));
    assert_eq!(result, Err(WorkIdError::OccupiedInputLimit));
    assert_eq!(result.unwrap_err().code(), "WORK_ID_INPUT_LIMIT");
}
