use std::{collections::BTreeMap, future::Future, sync::Arc};

use anyhow::{Result, anyhow, ensure};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
};
use futures::FutureExt;
use libsignal_protocol::{
    DeviceId, IdentityKeyPair, InMemSignalProtocolStore, ProtocolAddress, SenderKeyStore,
    SignalProtocolError, create_sender_key_distribution_message, group_decrypt, group_encrypt,
    process_sender_key_distribution_message, viz_trace,
};
use rand::{TryRngCore as _, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{ApiError, ApiResult, NameRequest, bad, internal};

pub(crate) const DISTRIBUTION_ID: Uuid = Uuid::from_u128(1);

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Member {
    Alice,
    Bob,
    Carol,
    Dave,
}
impl Member {
    const ALL: [Self; 4] = [Self::Alice, Self::Bob, Self::Carol, Self::Dave];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Alice => "alice",
            Self::Bob => "bob",
            Self::Carol => "carol",
            Self::Dave => "dave",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Alice => "Alice",
            Self::Bob => "Bob",
            Self::Carol => "Carol",
            Self::Dave => "Dave",
        }
    }
    fn address(self) -> ProtocolAddress {
        ProtocolAddress::new(self.name().into(), DeviceId::new(1).expect("valid device"))
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Ok,
    Duplicate,
    BadSignature,
    NoSenderKey,
    Error,
    AnyError,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActionType {
    Join,
    Send,
    Deliver,
    Tamper,
    Replay,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Event {
    kind: &'static str,
    who: Member,
    sender: Member,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<Vec<Member>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Action {
    pub i: usize,
    #[serde(rename = "type")]
    pub action_type: ActionType,
    pub who: Member,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distributed_to: Option<Vec<Member>>,
    pub outcome: Outcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub expect: Outcome,
    pub events: Vec<Event>,
}
impl Action {
    fn new(action_type: ActionType, who: Member) -> Self {
        Self {
            i: 0,
            action_type,
            who,
            message_id: None,
            text: None,
            distributed_to: None,
            outcome: Outcome::Ok,
            error: None,
            expect: Outcome::Ok,
            events: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct Delivery {
    to: Member,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    delivered_order: Option<u32>,
}
#[derive(Clone, Debug, Serialize)]
struct Message {
    id: u32,
    from: Member,
    text: String,
    iteration: u32,
    ciphertext_hex: String,
    ciphertext_len: usize,
    deliveries: Vec<Delivery>,
}
struct StoredMessage {
    view: Message,
    bytes: Vec<u8>,
}

#[derive(Serialize)]
struct KeyBox {
    id: String,
    label: String,
    sender: Member,
    fp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<u32>,
}
#[derive(Serialize)]
pub(crate) struct GroupState {
    members: Vec<Member>,
    can_join: Vec<Member>,
    keys: BTreeMap<Member, Vec<KeyBox>>,
    messages: Vec<Message>,
    script: Vec<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last: Option<Action>,
}

pub(crate) struct Sim {
    stores: Vec<InMemSignalProtocolStore>,
    members: Vec<Member>,
    distributed: [[bool; 4]; 4],
    messages: Vec<StoredMessage>,
    pub script: Vec<Action>,
    delivered: u32,
}
impl Sim {
    pub(crate) fn new() -> Result<Self> {
        let mut rng = OsRng.unwrap_err();
        let stores = Member::ALL
            .iter()
            .map(|_| InMemSignalProtocolStore::new(IdentityKeyPair::generate(&mut rng), 1))
            .collect::<Result<Vec<_>, SignalProtocolError>>()?;
        Ok(Self {
            stores,
            members: vec![Member::Alice, Member::Bob],
            distributed: [[false; 4]; 4],
            messages: Vec::new(),
            script: Vec::new(),
            delivered: 0,
        })
    }

    fn state(&mut self) -> Result<GroupState> {
        let mut keys = BTreeMap::new();
        for &who in &self.members {
            let mut boxes = Vec::new();
            for &sender in &self.members {
                if let Some(record) = self.stores[who as usize]
                    .load_sender_key(&sender.address(), DISTRIBUTION_ID)
                    .now_or_never()
                    .expect("sync")?
                {
                    for key in viz_trace::sender_key_boxes(&record) {
                        let skipped = key.id.strip_prefix("skipped:");
                        let (id, label) = if let Some(iteration) = skipped {
                            (
                                format!("skipped:{}:{iteration}", sender.name()),
                                format!("取り置きの鍵 #{iteration}（{}）", sender.label()),
                            )
                        } else {
                            (
                                format!("sk:{}", sender.name()),
                                if who == sender {
                                    "自分の Sender Key".into()
                                } else {
                                    format!("{} の Sender Key", sender.label())
                                },
                            )
                        };
                        boxes.push(KeyBox {
                            id,
                            label,
                            sender,
                            fp: key.fp,
                            index: key.index,
                        });
                    }
                }
            }
            keys.insert(who, boxes);
        }
        Ok(GroupState {
            members: self.members.clone(),
            can_join: [Member::Carol, Member::Dave]
                .into_iter()
                .filter(|m| !self.members.contains(m))
                .collect(),
            keys,
            messages: self.messages.iter().map(|m| m.view.clone()).collect(),
            script: self.script.clone(),
            last: self.script.last().cloned(),
        })
    }

    pub(crate) fn join(&mut self, who: Member) -> Result<()> {
        ensure!(!self.members.contains(&who), "すでに参加しています");
        self.members.push(who);
        for message in &mut self.messages {
            message.view.deliveries.push(Delivery {
                to: who,
                status: "not_addressed",
                delivered_order: None,
            });
        }
        self.push(Action::new(ActionType::Join, who));
        Ok(())
    }

    pub(crate) fn send(&mut self, from: Member, text: String, deliver: bool) -> Result<()> {
        ensure!(self.members.contains(&from), "送信者はまだ参加していません");
        ensure!(
            (1..=200).contains(&text.chars().count()),
            "本文は1〜200文字で入力してください"
        );
        let recipients: Vec<_> = self
            .members
            .iter()
            .copied()
            .filter(|&m| m != from)
            .collect();
        let distributed: Vec<_> = recipients
            .iter()
            .copied()
            .filter(|&m| !self.distributed[from as usize][m as usize])
            .collect();
        let mut rng = OsRng.unwrap_err();
        let mut events = Vec::new();
        let distribution = recorded(
            from,
            from,
            &mut events,
            create_sender_key_distribution_message(
                &from.address(),
                DISTRIBUTION_ID,
                &mut self.stores[from as usize],
                &mut rng,
            ),
        )?;
        for &to in &distributed {
            recorded(
                to,
                from,
                &mut events,
                process_sender_key_distribution_message(
                    &from.address(),
                    &distribution,
                    &mut self.stores[to as usize],
                ),
            )?;
            self.distributed[from as usize][to as usize] = true;
        }
        if !distributed.is_empty() {
            events.push(Event {
                kind: "sender_key_distributed",
                who: from,
                sender: from,
                to: Some(distributed.clone()),
                from: None,
                to_index: None,
                index: Some(distribution.iteration()?),
                fp: Some(viz_trace::fp(distribution.chain_key()?)),
                ok: None,
                stage: None,
                status: None,
                source: None,
                detail: None,
            });
        }
        let message = recorded(
            from,
            from,
            &mut events,
            group_encrypt(
                &mut self.stores[from as usize],
                &from.address(),
                DISTRIBUTION_ID,
                text.as_bytes(),
                &mut rng,
            ),
        )?;
        let bytes = message.serialized().to_vec();
        let id = self.messages.len() as u32 + 1;
        self.messages.push(StoredMessage {
            view: Message {
                id,
                from,
                text: text.clone(),
                iteration: message.iteration(),
                ciphertext_hex: bytes.iter().take(12).map(|b| format!("{b:02x}")).collect(),
                ciphertext_len: bytes.len(),
                deliveries: recipients
                    .iter()
                    .map(|&to| Delivery {
                        to,
                        status: "waiting",
                        delivered_order: None,
                    })
                    .collect(),
            },
            bytes,
        });
        let mut action = Action::new(ActionType::Send, from);
        action.message_id = Some(id);
        action.text = Some(text);
        action.distributed_to = Some(distributed);
        action.events = events;
        self.push(action);
        if deliver {
            for to in recipients {
                self.receive(id, to, ActionType::Deliver)?;
            }
        }
        Ok(())
    }

    pub(crate) fn receive(&mut self, id: u32, to: Member, kind: ActionType) -> Result<()> {
        ensure!(self.members.contains(&to), "受信者はまだ参加していません");
        let index = id
            .checked_sub(1)
            .ok_or_else(|| anyhow!("メッセージが見つかりません"))? as usize;
        let message = self
            .messages
            .get(index)
            .ok_or_else(|| anyhow!("メッセージが見つかりません"))?;
        let delivery = message
            .view
            .deliveries
            .iter()
            .find(|d| d.to == to)
            .ok_or_else(|| anyhow!("送信者には配達できません"))?;
        ensure!(
            match kind {
                ActionType::Deliver => matches!(delivery.status, "waiting" | "not_addressed"),
                ActionType::Tamper => delivery.status == "waiting",
                ActionType::Replay => delivery.status == "delivered",
                _ => false,
            },
            "このメッセージにはその操作を行えません"
        );
        let sender = message.view.from;
        let mut bytes = message.bytes.clone();
        if kind == ActionType::Tamper {
            tamper_bytes(&mut bytes);
        }
        let mut action = Action::new(kind, to);
        action.message_id = Some(id);
        let result = recorded(
            to,
            sender,
            &mut action.events,
            group_decrypt(&bytes, &mut self.stores[to as usize], &sender.address()),
        );
        match result {
            Ok(plaintext) => {
                ensure!(
                    plaintext == self.messages[index].view.text.as_bytes(),
                    "復号した本文が一致しません"
                );
                if kind == ActionType::Deliver {
                    self.delivered += 1;
                    let delivery = self.messages[index]
                        .view
                        .deliveries
                        .iter_mut()
                        .find(|d| d.to == to)
                        .expect("validated recipient");
                    delivery.status = "delivered";
                    delivery.delivered_order = Some(self.delivered);
                }
            }
            Err(error) => {
                action.outcome = match error {
                    SignalProtocolError::DuplicatedMessage(..) => Outcome::Duplicate,
                    SignalProtocolError::SignatureValidationFailed => Outcome::BadSignature,
                    SignalProtocolError::NoSenderKeyState { .. } => Outcome::NoSenderKey,
                    _ => Outcome::Error,
                };
                action.error = Some(format!("{error:?}"));
            }
        }
        action.expect = action.outcome;
        self.push(action);
        Ok(())
    }
    fn push(&mut self, mut action: Action) {
        action.i = self.script.len();
        self.script.push(action);
    }
}

fn recorded<T>(
    who: Member,
    sender: Member,
    events: &mut Vec<Event>,
    call: impl Future<Output = Result<T, SignalProtocolError>>,
) -> Result<T, SignalProtocolError> {
    viz_trace::start();
    let result = call
        .now_or_never()
        .expect("in-memory stores are synchronous");
    events.extend(viz_trace::take().into_iter().map(|e| Event {
        kind: e.kind,
        who,
        sender,
        to: None,
        from: e.from,
        to_index: e.to,
        index: e.index,
        fp: e.fp,
        ok: e.ok,
        stage: e.stage,
        status: e.status,
        source: e.source,
        detail: e.detail,
    }));
    result
}

// Included verbatim in generated tests, which must run without viz-trace.
pub(crate) const TAMPER_HELPER: &str = include_str!("group_tamper.rs");
include!("group_tamper.rs");

type Shared = Arc<Mutex<Sim>>;
type Body<T> = Result<Json<T>, axum::extract::rejection::JsonRejection>;
fn parse<T>(body: Body<T>) -> Result<T, ApiError> {
    body.map(|Json(r)| r)
        .map_err(|_| bad("リクエストのJSONが正しくありません"))
}
fn conflict(e: anyhow::Error) -> ApiError {
    ApiError(StatusCode::CONFLICT, e.to_string())
}

pub(crate) fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/state", get(state))
        .route("/reset", post(reset))
        .route("/join", post(join))
        .route("/send", post(send))
        .route("/deliver", post(deliver))
        .route("/tamper", post(tamper))
        .route("/replay", post(replay))
        .route("/script/{i}/expect", put(set_expect))
        .route("/test/generate", post(generate))
        .with_state(Arc::new(Mutex::new(Sim::new().expect("group stores"))))
}
async fn state(State(s): State<Shared>) -> ApiResult<GroupState> {
    Ok(Json(s.lock().await.state().map_err(internal)?))
}
async fn reset(State(s): State<Shared>) -> ApiResult<GroupState> {
    let mut s = s.lock().await;
    *s = Sim::new().map_err(internal)?;
    Ok(Json(s.state().map_err(internal)?))
}
#[derive(Deserialize)]
struct JoinRequest {
    who: Member,
}
async fn join(State(s): State<Shared>, body: Body<JoinRequest>) -> ApiResult<GroupState> {
    let r = parse(body)?;
    let mut s = s.lock().await;
    s.join(r.who).map_err(conflict)?;
    Ok(Json(s.state().map_err(internal)?))
}
#[derive(Deserialize)]
struct SendRequest {
    from: Member,
    text: String,
    deliver: bool,
}
async fn send(State(s): State<Shared>, body: Body<SendRequest>) -> ApiResult<GroupState> {
    let r = parse(body)?;
    let mut s = s.lock().await;
    if !s.members.contains(&r.from) {
        return Err(bad("送信者はまだ参加していません"));
    }
    if !(1..=200).contains(&r.text.chars().count()) {
        return Err(bad("本文は1〜200文字で入力してください"));
    }
    s.send(r.from, r.text, r.deliver).map_err(internal)?;
    Ok(Json(s.state().map_err(internal)?))
}
#[derive(Deserialize)]
struct DeliveryRequest {
    message_id: u32,
    to: Member,
}
async fn receive(
    s: Shared,
    body: Body<DeliveryRequest>,
    kind: ActionType,
) -> ApiResult<GroupState> {
    let r = parse(body)?;
    let mut s = s.lock().await;
    s.receive(r.message_id, r.to, kind).map_err(conflict)?;
    Ok(Json(s.state().map_err(internal)?))
}
async fn deliver(State(s): State<Shared>, body: Body<DeliveryRequest>) -> ApiResult<GroupState> {
    receive(s, body, ActionType::Deliver).await
}
async fn tamper(State(s): State<Shared>, body: Body<DeliveryRequest>) -> ApiResult<GroupState> {
    receive(s, body, ActionType::Tamper).await
}
async fn replay(State(s): State<Shared>, body: Body<DeliveryRequest>) -> ApiResult<GroupState> {
    receive(s, body, ActionType::Replay).await
}
#[derive(Deserialize)]
struct ExpectRequest {
    expect: Outcome,
}
async fn set_expect(
    State(s): State<Shared>,
    path: Result<Path<usize>, axum::extract::rejection::PathRejection>,
    body: Body<ExpectRequest>,
) -> ApiResult<GroupState> {
    let Path(i) = path.map_err(|_| bad("操作番号が正しくありません"))?;
    let r = parse(body)?;
    let mut s = s.lock().await;
    s.script
        .get_mut(i)
        .ok_or_else(|| {
            ApiError(
                StatusCode::NOT_FOUND,
                "指定された操作が見つかりません".into(),
            )
        })?
        .expect = r.expect;
    Ok(Json(s.state().map_err(internal)?))
}
async fn generate(State(s): State<Shared>, body: Body<NameRequest>) -> ApiResult<Value> {
    let r = parse(body)?;
    if !crate::valid_name(&r.name) {
        return Err(bad("テスト名が正しくありません"));
    }
    let s = s.lock().await;
    if s.script.is_empty() {
        return Err(bad("操作を1つ以上行ってから生成してください"));
    }
    let code = crate::group_testgen::generate(&r.name, &s.script).map_err(internal)?;
    drop(s);
    crate::write_generated(&r.name, &code).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn last(s: &Sim) -> &Action {
        s.script.last().expect("action")
    }

    fn stage<'a>(action: &'a Action, stage: &str, status: &str) -> &'a Event {
        action
            .events
            .iter()
            .find(|e| {
                e.kind == "decrypt_stage" && e.stage == Some(stage) && e.status == Some(status)
            })
            .unwrap_or_else(|| panic!("missing stage {stage}: {status}"))
    }

    fn assert_not_reached(action: &Action, stages: &[&str]) {
        assert!(action.events.iter().all(|e| {
            e.kind != "decrypt_stage" || !e.stage.is_some_and(|stage| stages.contains(&stage))
        }));
    }

    #[test]
    fn alice_bob_exchange_with_delivery() -> Result<()> {
        let mut s = Sim::new()?;
        assert_eq!(s.state()?.members, [Member::Alice, Member::Bob]);
        s.send(Member::Alice, "hi".into(), true)?;
        s.send(Member::Bob, "yo".into(), true)?;
        assert_eq!(s.script.len(), 4);
        assert!(s.script.iter().all(|a| a.outcome == Outcome::Ok));
        assert_eq!(s.script[0].action_type, ActionType::Send);
        assert_eq!(s.script[1].action_type, ActionType::Deliver);
        assert_eq!(s.script[0].distributed_to, Some(vec![Member::Bob]));
        for action in s
            .script
            .iter()
            .filter(|a| a.action_type == ActionType::Deliver)
        {
            for name in [
                "parse",
                "sender_key_load",
                "sender_key_state",
                "signature",
                "key_prepare",
                "plaintext",
                "sender_key_save",
                "receive",
            ] {
                let event = stage(action, name, "ok");
                assert_eq!(event.who, action.who);
                assert_ne!(event.sender, action.who);
                assert!(event.source.is_some());
                assert!(event.detail.is_none());
            }
            let order: Vec<_> = action
                .events
                .iter()
                .filter(|e| e.kind == "decrypt_stage" && e.status == Some("ok"))
                .filter_map(|e| e.stage)
                .collect();
            assert_eq!(
                order,
                [
                    "parse",
                    "sender_key_load",
                    "sender_key_state",
                    "signature",
                    "key_prepare",
                    "plaintext",
                    "sender_key_save",
                    "receive"
                ]
            );
        }
        let events = &s.script[0].events;
        assert!(
            events
                .iter()
                .any(|e| e.kind == "sender_key_created" && e.who == Member::Alice)
        );
        assert!(
            events
                .iter()
                .any(|e| e.kind == "sender_key_received" && e.who == Member::Bob)
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == "sender_key_distributed")
                .count(),
            1
        );
        let state = serde_json::to_value(s.state()?)?;
        assert_eq!(state["keys"]["alice"][0]["label"], "自分の Sender Key");
        assert_eq!(state["keys"]["bob"][0]["label"], "Alice の Sender Key");
        let chain = state["script"][0]["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["kind"] == "sender_chain_step")
            .unwrap();
        assert_eq!(chain["to_index"], 1);
        assert!(chain.get("to").is_none());
        let receive = state["script"][1]["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["stage"] == "sender_key_save" && e["status"] == "ok")
            .unwrap();
        assert_eq!(receive["kind"], "decrypt_stage");
        assert_eq!(receive["who"], "bob");
        assert_eq!(receive["sender"], "alice");
        assert!(receive["source"].as_str().is_some());
        s.send(Member::Alice, "again".into(), true)?;
        assert_eq!(s.script[4].distributed_to, Some(vec![]));
        Ok(())
    }

    #[test]
    fn late_joiner_gets_no_key_then_duplicate_for_old_message() -> Result<()> {
        let mut s = Sim::new()?;
        s.send(Member::Alice, "old".into(), true)?;
        s.join(Member::Carol)?;
        assert!(last(&s).events.is_empty());
        assert_eq!(s.messages[0].view.deliveries[1].status, "not_addressed");
        s.receive(1, Member::Carol, ActionType::Deliver)?;
        assert_eq!(last(&s).outcome, Outcome::NoSenderKey);
        stage(last(&s), "parse", "ok");
        assert!(stage(last(&s), "sender_key_load", "error").detail.is_some());
        stage(last(&s), "receive", "error");
        assert_not_reached(
            last(&s),
            &[
                "sender_key_state",
                "signature",
                "key_prepare",
                "plaintext",
                "sender_key_save",
            ],
        );
        s.send(Member::Alice, "new".into(), true)?;
        assert_eq!(s.script[4].distributed_to, Some(vec![Member::Carol]));
        assert!(
            s.messages[1]
                .view
                .deliveries
                .iter()
                .all(|d| d.status == "delivered")
        );
        s.receive(1, Member::Carol, ActionType::Deliver)?;
        assert_eq!(last(&s).outcome, Outcome::Duplicate);
        stage(last(&s), "signature", "ok");
        stage(last(&s), "key_prepare", "error");
        assert_not_reached(last(&s), &["plaintext", "sender_key_save"]);
        assert_eq!(s.messages[0].view.deliveries[1].status, "not_addressed");
        Ok(())
    }

    #[test]
    fn out_of_order_stores_then_uses_skipped_keys() -> Result<()> {
        let mut s = Sim::new()?;
        for text in ["zero", "one", "two"] {
            s.send(Member::Alice, text.into(), false)?;
        }
        s.receive(3, Member::Bob, ActionType::Deliver)?;
        assert_eq!(
            last(&s)
                .events
                .iter()
                .filter(|e| e.kind == "skipped_key_stored")
                .count(),
            2
        );
        assert!(
            s.state()?.keys[&Member::Bob]
                .iter()
                .any(|k| k.id == "skipped:alice:0" && k.label == "取り置きの鍵 #0（Alice）")
        );
        s.receive(1, Member::Bob, ActionType::Deliver)?;
        assert!(
            last(&s)
                .events
                .iter()
                .any(|e| e.kind == "skipped_key_used" && e.index == Some(0))
        );
        assert_eq!(s.messages[2].view.deliveries[0].delivered_order, Some(1));
        assert_eq!(s.messages[0].view.deliveries[0].delivered_order, Some(2));
        Ok(())
    }

    #[test]
    fn tamper_preserves_original_and_keys_then_replay_is_duplicate() -> Result<()> {
        let mut s = Sim::new()?;
        s.send(Member::Alice, "hi".into(), false)?;
        let before = serde_json::to_value(s.state()?.keys)?;
        let original = s.messages[0].bytes.clone();
        let mut tampered = original.clone();
        tamper_bytes(&mut tampered);
        let parsed = libsignal_protocol::SenderKeyMessage::try_from(tampered.as_slice())?;
        let clean = libsignal_protocol::SenderKeyMessage::try_from(original.as_slice())?;
        assert_eq!(
            &tampered[tampered.len() - 64..],
            &original[original.len() - 64..]
        );
        assert_ne!(parsed.ciphertext(), clean.ciphertext());
        assert_eq!(parsed.iteration(), clean.iteration());
        assert_eq!(parsed.chain_id(), clean.chain_id());
        s.receive(1, Member::Bob, ActionType::Tamper)?;
        assert_eq!(last(&s).outcome, Outcome::BadSignature);
        stage(last(&s), "parse", "ok");
        stage(last(&s), "sender_key_load", "ok");
        stage(last(&s), "sender_key_state", "ok");
        assert!(stage(last(&s), "signature", "error").detail.is_some());
        stage(last(&s), "receive", "error");
        assert_not_reached(last(&s), &["key_prepare", "plaintext", "sender_key_save"]);
        assert!(
            last(&s)
                .events
                .iter()
                .any(|e| e.kind == "signature_checked" && e.ok == Some(false))
        );
        assert_eq!(before, serde_json::to_value(s.state()?.keys)?);
        assert_eq!(s.messages[0].bytes, original);
        assert_eq!(s.messages[0].view.deliveries[0].status, "waiting");
        s.receive(1, Member::Bob, ActionType::Deliver)?;
        assert_eq!(last(&s).outcome, Outcome::Ok);
        stage(last(&s), "signature", "ok");
        stage(last(&s), "sender_key_save", "ok");
        stage(last(&s), "receive", "ok");
        s.receive(1, Member::Bob, ActionType::Replay)?;
        assert_eq!(last(&s).outcome, Outcome::Duplicate);
        stage(last(&s), "signature", "ok");
        stage(last(&s), "key_prepare", "error");
        stage(last(&s), "receive", "error");
        assert_not_reached(last(&s), &["plaintext", "sender_key_save"]);
        Ok(())
    }

    #[test]
    fn membership_order_validation_and_no_action_on_rejection() -> Result<()> {
        let mut s = Sim::new()?;
        assert!(s.send(Member::Carol, "hi".into(), true).is_err());
        assert!(s.send(Member::Alice, "".into(), true).is_err());
        assert!(s.send(Member::Alice, "あ".repeat(201), true).is_err());
        assert!(s.join(Member::Bob).is_err());
        assert!(s.receive(0, Member::Bob, ActionType::Deliver).is_err());
        assert!(s.script.is_empty());
        s.join(Member::Dave)?;
        s.join(Member::Carol)?;
        assert!(s.state()?.can_join.is_empty());
        s.send(Member::Alice, "あ".repeat(200), true)?;
        assert_eq!(
            s.script.iter().skip(3).map(|a| a.who).collect::<Vec<_>>(),
            [Member::Bob, Member::Dave, Member::Carol]
        );
        assert!(s.receive(1, Member::Alice, ActionType::Deliver).is_err());
        assert!(s.receive(1, Member::Bob, ActionType::Tamper).is_err());
        assert_eq!(s.script.len(), 6);
        Ok(())
    }
}
