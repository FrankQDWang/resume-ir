use crate::model::{QueryVector, VectorDocument, VectorHit, VectorIndexError};
use hnsw_rs::prelude::{DistCosine, Hnsw};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

const HNSW_MAX_CONNECTIONS: usize = 24;
const HNSW_MAX_LAYERS: usize = 16;
const HNSW_EF_CONSTRUCTION: usize = 200;
const HNSW_EF_SEARCH: usize = 64;

pub(crate) struct AnnIndex {
    all: Option<AnnShard>,
    by_model: BTreeMap<String, AnnShard>,
}

impl AnnIndex {
    pub(crate) fn build(documents: &[VectorDocument]) -> Self {
        let mut by_model_documents = BTreeMap::<String, Vec<VectorDocument>>::new();
        for document in documents {
            by_model_documents
                .entry(document.model_id().to_string())
                .or_default()
                .push(document.clone());
        }
        Self {
            all: AnnShard::build(documents.to_vec()),
            by_model: by_model_documents
                .into_iter()
                .filter_map(|(model_id, documents)| {
                    AnnShard::build(documents).map(|shard| (model_id, shard))
                })
                .collect(),
        }
    }

    pub(crate) fn knn(
        &self,
        query: QueryVector,
        k: usize,
        model_id: Option<&str>,
    ) -> Result<Vec<VectorHit>, VectorIndexError> {
        let shard = match model_id {
            Some(model_id) => self.by_model.get(model_id),
            None => self.all.as_ref(),
        };
        Ok(shard
            .map(|shard| shard.knn(query.values(), k))
            .unwrap_or_default())
    }
}

struct AnnShard {
    index: Hnsw<'static, f32, DistCosine>,
    documents: Vec<VectorDocument>,
}

impl AnnShard {
    fn build(documents: Vec<VectorDocument>) -> Option<Self> {
        if documents.is_empty() {
            return None;
        }
        let max_layer = HNSW_MAX_LAYERS
            .min((documents.len() as f32).ln().trunc() as usize)
            .max(1);
        let mut index = Hnsw::<f32, DistCosine>::new(
            HNSW_MAX_CONNECTIONS,
            documents.len(),
            max_layer,
            HNSW_EF_CONSTRUCTION,
            DistCosine {},
        );
        for (external_id, document) in documents.iter().enumerate() {
            index.insert((document.values(), external_id));
        }
        index.set_searching_mode(true);
        Some(Self { index, documents })
    }

    fn knn(&self, query: &[f32], k: usize) -> Vec<VectorHit> {
        let target_count = k.min(self.documents.len());
        if target_count == 0 {
            return Vec::new();
        }
        let mut hits = self
            .index
            .search(query, target_count, HNSW_EF_SEARCH.max(target_count))
            .into_iter()
            .filter_map(|neighbour| self.documents.get(neighbour.d_id))
            .map(|document| {
                VectorHit::from_document(document, cosine_similarity(query, document.values()))
            })
            .collect::<Vec<_>>();
        if hits.len() < target_count {
            let mut seen = hits
                .iter()
                .map(VectorHit::vector_id)
                .collect::<BTreeSet<_>>();
            let mut backfill = self
                .documents
                .iter()
                .filter(|document| seen.insert(document.vector_id()))
                .map(|document| {
                    VectorHit::from_document(document, cosine_similarity(query, document.values()))
                })
                .collect::<Vec<_>>();
            sort_hits(&mut backfill);
            hits.extend(backfill.into_iter().take(target_count - hits.len()));
        }
        sort_hits(&mut hits);
        hits.truncate(target_count);
        hits
    }
}

fn sort_hits(hits: &mut [VectorHit]) {
    hits.sort_by(|left, right| {
        right
            .score()
            .partial_cmp(&left.score())
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.document_id().cmp(right.document_id()))
            .then_with(|| left.resume_version_id().cmp(right.resume_version_id()))
            .then_with(|| left.vector_id().cmp(right.vector_id()))
    });
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let dot = left
        .iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm * right_norm)
    }
}
