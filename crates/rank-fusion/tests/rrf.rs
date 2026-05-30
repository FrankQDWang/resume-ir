use rank_fusion::{HybridRankInput, RankedHit, fuse_hybrid_results, reciprocal_rank_fusion};

#[test]
fn reciprocal_rank_fusion_promotes_documents_seen_by_both_rankers() {
    let fulltext = vec![
        RankedHit::new("doc_java", 1),
        RankedHit::new("doc_design", 2),
    ];
    let vector = vec![RankedHit::new("doc_rust", 1), RankedHit::new("doc_java", 2)];

    let fused = reciprocal_rank_fusion(&[fulltext, vector], 10);

    assert_eq!(fused[0].doc_id, "doc_java");
    assert_eq!(fused[0].rank, 1);
    assert!(fused[0].score > fused[1].score);
}

#[test]
fn reciprocal_rank_fusion_respects_top_k() {
    let fulltext = vec![
        RankedHit::new("doc_a", 1),
        RankedHit::new("doc_b", 2),
        RankedHit::new("doc_c", 3),
    ];

    let fused = reciprocal_rank_fusion(&[fulltext], 2);

    assert_eq!(fused.len(), 2);
    assert_eq!(fused[0].rank, 1);
    assert_eq!(fused[1].rank, 2);
}

#[test]
fn hybrid_fusion_interface_combines_fulltext_and_vector_rankers() {
    let fused = fuse_hybrid_results(HybridRankInput {
        fulltext: vec![RankedHit::new("doc_java", 1)],
        vector: vec![RankedHit::new("doc_java", 2), RankedHit::new("doc_rust", 1)],
        top_k: 2,
    });

    assert_eq!(fused[0].doc_id, "doc_java");
    assert_eq!(fused.len(), 2);
}
