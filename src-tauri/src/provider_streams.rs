//! Renderer-initiated provider streams (FR-001 AC33).
//!
//! The registry maps a host-issued `stream_id` to the window label that started it and the token
//! that stops it. A window can cancel only its own streams; an unknown, finished or foreign id
//! answers `false` and changes nothing. [`pump`] turns upstream chunks into
//! [`ProviderStreamEvent`]s on the caller's Channel; exactly one terminal event ends a stream.

use dgos_providers::BoxStream;
use dgos_types::{ChatChunk, DgosError, ProviderStreamEvent};
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

struct Entry {
    owner: String,
    token: CancellationToken,
}

#[derive(Default)]
pub struct ProviderStreams {
    entries: Mutex<HashMap<String, Entry>>,
}

impl ProviderStreams {
    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Registers a stream for `owner` (a window label) and returns its id and cancellation token.
    pub fn register(&self, owner: &str) -> (String, CancellationToken) {
        let id = uuid::Uuid::new_v4().to_string();
        let token = CancellationToken::new();
        self.entries().insert(
            id.clone(),
            Entry {
                owner: owner.to_owned(),
                token: token.clone(),
            },
        );
        (id, token)
    }

    /// Cancels `stream_id` if `owner` started it and it is still running.
    pub fn cancel(&self, owner: &str, stream_id: &str) -> bool {
        let mut entries = self.entries();
        match entries.get(stream_id) {
            Some(entry) if entry.owner == owner => {
                if let Some(entry) = entries.remove(stream_id) {
                    entry.token.cancel();
                }
                true
            }
            _ => false,
        }
    }

    /// Cancels every stream a window started (the window was destroyed or its page reloaded).
    pub fn cancel_owner(&self, owner: &str) -> usize {
        let mut entries = self.entries();
        let ids: Vec<String> = entries
            .iter()
            .filter(|(_, entry)| entry.owner == owner)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &ids {
            if let Some(entry) = entries.remove(id) {
                entry.token.cancel();
            }
        }
        ids.len()
    }

    /// Forgets a stream that ended on its own.
    pub fn finish(&self, stream_id: &str) {
        self.entries().remove(stream_id);
    }

    #[cfg(test)]
    fn active(&self) -> usize {
        self.entries().len()
    }
}

fn error_event(error: &DgosError) -> ProviderStreamEvent {
    ProviderStreamEvent::Error {
        code: error.error_code(),
        message: error.to_string(),
    }
}

/// Forwards `stream` through `send` until a terminal event. `send` returns `false` when nobody
/// can receive any more (the page is gone); the stream is then dropped, which closes the upstream
/// response and lets the usage record settle as interrupted. A stream that ends without a
/// finish reason is reported as one `stream_interrupted` error.
pub async fn pump(
    mut stream: BoxStream<ChatChunk>,
    mut send: impl FnMut(ProviderStreamEvent) -> bool,
) {
    loop {
        match stream.next().await {
            Some(Ok(chunk)) => {
                for event in ProviderStreamEvent::from_chunk(chunk) {
                    let terminal = event.is_terminal();
                    if !send(event) || terminal {
                        return;
                    }
                }
            }
            Some(Err(error)) => {
                send(error_event(&error));
                return;
            }
            None => {
                send(error_event(&DgosError::StreamInterrupted));
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_types::TokenUsage;
    use futures::stream;

    fn chunk(delta: &str) -> ChatChunk {
        ChatChunk {
            delta: delta.into(),
            ..Default::default()
        }
    }

    fn last(finish: &str) -> ChatChunk {
        ChatChunk {
            finish_reason: Some(finish.into()),
            usage: Some(TokenUsage {
                prompt_tokens: 4,
                completion_tokens: 2,
                total_tokens: 6,
            }),
            ..Default::default()
        }
    }

    async fn collect(items: Vec<Result<ChatChunk, DgosError>>) -> Vec<ProviderStreamEvent> {
        let mut seen = Vec::new();
        pump(Box::pin(stream::iter(items)), |event| {
            seen.push(event);
            true
        })
        .await;
        seen
    }

    #[test]
    fn provider_stream_cancel_only_reaches_the_window_that_started_it() {
        let streams = ProviderStreams::default();
        let (id, token) = streams.register("main");

        assert!(
            !streams.cancel("other", &id),
            "a foreign window cannot cancel"
        );
        assert!(!token.is_cancelled());
        assert!(!streams.cancel("main", "no-such-stream"));

        assert!(streams.cancel("main", &id));
        assert!(token.is_cancelled());
        assert!(!streams.cancel("main", &id), "already cancelled");
        assert_eq!(streams.active(), 0);
    }

    #[test]
    fn provider_stream_finished_ids_answer_false_and_ids_are_unique() {
        let streams = ProviderStreams::default();
        let (first, _) = streams.register("main");
        let (second, token) = streams.register("main");
        assert_ne!(first, second);

        streams.finish(&second);
        assert!(!streams.cancel("main", &second));
        assert!(!token.is_cancelled(), "finishing is not cancelling");
        assert_eq!(streams.active(), 1);
    }

    #[test]
    fn provider_stream_owner_cleanup_cancels_only_that_windows_streams() {
        let streams = ProviderStreams::default();
        let (_, mine_a) = streams.register("main");
        let (_, mine_b) = streams.register("main");
        let (other_id, theirs) = streams.register("other");

        assert_eq!(streams.cancel_owner("main"), 2);
        assert!(mine_a.is_cancelled() && mine_b.is_cancelled());
        assert!(!theirs.is_cancelled());
        assert!(streams.cancel("other", &other_id));
    }

    #[tokio::test]
    async fn provider_stream_events_arrive_in_order_and_nothing_follows_done() {
        let events = collect(vec![
            Ok(chunk("he")),
            Ok(ChatChunk {
                reasoning_delta: Some("hmm".into()),
                ..Default::default()
            }),
            Ok(chunk("llo")),
            Ok(last("stop")),
            // Would be a protocol violation upstream; the pump must not forward it.
            Ok(chunk("late")),
        ])
        .await;

        assert_eq!(
            events,
            vec![
                ProviderStreamEvent::Delta { text: "he".into() },
                ProviderStreamEvent::ReasoningDelta { text: "hmm".into() },
                ProviderStreamEvent::Delta { text: "llo".into() },
                ProviderStreamEvent::Done {
                    finish_reason: "stop".into(),
                    usage: TokenUsage {
                        prompt_tokens: 4,
                        completion_tokens: 2,
                        total_tokens: 6
                    },
                },
            ]
        );
    }

    #[tokio::test]
    async fn provider_stream_error_is_one_terminal_event_with_the_interrupt_code() {
        let events = collect(vec![
            Ok(chunk("partial")),
            Err(DgosError::StreamInterrupted),
            Ok(chunk("never")),
        ])
        .await;

        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            ProviderStreamEvent::Delta {
                text: "partial".into()
            }
        );
        match &events[1] {
            ProviderStreamEvent::Error { code, .. } => assert_eq!(*code, 1001007),
            other => panic!("expected an error event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn provider_stream_ending_without_a_finish_reason_is_reported_as_interrupted() {
        let events = collect(vec![Ok(chunk("cut off"))]).await;
        assert!(matches!(
            events.last(),
            Some(ProviderStreamEvent::Error { code: 1001007, .. })
        ));
        assert_eq!(events.iter().filter(|e| e.is_terminal()).count(), 1);
    }

    #[tokio::test]
    async fn provider_stream_stops_reading_when_the_receiver_is_gone() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let polled = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = polled.clone();
        let source = stream::iter(vec![Ok(chunk("a")), Ok(chunk("b")), Ok(last("stop"))]).inspect(
            move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
            },
        );
        let mut delivered = 0;
        pump(Box::pin(source), |_| {
            delivered += 1;
            false
        })
        .await;

        assert_eq!(delivered, 1);
        assert_eq!(polled.load(Ordering::SeqCst), 1);
    }
}
