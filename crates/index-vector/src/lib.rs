use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq)]
pub struct VectorDocument {
    pub doc_id: String,
    pub vector: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VectorHit {
    pub rank: usize,
    pub doc_id: String,
    pub score: f32,
}

pub trait VectorIndex {
    fn upsert(&mut self, document: VectorDocument);
    fn search(&self, query: &[f32], top_k: usize) -> Vec<VectorHit>;
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InMemoryVectorIndex {
    documents: Vec<VectorDocument>,
}

impl VectorIndex for InMemoryVectorIndex {
    fn upsert(&mut self, document: VectorDocument) {
        if let Some(existing) = self
            .documents
            .iter_mut()
            .find(|existing| existing.doc_id == document.doc_id)
        {
            *existing = document;
            return;
        }
        self.documents.push(document);
    }

    fn search(&self, query: &[f32], top_k: usize) -> Vec<VectorHit> {
        let mut hits: Vec<VectorHit> = self
            .documents
            .iter()
            .map(|document| VectorHit {
                rank: 0,
                doc_id: document.doc_id.clone(),
                score: cosine_similarity(query, &document.vector),
            })
            .collect();
        hits.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.doc_id.cmp(&right.doc_id))
        });
        hits.truncate(top_k);
        for (index, hit) in hits.iter_mut().enumerate() {
            hit.rank = index + 1;
        }
        hits
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let dot = left
        .iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        return 0.0;
    }
    dot / (left_norm * right_norm)
}

#[must_use]
pub fn crate_name() -> &'static str {
    "index-vector"
}
