//! Process logging. Verbosity follows `RUST_LOG` (default `info`); independently, the database
//! targets (`dgos_db::QUIET_LOG_TARGETS`) are capped at `warn` by a second filter AND-ed with it.
//! Their `trace` spans carry bound statement values, which include chat text. Because the cap is
//! a separate filter, no `RUST_LOG` spec, however specific its targets, can lift it.

use tracing::level_filters::LevelFilter;
use tracing_subscriber::filter::{EnvFilter, FilterExt, Targets};
use tracing_subscriber::layer::{Filter, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

/// The user's verbosity, falling back to `info` when the spec does not parse.
fn verbosity(spec: &str) -> EnvFilter {
    EnvFilter::try_new(spec).unwrap_or_else(|_| EnvFilter::new("info"))
}

/// Everything passes except the database targets above `warn`.
fn database_cap() -> Targets {
    dgos_db::QUIET_LOG_TARGETS.iter().fold(
        Targets::new().with_default(LevelFilter::TRACE),
        |cap, target| cap.with_target(*target, LevelFilter::WARN),
    )
}

pub fn filter<S>(spec: &str) -> impl Filter<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    verbosity(spec).and(database_cap())
}

pub fn init() {
    let spec = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter(&spec)))
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = Buffer;
        fn make_writer(&'a self) -> Buffer {
            self.clone()
        }
    }

    fn captured(
        filter: impl Filter<tracing_subscriber::Registry> + Send + Sync + 'static,
    ) -> String {
        let buffer = Buffer::default();
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(buffer.clone())
            .with_filter(filter);
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(target: "sea_orm::driver::sqlx_sqlite", values = "DB-SENTINEL-SQL", "statement");
            tracing::debug!(target: "sqlx::query", values = "DB-SENTINEL-SQLX", "statement");
            // The real leak path: sea-orm's trace span records the statement, and the fmt layer
            // prints enabled span fields with every event logged inside it.
            let span = tracing::trace_span!(target: "sea_orm::driver::sqlx_sqlite", "sqlx_sqlite", stmt = "DB-SENTINEL-SPAN");
            span.in_scope(|| tracing::trace!(target: "dgos_lib", "event inside a database span"));
            tracing::trace!(target: "dgos_lib", marker = "APP-TRACE-STILL-WORKS", "app event");
        });
        let bytes = buffer.0.lock().unwrap().clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn a_trace_filter_cannot_lift_the_database_cap() {
        let text = captured(filter("trace"));
        assert!(
            text.contains("APP-TRACE-STILL-WORKS"),
            "other targets keep the requested verbosity"
        );
        assert!(!text.contains("DB-SENTINEL-SQL") && !text.contains("DB-SENTINEL-SQLX"));
        let explicit = captured(filter("sea_orm=trace,sqlx=trace,trace"));
        assert!(
            !explicit.contains("DB-SENTINEL"),
            "even an explicit database directive is overridden"
        );
        // EnvFilter picks the most specific matching directive, so a longer target in the user's
        // spec would beat `sea_orm=warn` if the cap were only another directive.
        let narrower = captured(filter("sea_orm::driver=trace,sqlx::query=trace,trace"));
        assert!(
            narrower.contains("APP-TRACE-STILL-WORKS") && !narrower.contains("DB-SENTINEL"),
            "a more specific database target cannot lift the cap either"
        );
    }

    #[test]
    fn without_the_cap_the_same_filter_would_log_database_values() {
        // Positive control: the sentinel events and span fields are visible to an uncapped filter.
        let text = captured(EnvFilter::new("trace"));
        assert!(text.contains("DB-SENTINEL-SQL") && text.contains("DB-SENTINEL-SQLX"));
        assert!(text.contains("DB-SENTINEL-SPAN"));
    }

    #[test]
    fn an_invalid_spec_falls_back_to_info_with_the_cap_intact() {
        let text = captured(filter("]]not a filter[["));
        assert!(!text.contains("DB-SENTINEL"));
    }
}
