//! Hybrid retrieval fusion contract tests for the S11 skeleton.

use rank_fusion::{
    HybridFusion, HybridRankedLists, RankSource, RankedSourceHit, ReciprocalRankFusion, RrfConfig,
};

#[test]
fn rrf_merges_fulltext_and_vector_lists_with_source_contributions() {
    let fusion = ReciprocalRankFusion::new(RrfConfig::new(60.0));
    let result = fusion.fuse(HybridRankedLists::new(
        vec![
            RankedSourceHit::new("doc-a", 1234.5),
            RankedSourceHit::new("doc-b", 1200.0),
        ],
        vec![
            RankedSourceHit::new("doc-b", 0.9876),
            RankedSourceHit::new("doc-c", 0.8765),
        ],
    ));

    assert_eq!(
        result
            .hits()
            .iter()
            .map(|hit| hit.doc_id())
            .collect::<Vec<_>>(),
        ["doc-b", "doc-a", "doc-c"]
    );

    let top = &result.hits()[0];
    assert_eq!(top.doc_id(), "doc-b");
    assert_eq!(top.contributions().len(), 2);
    assert_eq!(top.contributions()[0].source(), RankSource::FullText);
    assert_eq!(top.contributions()[0].rank(), 2);
    assert_eq!(top.contributions()[1].source(), RankSource::Vector);
    assert_eq!(top.contributions()[1].rank(), 1);
    assert!(
        (top.score() - ((1.0 / 62.0) + (1.0 / 61.0))).abs() < f32::EPSILON,
        "unexpected RRF score: {}",
        top.score()
    );

    let debug = format!("{result:?}");
    assert!(debug.contains("[redacted source score]"));
    assert!(!debug.contains("1234.5"));
    assert!(!debug.contains("0.9876"));
}

#[test]
fn rrf_uses_configured_k_and_doc_id_tie_breaking() {
    let fusion = ReciprocalRankFusion::new(RrfConfig::new(10.0));
    let result = fusion.fuse(HybridRankedLists::new(
        vec![RankedSourceHit::new("doc-b", 5.0)],
        vec![RankedSourceHit::new("doc-a", 0.5)],
    ));

    assert_eq!(
        result
            .hits()
            .iter()
            .map(|hit| hit.doc_id())
            .collect::<Vec<_>>(),
        ["doc-a", "doc-b"]
    );
    assert!((result.hits()[0].score() - (1.0 / 11.0)).abs() < f32::EPSILON);
    assert!((result.hits()[1].score() - (1.0 / 11.0)).abs() < f32::EPSILON);
}
