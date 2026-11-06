//! Process logging. Verbosity follows `RUST_LOG` (default `info`); independently, the database
//! targets (`dgos_db::QUIET_LOG_TARGETS`) are capped at `warn` before consulting user verbosity.
//! Their `trace` spans carry bound statement values, which include chat text. Callsite interests
//! are shared across dispatchers, so this filter requests a subscriber-local check for every
//! event and span, including when another subscriber enables the same callsite.

use tracing::{level_filters::LevelFilter, span, subscriber::Interest, Metadata};
use tracing_subscriber::filter::{EnvFilter, Targets};
use tracing_subscriber::layer::{Context, Filter, SubscriberExt};
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

struct LogFilter {
    verbosity: EnvFilter,
    cap: Targets,
}

impl<S> Filter<S> for LogFilter
where
    S: tracing::Subscriber,
{
    fn enabled(&self, meta: &Metadata<'_>, cx: &Context<'_, S>) -> bool {
        self.cap.would_enable(meta.target(), meta.level())
            && self.verbosity.enabled(meta, cx.clone())
    }

    fn callsite_enabled(&self, meta: &'static Metadata<'static>) -> Interest {
        // EnvFilter must still register dynamic span/field matchers. Never cache `always`:
        // an uncapped dispatcher may concurrently register this very same callsite.
        <EnvFilter as Filter<S>>::callsite_enabled(&self.verbosity, meta);
        Interest::sometimes()
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        self.verbosity.max_level_hint()
    }

    fn event_enabled(&self, event: &tracing::Event<'_>, cx: &Context<'_, S>) -> bool {
        self.enabled(event.metadata(), cx)
    }

    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, cx: Context<'_, S>) {
        <EnvFilter as Filter<S>>::on_new_span(&self.verbosity, attrs, id, cx);
    }

    fn on_record(&self, id: &span::Id, values: &span::Record<'_>, cx: Context<'_, S>) {
        <EnvFilter as Filter<S>>::on_record(&self.verbosity, id, values, cx);
    }

    fn on_enter(&self, id: &span::Id, cx: Context<'_, S>) {
        <EnvFilter as Filter<S>>::on_enter(&self.verbosity, id, cx);
    }

    fn on_exit(&self, id: &span::Id, cx: Context<'_, S>) {
        <EnvFilter as Filter<S>>::on_exit(&self.verbosity, id, cx);
    }

    fn on_close(&self, id: span::Id, cx: Context<'_, S>) {
        <EnvFilter as Filter<S>>::on_close(&self.verbosity, id, cx);
    }
}

pub fn filter<S>(spec: &str) -> impl Filter<S>
where
    S: tracing::Subscriber,
{
    LogFilter {
        verbosity: verbosity(spec),
        cap: database_cap(),
    }
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
        captured_after(filter, || {})
    }

    fn captured_after(
        filter: impl Filter<tracing_subscriber::Registry> + Send + Sync + 'static,
        before_events: impl FnOnce(),
    ) -> String {
        let buffer = Buffer::default();
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(buffer.clone())
            .with_filter(filter);
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            before_events();
            tracing::trace!(target: "sea_orm::driver::sqlx_sqlite", values = "DB-SENTINEL-SQL", "statement");
            tracing::debug!(target: "sqlx::query", values = "DB-SENTINEL-SQLX", "statement");
            // The real leak path: sea-orm's trace span records the statement, and the fmt layer
            // prints enabled span fields with every event logged inside it.
            let span = tracing::trace_span!(target: "sea_orm::driver::sqlx_sqlite", "sqlx_sqlite", stmt = "DB-SENTINEL-SPAN");
            span.in_scope(|| tracing::trace!(target: "dgos_lib", "event inside a database span"));
            tracing::trace!(target: "dgos_lib", marker = "APP-TRACE-STILL-WORKS", "app event");
            tracing::debug!(target: "dgos_lib", marker = "APP-DEBUG", "app debug event");
            tracing::info!(target: "dgos_lib", marker = "APP-INFO", "app info event");
            tracing::warn!(target: "sqlx::query", marker = "DB-WARN", "database warning");
            let app_span = tracing::trace_span!(target: "dgos_lib", "app_diagnostic", selected = true);
            app_span.in_scope(|| {
                tracing::trace!(target: "dgos_lib", marker = "APP-DYNAMIC-TRACE", "scoped event");
                tracing::debug!(target: "sqlx::query", values = "DB-SENTINEL-DYNAMIC", "scoped statement");
            });
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
        assert!(!text.contains("DB-SENTINEL"));
        assert!(text.contains("DB-WARN"));
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
        let dynamic = captured(filter("info,dgos_lib[app_diagnostic{selected=true}]=trace"));
        assert!(dynamic.contains("APP-DYNAMIC-TRACE"));
        assert!(!dynamic.contains("APP-TRACE-STILL-WORKS"));
        assert!(!dynamic.contains("DB-SENTINEL"));
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
        assert!(text.contains("APP-INFO") && text.contains("DB-WARN"));
        assert!(!text.contains("APP-DEBUG") && !text.contains("APP-TRACE-STILL-WORKS"));
    }

    #[test]
    fn privacy_decisions_are_never_cached_as_unconditional_interest() {
        use tracing::callsite::Callsite;

        let callsite = tracing::callsite! {
            name: "private statement",
            kind: tracing::metadata::Kind::SPAN,
            target: "sea_orm::driver::sqlx_sqlite",
            level: tracing::Level::TRACE,
            fields: stmt,
        };
        let privacy = filter::<tracing_subscriber::Registry>("trace");
        assert!(privacy.callsite_enabled(callsite.metadata()).is_sometimes());
    }

    #[test]
    fn concurrent_uncapped_subscribers_cannot_enable_private_callsites() {
        // Keep all dispatchers alive while the shared macro callsites are first registered and
        // used. This exercises cache rebuilding and subscriber-local filtering without a lock
        // that could hide a production leak.
        for _ in 0..32 {
            let ready = std::sync::Barrier::new(3);
            std::thread::scope(|scope| {
                let uncapped = scope.spawn(|| {
                    captured_after(EnvFilter::new("trace"), || {
                        ready.wait();
                    })
                });
                let capped = scope.spawn(|| {
                    captured_after(filter("trace,sea_orm::driver=trace,sqlx::query=trace"), || {
                        ready.wait();
                    })
                });
                let fallback = captured_after(filter("]]not a filter[["), || {
                    ready.wait();
                });
                let uncapped = uncapped.join().unwrap();
                let capped = capped.join().unwrap();
                assert!(uncapped.contains("DB-SENTINEL-SQL"));
                assert!(uncapped.contains("DB-SENTINEL-SPAN"));
                assert!(capped.contains("APP-TRACE-STILL-WORKS"));
                assert!(!capped.contains("DB-SENTINEL"), "{capped}");
                assert!(fallback.contains("APP-INFO"));
                assert!(!fallback.contains("APP-DEBUG"));
                assert!(!fallback.contains("DB-SENTINEL"), "{fallback}");
            });
        }
    }
}
