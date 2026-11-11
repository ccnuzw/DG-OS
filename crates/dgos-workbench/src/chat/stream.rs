//! Read-only recovery at the exact sequence boundary of the live event stream.
use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct ChatLiveSnapshot {
    pub seq: u64,
    pub round: u32,
    pub text: String,
    pub reasoning: String,
    pub phase: Option<dgos_types::AiPhase>,
    pub proposal: Option<ChatToolProposal>,
    pub activities: Vec<ChatRunStep>,
    pub terminal: Option<ChatEvent>,
}

#[derive(Debug)]
pub(super) struct StreamState {
    snapshot: ChatLiveSnapshot,
    closed: bool,
}
impl Default for StreamState {
    fn default() -> Self {
        Self { snapshot: ChatLiveSnapshot { seq: 0, round: 1, text: String::new(), reasoning: String::new(), phase: None, proposal: None, activities: Vec::new(), terminal: None }, closed: false }
    }
}

#[derive(Serialize)]
pub struct ChatRunSnapshot {
    #[serde(flatten)]
    pub run: ChatRun,
    pub live_snapshot: Option<ChatLiveSnapshot>,
}

impl ChatManager {
    pub async fn get_run_snapshot(&self, user: &str, input: GetChatRunInput) -> Result<ChatRunSnapshot> {
        self.check_lease()?;
        // Keep the stream alive across the database read, including concurrent finalization.
        let active = self.registry().get(&input.task_id).filter(|entry| entry.user == user).cloned();
        let run = self.get_run(user, input).await?;
        let live_snapshot = active.and_then(|active| {
            let state = active.stream.lock().unwrap_or_else(|e| e.into_inner());
            (run.status == "running" && state.snapshot.seq > 0).then(|| state.snapshot.clone())
        });
        self.check_lease()?;
        Ok(ChatRunSnapshot { run, live_snapshot })
    }
}

/// Sequence assignment, projection and delivery share one lock. Database checkpoints are
/// deliberately not used as stream watermarks: their asynchronous writer can lag by a round.
pub(super) struct OrderedChatSink<S> { inner: S, state: Arc<Mutex<StreamState>> }
impl<S> OrderedChatSink<S> {
    pub(super) fn new(inner: S, state: Arc<Mutex<StreamState>>) -> Self { Self { inner, state } }
    fn forward(&self, sink: &dyn ChatSink, mut event: ChatEvent) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed { return false; }
        let snapshot = &mut state.snapshot;
        snapshot.seq += 1;
        *event.sequence() = snapshot.seq;
        match &event {
            ChatEvent::Delta { text, .. } => append(&mut snapshot.text, text),
            ChatEvent::ReasoningDelta { text, .. } => append(&mut snapshot.reasoning, text),
            ChatEvent::ModelSegment { round, .. } => {
                snapshot.round = *round;
                snapshot.text.clear();
                snapshot.reasoning.clear();
                snapshot.proposal = None;
            }
            ChatEvent::Phase { phase, .. } => {
                snapshot.phase = Some(*phase);
                if *phase != dgos_types::AiPhase::AwaitingConfirmation { snapshot.proposal = None; }
            }
            ChatEvent::ToolProposal { proposal, .. } => snapshot.proposal = Some(proposal.clone()),
            ChatEvent::Activity { step, .. } => {
                if let Some(previous) = snapshot.activities.iter_mut().find(|previous| previous.id == step.id) { *previous = step.clone(); }
                else { snapshot.activities.push(step.clone()); }
            }
            _ => {}
        }
        if event.terminal() { snapshot.terminal = Some(event.clone()); }
        state.closed = event.terminal();
        let sent = sink.send(event);
        state.closed |= !sent;
        sent
    }
}
fn append(target: &mut String, text: &str) {
    let (piece, _) = split_at_boundary(text, MAX_REPLY_BYTES.saturating_sub(target.len()));
    target.push_str(piece);
}
impl ChatSink for OrderedChatSink<Arc<dyn ChatSink>> {
    fn send(&self, event: ChatEvent) -> bool { self.forward(&*self.inner, event) }
}
impl ChatSink for OrderedChatSink<&dyn ChatSink> {
    fn send(&self, event: ChatEvent) -> bool { self.forward(self.inner, event) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_replaces_round_text_at_the_same_sequence_as_delivery() {
        let state = Arc::new(Mutex::new(StreamState::default()));
        let sink: Arc<dyn ChatSink> = Arc::new(|_| true);
        let ordered = OrderedChatSink::new(sink, state.clone());
        assert!(ordered.send(ChatEvent::Delta { seq: 0, assistant_message_id: "answer".into(), text: "Long old plan".into() }));
        assert!(ordered.send(ChatEvent::ReasoningDelta { seq: 0, assistant_message_id: "answer".into(), text: "private thought".into() }));
        assert!(ordered.send(ChatEvent::ModelSegment { seq: 0, task_id: "run".into(), assistant_message_id: "answer".into(), round: 2 }));
        assert!(ordered.send(ChatEvent::Delta { seq: 0, assistant_message_id: "answer".into(), text: "Final".into() }));
        let snapshot = state.lock().unwrap().snapshot.clone();
        assert_eq!(snapshot.seq, 4);
        assert_eq!(snapshot.round, 2);
        assert_eq!(snapshot.text, "Final");
        assert_eq!(snapshot.reasoning, "");
        assert!(ordered.send(error_event("answer", None, "task_cancelled".into(), false, false)));
        assert!(!ordered.send(ChatEvent::Delta { seq: 0, assistant_message_id: "answer".into(), text: "late".into() }));
        let state = state.lock().unwrap();
        assert_eq!(state.snapshot.seq, 5);
        assert!(matches!(state.snapshot.terminal, Some(ChatEvent::Error { seq: 5, .. })));
        assert_eq!(state.snapshot.text, "Final");
    }
}
