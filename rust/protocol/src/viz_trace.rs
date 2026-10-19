//! Opt-in, thread-local observations of the classical Double Ratchet.
//!
//! Fingerprints are short teaching aids, not cryptographic identifiers. Events
//! include attempted operations; a failed delivery does not commit session state.

use std::cell::RefCell;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::SessionRecord;

#[derive(Clone, Debug, Serialize)]
pub struct VizEvent {
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_fp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
}

impl VizEvent {
    pub(crate) fn new(kind: &'static str) -> Self {
        Self {
            kind,
            chain: None,
            from: None,
            to: None,
            index: None,
            fp: None,
            old_fp: None,
            stage: None,
            status: None,
            source: None,
            detail: None,
            candidate: None,
            candidate_kind: None,
            ok: None,
        }
    }
}

#[derive(Clone, Copy)]
struct Candidate {
    id: u32,
    kind: &'static str,
}

#[derive(Default)]
struct Recording {
    events: Vec<VizEvent>,
    next_candidate: u32,
    candidate: Option<Candidate>,
}

thread_local! {
    static RECORDER: RefCell<Option<Recording>> = const { RefCell::new(None) };
}

/// Start a fresh recording on this thread, discarding any previous recording.
///
/// The recorder is thread-local: call `start`, the libsignal operation and
/// `take` on the same thread with no `.await` that could move the task in
/// between (viz/server drives each call with `now_or_never`).
pub fn start() {
    RECORDER.with_borrow_mut(|recorder| *recorder = Some(Recording::default()));
}

/// Stop recording on this thread and return its events.
pub fn take() -> Vec<VizEvent> {
    RECORDER.with_borrow_mut(|recorder| recorder.take().map(|r| r.events).unwrap_or_default())
}

pub(crate) fn record(mut event: VizEvent) {
    RECORDER.with_borrow_mut(|recorder| {
        if let Some(recording) = recorder {
            if let Some(candidate) = recording.candidate {
                event.candidate = Some(candidate.id);
                event.candidate_kind = Some(candidate.kind);
            }
            recording.events.push(event);
        }
    });
}

/// Record the start of a diagnostic stage, including stages in the viz server.
pub fn stage_started(stage: &'static str, source: &'static str) {
    let mut event = VizEvent::new("decrypt_stage");
    event.stage = Some(stage);
    event.status = Some("started");
    event.source = Some(source);
    record(event);
}

/// Record the original operation's result without replacing its error.
pub fn stage_finished<T, E: std::fmt::Display>(
    stage: &'static str,
    source: &'static str,
    result: &std::result::Result<T, E>,
) {
    let mut event = VizEvent::new("decrypt_stage");
    event.stage = Some(stage);
    event.source = Some(source);
    match result {
        Ok(_) => event.status = Some("ok"),
        Err(error) => {
            event.status = Some("error");
            event.detail = Some(error.to_string());
        }
    }
    record(event);
}

// Candidate IDs are local to one start()/take() recording, not message counters.
pub(crate) struct CandidateGuard {
    previous: Option<Candidate>,
}

pub(crate) fn candidate(kind: &'static str) -> Option<CandidateGuard> {
    RECORDER.with_borrow_mut(|recorder| {
        let recording = recorder.as_mut()?;
        let previous = recording.candidate;
        let id = recording.next_candidate;
        recording.next_candidate += 1;
        recording.candidate = Some(Candidate { id, kind });
        Some(CandidateGuard { previous })
    })
}

impl Drop for CandidateGuard {
    fn drop(&mut self) {
        RECORDER.with_borrow_mut(|recorder| {
            if let Some(recording) = recorder {
                recording.candidate = self.previous;
            }
        });
    }
}

// A failed trial may be followed by a successful archived session. Call this
// only at a terminal delivery failure, once all eligible sessions were tried.
pub(crate) fn record_mac_rejection(errors: &[crate::SignalProtocolError]) {
    if errors.iter().any(|error| matches!(error,
        crate::SignalProtocolError::InvalidMessage(_, reason) if reason == "MAC verification failed"
    )) {
        record(VizEvent::new("mac_rejected"));
    }
}

pub fn fp(bytes: &[u8]) -> String {
    hex::encode(&Sha256::digest(bytes)[..4])
}

#[derive(Serialize)]
pub struct KeyBoxView {
    pub id: String,
    // Skipped-key labels contain a runtime counter, so they must be owned.
    pub label: String,
    pub fp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

/// Snapshot only the current session, without mutating it or exposing key bytes.
pub fn session_boxes(record: &SessionRecord) -> Vec<KeyBoxView> {
    let Some(state) = record.session_state() else {
        return Vec::new();
    };
    let state = state.viz_structure();
    let mut boxes = vec![KeyBoxView {
        id: "root".into(),
        label: "ルート鍵".into(),
        fp: fp(&state.root_key),
        index: None,
    }];
    if let Some(sender) = &state.sender_chain {
        boxes.push(KeyBoxView {
            id: "dh".into(),
            label: "自分のDH鍵".into(),
            fp: fp(&sender.sender_ratchet_key),
            index: None,
        });
        if let Some(chain) = &sender.chain_key {
            boxes.push(KeyBoxView {
                id: "send".into(),
                label: "送信の鎖".into(),
                fp: fp(&chain.key),
                index: Some(chain.index),
            });
        }
    }
    for receiver in &state.receiver_chains {
        let ratchet_fp = fp(&receiver.sender_ratchet_key);
        if let Some(chain) = &receiver.chain_key {
            boxes.push(KeyBoxView {
                id: format!("recv:{ratchet_fp}"),
                label: "受信の鎖".into(),
                fp: fp(&chain.key),
                index: Some(chain.index),
            });
        }
    }
    for receiver in &state.receiver_chains {
        let ratchet_fp = fp(&receiver.sender_ratchet_key);
        for key in &receiver.message_keys {
            let bytes = if key.seed.is_empty() {
                &key.cipher_key
            } else {
                &key.seed
            };
            boxes.push(KeyBoxView {
                id: format!("skipped:{ratchet_fp}:{}", key.index),
                label: format!("取り置きの鍵 #{}", key.index),
                fp: fp(bytes),
                index: None,
            });
        }
    }
    boxes
}

/// Snapshot the current Sender Key chain and stored message keys without mutation.
/// The server supplies participant-specific labels and sender identifiers.
pub fn sender_key_boxes(record: &crate::SenderKeyRecord) -> Vec<KeyBoxView> {
    let Ok(state) = record.sender_key_state() else {
        return Vec::new();
    };
    let state = state.as_protobuf();
    let mut boxes = Vec::new();
    if let Some(chain) = state.sender_chain_key {
        boxes.push(KeyBoxView {
            id: "sk".into(),
            label: "Sender Key".into(),
            fp: fp(&chain.seed),
            index: Some(chain.iteration),
        });
    }
    for key in state.sender_message_keys {
        boxes.push(KeyBoxView {
            id: format!("skipped:{}", key.iteration),
            label: format!("取り置きの鍵 #{}", key.iteration),
            fp: fp(&key.seed),
            index: None,
        });
    }
    boxes
}
