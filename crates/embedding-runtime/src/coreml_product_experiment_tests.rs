use super::*;

#[test]
fn experiment_role_is_explicit_and_closed() {
    assert_eq!(parse_role(&[], false).unwrap(), None);
    assert_eq!(
        parse_role(
            &[
                "--resident-embedding-pool-experiment".into(),
                "--resident-embedding-pool-role=interactive".into(),
            ],
            true,
        )
        .unwrap(),
        Some(CoreMlRole::Interactive)
    );
    assert_eq!(
        parse_role(
            &[
                "--resident-embedding-pool-experiment".into(),
                "--resident-embedding-pool-role=bulk".into(),
            ],
            true,
        )
        .unwrap(),
        Some(CoreMlRole::Bulk)
    );
    assert!(parse_role(&["--resident".into()], true).is_err());
}

#[test]
fn vector_decode_is_bounded_finite_and_drops_padded_rows() {
    let mut bytes = Vec::new();
    for row in 0..4 {
        for column in 0..DIMENSION {
            bytes.extend_from_slice(&((row * DIMENSION + column) as f32).to_le_bytes());
        }
    }
    let vectors = decode_vectors(&bytes, 2).unwrap();
    assert_eq!(vectors.len(), 2);
    assert_eq!(vectors[0].len(), DIMENSION);
    assert_eq!(vectors[1][0], DIMENSION as f32);

    bytes[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(decode_vectors(&bytes, 1).is_err());
    assert!(decode_vectors(&bytes, 0).is_err());
}
