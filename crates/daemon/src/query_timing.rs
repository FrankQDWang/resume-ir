use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(crate) enum QueryStage {
    QueryParse,
    Prefilter,
    Bm25,
    Ann,
    Fusion,
    BulkHydrate,
    Snippet,
}

#[derive(Default)]
pub(crate) struct QueryStageTiming {
    query_parse: Duration,
    prefilter: Duration,
    bm25: Duration,
    ann: Duration,
    fusion: Duration,
    bulk_hydrate: Duration,
    snippet: Duration,
}

impl QueryStageTiming {
    pub(crate) fn measure<T>(&mut self, stage: QueryStage, operation: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let output = operation();
        self.record(stage, started.elapsed());
        output
    }

    pub(crate) fn record_since(&mut self, stage: QueryStage, started: Instant) {
        self.record(stage, started.elapsed());
    }

    pub(crate) fn record_duration(&mut self, stage: QueryStage, elapsed: Duration) {
        self.record(stage, elapsed);
    }

    pub(crate) fn server_timing_header_value(&self) -> String {
        format!(
            concat!(
                "query_parse;dur={:.3},prefilter;dur={:.3},bm25;dur={:.3},",
                "ann;dur={:.3},fusion;dur={:.3},bulk_hydrate;dur={:.3},",
                "snippet;dur={:.3}"
            ),
            duration_ms(self.query_parse),
            duration_ms(self.prefilter),
            duration_ms(self.bm25),
            duration_ms(self.ann),
            duration_ms(self.fusion),
            duration_ms(self.bulk_hydrate),
            duration_ms(self.snippet),
        )
    }

    pub(crate) fn duration_ms(&self, stage: QueryStage) -> f64 {
        let duration = match stage {
            QueryStage::QueryParse => self.query_parse,
            QueryStage::Prefilter => self.prefilter,
            QueryStage::Bm25 => self.bm25,
            QueryStage::Ann => self.ann,
            QueryStage::Fusion => self.fusion,
            QueryStage::BulkHydrate => self.bulk_hydrate,
            QueryStage::Snippet => self.snippet,
        };
        duration_ms(duration)
    }

    fn record(&mut self, stage: QueryStage, elapsed: Duration) {
        let target = match stage {
            QueryStage::QueryParse => &mut self.query_parse,
            QueryStage::Prefilter => &mut self.prefilter,
            QueryStage::Bm25 => &mut self.bm25,
            QueryStage::Ann => &mut self.ann,
            QueryStage::Fusion => &mut self.fusion,
            QueryStage::BulkHydrate => &mut self.bulk_hydrate,
            QueryStage::Snippet => &mut self.snippet,
        };
        *target += elapsed;
    }
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
