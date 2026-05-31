use rank_fusion::{
    fold_by_candidate, fuse_hybrid_rrf, fuse_ranked_channels, HybridRecall, RankedChannel,
    RankedHit, RetrievalChannel,
};

#[test]
fn hybrid_ranked_channels_use_rrf_without_channel_score_scaling() {
    let fused = fuse_ranked_channels(
        [
            RankedChannel::new(
                RetrievalChannel::FullText,
                ["doc_keyword", "doc_shared", "doc_other"],
            ),
            RankedChannel::new(RetrievalChannel::Vector, ["doc_shared", "doc_semantic"]),
        ],
        60.0,
    );

    assert_eq!(fused[0].doc_id(), "doc_shared");
    assert!(!format!("{:?}", fused[0]).contains("doc_semantic"));
}

#[test]
fn hybrid_rrf_preserves_candidate_keys_for_later_candidate_fold() {
    let fulltext = vec![
        RankedHit::new("doc_keyword", 1, 9000.0).with_candidate_key("cand_same"),
        RankedHit::new("doc_exact", 2, 8000.0).with_candidate_key("cand_exact"),
    ];
    let vector = vec![
        RankedHit::new("doc_semantic", 1, 0.2).with_candidate_key("cand_same"),
        RankedHit::new("doc_exact", 2, 0.1).with_candidate_key("cand_exact"),
    ];

    let fused = fuse_hybrid_rrf(HybridRecall::new(fulltext, vector), 60.0, 10);
    let folded = fold_by_candidate(fused);

    assert_eq!(folded.len(), 2);
    assert!(folded.iter().any(|hit| hit.doc_id() == "doc_keyword"));
    assert!(folded.iter().any(|hit| hit.doc_id() == "doc_exact"));
}
