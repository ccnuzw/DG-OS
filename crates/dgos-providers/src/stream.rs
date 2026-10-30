//! Protocol-neutral SSE driver. A [`Mapper`] turns decoded events of one protocol into chunks;
//! this module owns reading the body, the idle timeout, and the rule that a stream ends with
//! exactly one terminal item (the final chunk or one error), nothing after it.

use std::collections::VecDeque;
use std::time::Duration;

use dgos_types::{ChatChunk, DgosError, DgosResult};
use futures::stream;
use reqwest::Response;
use tracing::warn;

use crate::sse::{Decoded, SseDecoder, SseEvent};

pub enum Outcome {
    Continue,
    /// The stream finished normally; [`Mapper::terminal`] supplies the final chunk.
    Complete,
    Fail(DgosError),
}

pub trait Mapper: Send + 'static {
    /// Pushes the chunks this event carries (never an empty one) and says whether it ended the
    /// stream.
    fn event(&mut self, event: &SseEvent, out: &mut Vec<ChatChunk>) -> Outcome;
    /// The final chunk: finish reason and usage.
    fn terminal(&mut self) -> ChatChunk;
    /// The body ended without a terminal event. `Complete` only if the protocol already told us
    /// how the answer ended.
    fn at_eof(&mut self) -> Outcome;
}

struct State<M> {
    response: Response,
    decoder: SseDecoder,
    idle: Duration,
    pending: VecDeque<DgosResult<ChatChunk>>,
    mapper: M,
    /// A terminal item is queued; nothing further is read.
    done: bool,
}

impl<M: Mapper> State<M> {
    fn apply(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Continue => {}
            Outcome::Complete => {
                let terminal = self.mapper.terminal();
                self.pending.push_back(Ok(terminal));
                self.done = true;
            }
            Outcome::Fail(error) => self.fail(error),
        }
    }

    /// Queues the terminal error. A stream that already has its terminal item ignores it.
    fn fail(&mut self, error: DgosError) {
        if self.done {
            return;
        }
        self.pending.push_back(Err(error));
        self.done = true;
    }

    fn handle(&mut self, events: Vec<SseEvent>) {
        for event in events {
            if self.done {
                break;
            }
            let mut chunks = Vec::new();
            let outcome = self.mapper.event(&event, &mut chunks);
            self.pending.extend(chunks.into_iter().map(Ok));
            self.apply(outcome);
        }
    }

    /// Events that were decoded before the decoder failed are still delivered, then the error.
    fn absorb(&mut self, decoded: Decoded) {
        self.handle(decoded.events);
        if let Some(error) = decoded.error {
            self.fail(error);
        }
    }

    fn finish_eof(&mut self) {
        let decoded = self.decoder.finish();
        self.absorb(decoded);
        if self.done {
            return;
        }
        let outcome = self.mapper.at_eof();
        self.apply(outcome);
    }
}

pub fn sse_stream<M: Mapper>(
    response: Response,
    idle: Duration,
    max_event_bytes: usize,
    mapper: M,
) -> impl futures::Stream<Item = DgosResult<ChatChunk>> + Send {
    let state = State {
        response,
        decoder: SseDecoder::new(max_event_bytes),
        idle,
        pending: VecDeque::new(),
        mapper,
        done: false,
    };
    stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.pending.pop_front() {
                return Some((item, state));
            }
            if state.done {
                return None;
            }
            match tokio::time::timeout(state.idle, state.response.chunk()).await {
                Err(_) => state.fail(DgosError::Timeout),
                Ok(Err(_)) => {
                    warn!("Provider stream interrupted");
                    state.fail(DgosError::StreamInterrupted);
                }
                Ok(Ok(Some(bytes))) => {
                    let decoded = state.decoder.push(&bytes);
                    state.absorb(decoded);
                }
                Ok(Ok(None)) => state.finish_eof(),
            }
        }
    })
}
