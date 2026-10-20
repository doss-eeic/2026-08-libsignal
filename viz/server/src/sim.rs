use std::time::SystemTime;

use anyhow::{Result, anyhow};
use futures::FutureExt;
use libsignal_protocol::{
    CiphertextMessage, CiphertextMessageType, DeviceId, GenericSignedPreKey, IdentityKeyPair,
    IdentityKeyStore, InMemSessionStore, InMemSignalProtocolStore, KeyPair, KyberPreKeyRecord,
    KyberPreKeyStore, PreKeyBundle, PreKeyRecord, PreKeySignalMessage, PreKeyStore,
    ProtocolAddress, SessionStore, SignalMessage, SignalProtocolError, SignedPreKeyRecord,
    SignedPreKeyStore, Timestamp, kem, message_decrypt, message_encrypt, process_prekey_bundle,
};
use rand::{CryptoRng, Rng, TryRngCore as _, rngs::OsRng};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Who {
    Alice,
    Bob,
}

impl Who {
    fn other(self) -> Self {
        match self {
            Self::Alice => Self::Bob,
            Self::Bob => Self::Alice,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActionType {
    Send,
    Deliver,
    Tamper,
    Replay,
}

/// The experiment changes a wire copy or an isolated receiver store, never the original.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TamperMode {
    #[default]
    BodyBitFlip,
    Truncate,
    NoSession,
    UntrustedIdentity,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Ok,
    Duplicate,
    BadMac,
    TooShort,
    NoSession,
    UntrustedIdentity,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Expect {
    Ok,
    Duplicate,
    BadMac,
    TooShort,
    NoSession,
    UntrustedIdentity,
    Error,
    AnyError,
}
impl From<Outcome> for Expect {
    fn from(value: Outcome) -> Self {
        match value {
            Outcome::Ok => Self::Ok,
            Outcome::Duplicate => Self::Duplicate,
            Outcome::BadMac => Self::BadMac,
            Outcome::TooShort => Self::TooShort,
            Outcome::NoSession => Self::NoSession,
            Outcome::UntrustedIdentity => Self::UntrustedIdentity,
            Outcome::Error => Self::Error,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Event {
    #[serde(flatten)]
    pub event: libsignal_protocol::viz_trace::VizEvent,
    pub who: Who,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Action {
    pub i: usize,
    #[serde(rename = "type")]
    pub action_type: ActionType,
    pub who: Who,
    pub message_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_now: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tamper_mode: Option<TamperMode>,
    pub outcome: Outcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub expect: Expect,
    pub events: Vec<Event>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Message {
    pub id: u32,
    pub from: Who,
    pub to: Who,
    pub text: String,
    pub counter: u32,
    pub kind: String,
    pub ciphertext_hex: String,
    pub ciphertext_len: usize,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_order: Option<u32>,
}
struct StoredMessage {
    view: Message,
    bytes: Vec<u8>,
    ty: CiphertextMessageType,
}

#[derive(Serialize)]
pub(crate) struct State {
    pub alice: Vec<libsignal_protocol::viz_trace::KeyBoxView>,
    pub bob: Vec<libsignal_protocol::viz_trace::KeyBoxView>,
    pub messages: Vec<Message>,
    pub script: Vec<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<Action>,
}

pub(crate) struct Sim {
    alice: InMemSignalProtocolStore,
    bob: InMemSignalProtocolStore,
    alice_addr: ProtocolAddress,
    bob_addr: ProtocolAddress,
    messages: Vec<StoredMessage>,
    pub script: Vec<Action>,
    delivered: u32,
}

impl Sim {
    pub(crate) fn new() -> Result<Self> {
        let mut rng = OsRng.unwrap_err();
        let mut alice = store(&mut rng)?;
        let mut bob = store(&mut rng)?;
        let alice_addr = ProtocolAddress::new("alice".into(), DeviceId::new(1).unwrap());
        let bob_addr = ProtocolAddress::new("bob".into(), DeviceId::new(1).unwrap());
        let bundle = bundle(&mut bob, &mut rng)?;
        libsignal_protocol::viz_trace::start();
        process_prekey_bundle(
            &bob_addr,
            &alice_addr,
            &mut alice.session_store,
            &mut alice.identity_store,
            &bundle,
            SystemTime::now(),
            &mut rng,
        )
        .now_or_never()
        .expect("sync")?;
        let _ = libsignal_protocol::viz_trace::take();
        Ok(Self {
            alice,
            bob,
            alice_addr,
            bob_addr,
            messages: vec![],
            script: vec![],
            delivered: 0,
        })
    }

    pub(crate) fn state(&self) -> State {
        State {
            alice: boxes(&self.alice, &self.bob_addr),
            bob: boxes(&self.bob, &self.alice_addr),
            messages: self.messages.iter().map(|m| m.view.clone()).collect(),
            script: self.script.clone(),
            last: self.script.last().cloned(),
        }
    }

    pub(crate) fn send(&mut self, from: Who, text: String, deliver_now: bool) -> Result<()> {
        let id = self.messages.len() as u32 + 1;
        let to = from.other();
        libsignal_protocol::viz_trace::start();
        let result = self.encrypt(from, text.as_bytes());
        let mut events = events(from);
        let message = result?;
        let ty = message.message_type();
        let bytes = message.serialize().to_vec();
        let signal = parse(ty, &bytes)?;
        let counter = match &signal {
            CiphertextMessage::SignalMessage(m) => m.counter(),
            CiphertextMessage::PreKeySignalMessage(m) => m.message().counter(),
            _ => 0,
        };
        self.messages.push(StoredMessage {
            view: Message {
                id,
                from,
                to,
                text: text.clone(),
                counter,
                kind: if ty == CiphertextMessageType::PreKey {
                    "prekey".into()
                } else {
                    "whisper".into()
                },
                ciphertext_hex: hex_prefix(&bytes),
                ciphertext_len: bytes.len(),
                status: "waiting".into(),
                delivered_order: None,
            },
            bytes,
            ty,
        });
        let (outcome, error) = if deliver_now {
            let (out, err, ev) = self.decrypt_index(id, None)?;
            events.extend(ev);
            if out == Outcome::Ok {
                self.mark_delivered(id);
            }
            (out, err)
        } else {
            (Outcome::Ok, None)
        };
        self.push(
            ActionType::Send,
            from,
            id,
            Some(text),
            Some(deliver_now),
            outcome,
            error,
            events,
        );
        Ok(())
    }

    pub(crate) fn deliver(&mut self, id: u32) -> Result<()> {
        self.require_status(id, "waiting")?;
        let who = self.message(id)?.view.to;
        let (out, err, ev) = self.decrypt_index(id, None)?;
        if out == Outcome::Ok {
            self.mark_delivered(id);
        }
        self.push(ActionType::Deliver, who, id, None, None, out, err, ev);
        Ok(())
    }
    pub(crate) fn tamper(&mut self, id: u32) -> Result<()> {
        self.experiment(id, TamperMode::BodyBitFlip)
    }
    pub(crate) fn experiment(&mut self, id: u32, mode: TamperMode) -> Result<()> {
        self.require_status(id, "waiting")?;
        if matches!(mode, TamperMode::NoSession | TamperMode::UntrustedIdentity)
            && self.message(id)?.ty != CiphertextMessageType::Whisper
        {
            return Err(anyhow!(
                "この実験は通常メッセージ用です。Aliceの最初のメッセージを届け、Bobから返信してから、通常メッセージを待機させてください"
            ));
        }
        let who = self.message(id)?.view.to;
        let (out, err, ev) = self.decrypt_index(id, Some(mode))?;
        self.push(ActionType::Tamper, who, id, None, None, out, err, ev);
        self.script
            .last_mut()
            .expect("just appended experiment")
            .tamper_mode = Some(mode);
        Ok(())
    }
    pub(crate) fn replay(&mut self, id: u32) -> Result<()> {
        self.require_status(id, "delivered")?;
        let who = self.message(id)?.view.to;
        let (out, err, ev) = self.decrypt_index(id, None)?;
        self.push(ActionType::Replay, who, id, None, None, out, err, ev);
        Ok(())
    }
    pub(crate) fn set_expect(&mut self, i: usize, expect: Expect) -> Result<()> {
        self.script
            .get_mut(i)
            .ok_or_else(|| anyhow!("指定された操作が見つかりません"))?
            .expect = expect;
        Ok(())
    }

    fn encrypt(&mut self, who: Who, text: &[u8]) -> Result<CiphertextMessage> {
        let mut rng = OsRng.unwrap_err();
        let (store, remote, local) = self.party_mut(who);
        message_encrypt(
            text,
            remote,
            local,
            &mut store.session_store,
            &mut store.identity_store,
            SystemTime::now(),
            &mut rng,
        )
        .now_or_never()
        .expect("sync")
        .map_err(Into::into)
    }
    fn decrypt_index(
        &mut self,
        id: u32,
        mode: Option<TamperMode>,
    ) -> Result<(Outcome, Option<String>, Vec<Event>)> {
        let (who, ty, mut bytes) = {
            let stored = self.message(id)?;
            (stored.view.to, stored.ty, stored.bytes.clone())
        };
        match mode {
            Some(TamperMode::BodyBitFlip) => tamper_bytes(ty, &mut bytes),
            // Empty input consistently reaches CiphertextMessageTooShort for both formats.
            Some(TamperMode::Truncate) => bytes.clear(),
            _ => {}
        }
        libsignal_protocol::viz_trace::start();
        let result = (|| {
            libsignal_protocol::viz_trace::stage_started("parse", "sim::parse");
            let parsed = parse(ty, &bytes);
            libsignal_protocol::viz_trace::stage_finished("parse", "sim::parse", &parsed);
            let msg = parsed?;
            self.decrypt(who, &msg, mode)
        })();
        let ev = events(who);
        let saw_mac_rejection = ev.iter().any(|e| e.event.kind == "mac_rejected");
        let (out, err) = match result {
            Ok(_) => (Outcome::Ok, None),
            Err(e) => (classify(&e, saw_mac_rejection), Some(format!("{e:?}"))),
        };
        Ok((out, err, ev))
    }
    fn decrypt(
        &mut self,
        who: Who,
        msg: &CiphertextMessage,
        mode: Option<TamperMode>,
    ) -> Result<Vec<u8>> {
        let (store, remote, local) = self.party_mut(who);
        if matches!(
            mode,
            Some(TamperMode::NoSession | TamperMode::UntrustedIdentity)
        ) {
            let mut receiver = store.clone();
            match mode {
                Some(TamperMode::NoSession) => receiver.session_store = InMemSessionStore::new(),
                Some(TamperMode::UntrustedIdentity) => {
                    let other_identity = IdentityKeyPair::generate(&mut OsRng.unwrap_err());
                    receiver
                        .identity_store
                        .save_identity(remote, other_identity.identity_key())
                        .now_or_never()
                        .expect("sync")?;
                }
                _ => unreachable!("receiver-condition experiment"),
            }
            decrypt_with_store(&mut receiver, remote, local, msg)
        } else {
            decrypt_with_store(store, remote, local, msg)
        }
    }
    fn party_mut(
        &mut self,
        who: Who,
    ) -> (
        &mut InMemSignalProtocolStore,
        &ProtocolAddress,
        &ProtocolAddress,
    ) {
        match who {
            Who::Alice => (&mut self.alice, &self.bob_addr, &self.alice_addr),
            Who::Bob => (&mut self.bob, &self.alice_addr, &self.bob_addr),
        }
    }
    fn message(&self, id: u32) -> Result<&StoredMessage> {
        self.messages
            .get(
                (id.checked_sub(1)
                    .ok_or_else(|| anyhow!("メッセージが見つかりません"))?)
                    as usize,
            )
            .ok_or_else(|| anyhow!("メッセージが見つかりません"))
    }
    fn require_status(&self, id: u32, status: &str) -> Result<()> {
        if self.message(id)?.view.status != status {
            return Err(anyhow!("このメッセージにはその操作を行えません"));
        }
        Ok(())
    }
    fn mark_delivered(&mut self, id: u32) {
        self.delivered += 1;
        let m = &mut self.messages[(id - 1) as usize].view;
        m.status = "delivered".into();
        m.delivered_order = Some(self.delivered);
    }
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        action_type: ActionType,
        who: Who,
        message_id: u32,
        text: Option<String>,
        delivered_now: Option<bool>,
        outcome: Outcome,
        error: Option<String>,
        events: Vec<Event>,
    ) {
        self.script.push(Action {
            i: self.script.len(),
            action_type,
            who,
            message_id,
            text,
            delivered_now,
            tamper_mode: None,
            expect: outcome.clone().into(),
            outcome,
            error,
            events,
        });
    }
}

/// Both normal delivery and experiments run the unchanged public decryption API.
fn decrypt_with_store(
    store: &mut InMemSignalProtocolStore,
    remote: &ProtocolAddress,
    local: &ProtocolAddress,
    msg: &CiphertextMessage,
) -> Result<Vec<u8>> {
    let mut rng = OsRng.unwrap_err();
    message_decrypt(
        msg,
        remote,
        local,
        &mut store.session_store,
        &mut store.identity_store,
        &mut store.pre_key_store,
        &store.signed_pre_key_store,
        &mut store.kyber_pre_key_store,
        &mut rng,
    )
    .now_or_never()
    .expect("sync")
    .map_err(Into::into)
}

fn store(rng: &mut (impl Rng + CryptoRng)) -> Result<InMemSignalProtocolStore> {
    Ok(InMemSignalProtocolStore::new(
        IdentityKeyPair::generate(rng),
        rng.random::<u8>() as u32,
    )?)
}
fn bundle<R: Rng + CryptoRng>(
    store: &mut InMemSignalProtocolStore,
    rng: &mut R,
) -> Result<PreKeyBundle> {
    let pre = KeyPair::generate(rng);
    let signed = KeyPair::generate(rng);
    let kyber = kem::KeyPair::generate(kem::KeyType::Kyber1024, rng);
    let pre_id: u32 = rng.random();
    let signed_id: u32 = rng.random();
    let kyber_id: u32 = rng.random();
    let identity = store
        .get_identity_key_pair()
        .now_or_never()
        .expect("sync")?;
    let sig = identity
        .private_key()
        .calculate_signature(&signed.public_key.serialize(), rng)?;
    let kyber_sig = identity
        .private_key()
        .calculate_signature(&kyber.public_key.serialize(), rng)?;
    let bundle = PreKeyBundle::new(
        store
            .get_local_registration_id()
            .now_or_never()
            .expect("sync")?,
        DeviceId::new(1).unwrap(),
        Some((pre_id.into(), pre.public_key)),
        signed_id.into(),
        signed.public_key,
        sig.to_vec(),
        kyber_id.into(),
        kyber.public_key.clone(),
        kyber_sig.to_vec(),
        *identity.identity_key(),
    )?;
    store
        .save_pre_key(pre_id.into(), &PreKeyRecord::new(pre_id.into(), &pre))
        .now_or_never()
        .expect("sync")?;
    store
        .save_signed_pre_key(
            signed_id.into(),
            &SignedPreKeyRecord::new(
                signed_id.into(),
                Timestamp::from_epoch_millis(rng.random()),
                &signed,
                &sig,
            ),
        )
        .now_or_never()
        .expect("sync")?;
    store
        .save_kyber_pre_key(
            kyber_id.into(),
            &KyberPreKeyRecord::new(
                kyber_id.into(),
                Timestamp::from_epoch_millis(43),
                &kyber,
                &kyber_sig,
            ),
        )
        .now_or_never()
        .expect("sync")?;
    Ok(bundle)
}
fn parse(ty: CiphertextMessageType, bytes: &[u8]) -> Result<CiphertextMessage> {
    Ok(match ty {
        CiphertextMessageType::PreKey => {
            CiphertextMessage::PreKeySignalMessage(PreKeySignalMessage::try_from(bytes)?)
        }
        CiphertextMessageType::Whisper => {
            CiphertextMessage::SignalMessage(SignalMessage::try_from(bytes)?)
        }
        _ => return Err(anyhow!("unsupported ciphertext")),
    })
}
fn classify(e: &anyhow::Error, saw_mac_rejection: bool) -> Outcome {
    match e.downcast_ref::<SignalProtocolError>() {
        Some(SignalProtocolError::DuplicatedMessage(..)) => Outcome::Duplicate,
        Some(SignalProtocolError::CiphertextMessageTooShort(..)) => Outcome::TooShort,
        Some(SignalProtocolError::SessionNotFound(..)) => Outcome::NoSession,
        Some(SignalProtocolError::UntrustedIdentity(..)) => Outcome::UntrustedIdentity,
        Some(SignalProtocolError::InvalidMessage(..)) if saw_mac_rejection => Outcome::BadMac,
        _ => Outcome::Error,
    }
}
fn events(who: Who) -> Vec<Event> {
    libsignal_protocol::viz_trace::take()
        .into_iter()
        .map(|event| Event { event, who })
        .collect()
}
fn boxes(
    store: &InMemSignalProtocolStore,
    remote: &ProtocolAddress,
) -> Vec<libsignal_protocol::viz_trace::KeyBoxView> {
    store
        .session_store
        .load_session(remote)
        .now_or_never()
        .expect("sync")
        .ok()
        .flatten()
        .map(|r| libsignal_protocol::viz_trace::session_boxes(&r))
        .unwrap_or_default()
}
fn hex_prefix(bytes: &[u8]) -> String {
    bytes[..bytes.len().min(12)]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
/// Flip an encrypted payload byte while preserving the protobuf shape and MAC bytes.
pub(crate) fn tamper_bytes(ty: CiphertextMessageType, bytes: &mut [u8]) {
    let signal = if ty == CiphertextMessageType::PreKey {
        let (start, len) =
            field_bytes(&bytes[1..], 4).expect("PreKeySignalMessage contains SignalMessage");
        &mut bytes[1 + start..1 + start + len]
    } else {
        bytes
    };
    let (start, len) =
        field_bytes(&signal[1..signal.len() - 8], 4).expect("SignalMessage contains ciphertext");
    assert!(len > 0, "ciphertext must not be empty");
    signal[1 + start + len / 2] ^= 1;
}
fn field_bytes(input: &[u8], wanted: u32) -> Option<(usize, usize)> {
    let mut p = 0;
    while p < input.len() {
        let key = varint(input, &mut p)?;
        let wire = key & 7;
        let field = key >> 3;
        if wire == 2 {
            let len = varint(input, &mut p)? as usize;
            let start = p;
            p = p.checked_add(len)?;
            if field == wanted {
                return Some((start, len));
            }
        } else if wire == 0 {
            let _ = varint(input, &mut p)?;
        } else {
            return None;
        }
    }
    None
}
fn varint(input: &[u8], p: &mut usize) -> Option<u32> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let b = *input.get(*p)?;
        *p += 1;
        value |= u32::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scenario_and_unchanged_tamper() -> Result<()> {
        let mut s = Sim::new()?;
        s.send(Who::Alice, "one".into(), true)?;
        s.send(Who::Bob, "two".into(), true)?;
        s.send(Who::Alice, "three".into(), false)?;
        let before = serde_json::to_string(&s.state().bob)?;
        s.tamper(3)?;
        assert_eq!(s.script.last().unwrap().outcome, Outcome::BadMac);
        let last = s.script.last().unwrap();
        assert!(
            last.events
                .iter()
                .any(|e| { e.event.stage == Some("parse") && e.event.status == Some("ok") })
        );
        assert!(last.events.iter().any(|e| {
            e.event.stage == Some("authentication") && e.event.status == Some("error")
        }));
        assert!(
            !last
                .events
                .iter()
                .any(|e| e.event.stage == Some("session_save"))
        );
        assert_eq!(before, serde_json::to_string(&s.state().bob)?);
        s.deliver(3)?;
        assert!(
            s.script
                .last()
                .unwrap()
                .events
                .iter()
                .any(|e| { e.event.stage == Some("session_save") && e.event.status == Some("ok") })
        );
        s.replay(3)?;
        assert_eq!(s.script.last().unwrap().outcome, Outcome::Duplicate);
        Ok(())
    }
    #[test]
    fn prekey_tamper_is_a_mac_error() -> Result<()> {
        let mut s = Sim::new()?;
        s.send(Who::Alice, "first".into(), false)?;
        s.tamper(1)?;
        assert_eq!(s.script.last().unwrap().outcome, Outcome::BadMac);
        assert!(
            s.script.last().unwrap().events.iter().any(|e| {
                e.event.stage == Some("prekey_process") && e.event.status == Some("ok")
            })
        );
        assert_eq!(s.messages[0].view.status, "waiting");
        Ok(())
    }
    #[test]
    fn malformed_message_fails_before_decryption() -> Result<()> {
        let mut s = Sim::new()?;
        s.send(Who::Alice, "first".into(), false)?;
        s.messages[0].bytes.clear();
        s.deliver(1)?;
        let last = s.script.last().unwrap();
        assert_eq!(last.outcome, Outcome::TooShort);
        assert!(
            last.events
                .iter()
                .any(|e| { e.event.stage == Some("parse") && e.event.status == Some("error") })
        );
        assert!(!last.events.iter().any(|e| e.event.stage == Some("receive")));
        Ok(())
    }
    #[test]
    fn out_of_order_stores_and_uses_skipped_keys() -> Result<()> {
        let mut s = Sim::new()?;
        s.send(Who::Alice, "zero".into(), true)?;
        s.send(Who::Alice, "one".into(), false)?;
        s.send(Who::Alice, "two".into(), false)?;
        s.send(Who::Alice, "three".into(), false)?;
        s.deliver(4)?;
        assert_eq!(s.messages[3].view.delivered_order, Some(2));
        assert_eq!(s.script.last().unwrap().who, Who::Bob);
        assert_eq!(
            s.script
                .last()
                .unwrap()
                .events
                .iter()
                .filter(|e| e.event.kind == "skipped_key_stored")
                .count(),
            2
        );
        assert!(s.state().bob.iter().any(|k| k.id.starts_with("skipped:")));
        s.deliver(2)?;
        assert!(
            s.script
                .last()
                .unwrap()
                .events
                .iter()
                .any(|e| e.event.kind == "skipped_key_used")
        );
        assert_eq!(s.messages[1].view.delivered_order, Some(3));
        Ok(())
    }

    fn assert_stage(action: &Action, stage: &str, status: &str) {
        assert!(
            action
                .events
                .iter()
                .any(|e| { e.event.stage == Some(stage) && e.event.status == Some(status) }),
            "missing {stage}: {status}"
        );
    }

    #[test]
    fn experiments_fail_at_real_stages_and_preserve_original_receiver() -> Result<()> {
        for (mode, expected, failure_stage, not_reached) in [
            (TamperMode::Truncate, Outcome::TooShort, "parse", "receive"),
            (
                TamperMode::NoSession,
                Outcome::NoSession,
                "session_load",
                "key_prepare",
            ),
            (
                TamperMode::UntrustedIdentity,
                Outcome::UntrustedIdentity,
                "identity_trust",
                "identity_save",
            ),
        ] {
            let mut s = Sim::new()?;
            s.send(Who::Alice, "first".into(), true)?;
            s.send(Who::Bob, "reply".into(), true)?;
            s.send(Who::Alice, "original".into(), false)?;
            assert_eq!(s.messages[2].ty, CiphertextMessageType::Whisper);
            let before_session = s
                .bob
                .load_session(&s.alice_addr)
                .now_or_never()
                .expect("sync")?
                .unwrap()
                .serialize()?;
            let before_identity = s
                .bob
                .get_identity(&s.alice_addr)
                .now_or_never()
                .expect("sync")?;
            let before_bytes = s.messages[2].bytes.clone();
            let before_delivered = s.delivered;

            s.experiment(3, mode)?;
            let action = s.script.last().unwrap();
            assert_eq!(action.outcome, expected);
            assert_eq!(action.tamper_mode, Some(mode));
            assert_eq!(action.expect, expected.into());
            assert_stage(action, failure_stage, "error");
            assert!(
                !action
                    .events
                    .iter()
                    .any(|e| e.event.stage == Some(not_reached))
            );
            assert!(
                !action
                    .events
                    .iter()
                    .any(|e| e.event.stage == Some("session_save"))
            );
            if mode != TamperMode::Truncate {
                assert_stage(action, "parse", "ok");
                assert_stage(action, "receive", "error");
            }
            if mode == TamperMode::UntrustedIdentity {
                for stage in [
                    "session_load",
                    "authentication",
                    "plaintext",
                    "session_candidates",
                ] {
                    assert_stage(action, stage, "ok");
                }
                assert!(action.error.as_ref().is_some_and(|error| !error.is_empty()));
            }
            assert_eq!(
                before_session,
                s.bob
                    .load_session(&s.alice_addr)
                    .now_or_never()
                    .expect("sync")?
                    .unwrap()
                    .serialize()?
            );
            assert_eq!(
                before_identity,
                s.bob
                    .get_identity(&s.alice_addr)
                    .now_or_never()
                    .expect("sync")?
            );
            assert_eq!(before_bytes, s.messages[2].bytes);
            assert_eq!(s.messages[2].view.status, "waiting");
            assert_eq!(s.delivered, before_delivered);
            s.deliver(3)?;
            assert_eq!(s.script.last().unwrap().outcome, Outcome::Ok);
            assert_stage(s.script.last().unwrap(), "session_save", "ok");
        }
        Ok(())
    }

    #[test]
    fn prekey_supports_truncation_but_rejects_normal_only_conditions() -> Result<()> {
        let mut s = Sim::new()?;
        s.send(Who::Alice, "first".into(), false)?;
        let count = s.script.len();
        for mode in [TamperMode::NoSession, TamperMode::UntrustedIdentity] {
            assert!(s.experiment(1, mode).is_err());
            assert_eq!(s.script.len(), count);
        }
        s.experiment(1, TamperMode::Truncate)?;
        assert_eq!(s.script.last().unwrap().outcome, Outcome::TooShort);
        assert_stage(s.script.last().unwrap(), "parse", "error");
        s.deliver(1)?;
        assert_eq!(s.script.last().unwrap().outcome, Outcome::Ok);
        Ok(())
    }

    #[test]
    fn receiver_condition_experiments_work_in_reverse_direction() -> Result<()> {
        let mut s = Sim::new()?;
        s.send(Who::Alice, "first".into(), true)?;
        s.send(Who::Bob, "reply".into(), false)?;
        s.experiment(2, TamperMode::NoSession)?;
        assert_eq!(s.script.last().unwrap().outcome, Outcome::NoSession);
        s.experiment(2, TamperMode::UntrustedIdentity)?;
        assert_eq!(s.script.last().unwrap().outcome, Outcome::UntrustedIdentity);
        s.deliver(2)?;
        assert_eq!(s.script.last().unwrap().outcome, Outcome::Ok);
        Ok(())
    }
    #[test]
    fn rejected_operations_do_not_append_actions() -> Result<()> {
        let mut s = Sim::new()?;
        assert!(s.deliver(999).is_err());
        s.send(Who::Alice, "one".into(), false)?;
        assert!(s.replay(1).is_err());
        assert_eq!(s.script.len(), 1);
        s.set_expect(0, Expect::AnyError)?;
        assert_eq!(s.state().last.unwrap().expect, Expect::AnyError);
        Ok(())
    }
}
