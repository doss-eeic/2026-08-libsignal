#![cfg(feature = "viz-trace")]

mod support;

use std::time::SystemTime;

use futures_util::FutureExt;
use libsignal_protocol::{viz_trace::*, *};
use rand::{TryRngCore as _, rngs::OsRng};
use support::*;

type TestResult = Result<(), SignalProtocolError>;

fn has_stage(events: &[VizEvent], stage: &str, status: &str) -> bool {
    events
        .iter()
        .any(|e| e.kind == "decrypt_stage" && e.stage == Some(stage) && e.status == Some(status))
}

#[test]
fn group_creation_distribution_and_normal_message() -> TestResult {
    async {
        let mut rng = OsRng.unwrap_err();
        let sender = ProtocolAddress::new("alice".into(), DeviceId::new(1).unwrap());
        let id = uuid::Uuid::from_u128(1);
        let mut a = test_in_memory_protocol_store()?;
        let mut b = test_in_memory_protocol_store()?;
        start();
        let distribution =
            create_sender_key_distribution_message(&sender, id, &mut a, &mut rng).await?;
        assert!(take().iter().any(|e| e.kind == "sender_key_created"
            && e.index == Some(0)
            && e.fp.as_ref().is_some_and(|fp| fp.len() == 8)));
        start();
        process_sender_key_distribution_message(&sender, &distribution, &mut b).await?;
        assert!(
            take()
                .iter()
                .any(|e| e.kind == "sender_key_received" && e.index == Some(0))
        );
        start();
        let message = group_encrypt(&mut a, &sender, id, b"hello", &mut rng).await?;
        let sent = take();
        assert!(
            sent.iter()
                .any(|e| e.kind == "sender_chain_step" && e.from == Some(0) && e.to == Some(1))
        );
        start();
        assert_eq!(
            group_decrypt(message.serialized(), &mut b, &sender).await?,
            b"hello"
        );
        let received = take();
        assert!(
            received
                .iter()
                .any(|e| e.kind == "sender_chain_step" && e.from == Some(0) && e.to == Some(1))
        );
        assert!(
            received
                .iter()
                .any(|e| e.kind == "signature_checked" && e.ok == Some(true))
        );
        let completed: Vec<_> = received
            .iter()
            .filter(|e| e.kind == "decrypt_stage" && e.status != Some("started"))
            .map(|e| (e.stage.unwrap(), e.status.unwrap()))
            .collect();
        assert_eq!(
            completed,
            [
                ("parse", "ok"),
                ("sender_key_load", "ok"),
                ("sender_key_state", "ok"),
                ("signature", "ok"),
                ("key_prepare", "ok"),
                ("plaintext", "ok"),
                ("sender_key_save", "ok"),
                ("receive", "ok"),
            ]
        );
        assert_eq!(
            sent.iter()
                .find(|e| e.kind == "sender_message_key")
                .unwrap()
                .fp,
            received
                .iter()
                .find(|e| e.kind == "sender_message_key")
                .unwrap()
                .fp
        );
        Ok(())
    }
    .now_or_never()
    .expect("sync")
}

#[test]
fn group_skipped_keys_duplicate_and_signature_failure() -> TestResult {
    async {
        let mut rng = OsRng.unwrap_err();
        let sender = ProtocolAddress::new("alice".into(), DeviceId::new(1).unwrap());
        let id = uuid::Uuid::from_u128(1);
        let mut a = test_in_memory_protocol_store()?;
        let mut b = test_in_memory_protocol_store()?;
        let distribution =
            create_sender_key_distribution_message(&sender, id, &mut a, &mut rng).await?;
        process_sender_key_distribution_message(&sender, &distribution, &mut b).await?;
        let mut messages = Vec::new();
        for _ in 0..3 {
            messages.push(group_encrypt(&mut a, &sender, id, b"hello", &mut rng).await?);
        }
        let mut corrupt = messages[2].serialized().to_vec();
        *corrupt.last_mut().unwrap() ^= 1;
        let before = b.load_sender_key(&sender, id).await?.unwrap().serialize()?;
        start();
        assert!(matches!(
            group_decrypt(&corrupt, &mut b, &sender).await,
            Err(SignalProtocolError::SignatureValidationFailed)
        ));
        let rejected = take();
        assert!(
            rejected
                .iter()
                .any(|e| e.kind == "signature_checked" && e.ok == Some(false))
        );
        assert!(has_stage(&rejected, "signature", "error"));
        assert!(has_stage(&rejected, "receive", "error"));
        assert!(!rejected.iter().any(|e| {
            matches!(
                e.stage,
                Some("key_prepare" | "plaintext" | "sender_key_save")
            )
        }));
        assert_eq!(
            before,
            b.load_sender_key(&sender, id).await?.unwrap().serialize()?
        );
        start();
        group_decrypt(messages[2].serialized(), &mut b, &sender).await?;
        let events = take();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == "skipped_key_stored")
                .count(),
            2
        );
        // Keys put aside for iterations 0 and 1 are not "used"; only iteration 2 is.
        let used: Vec<_> = events
            .iter()
            .filter(|e| e.kind == "sender_message_key")
            .map(|e| e.index)
            .collect();
        assert_eq!(used, vec![Some(2)]);
        let record = b.load_sender_key(&sender, id).await?.unwrap();
        let before_snapshot = record.serialize()?;
        let boxes = sender_key_boxes(&record);
        assert_eq!(before_snapshot, record.serialize()?);
        assert_eq!(boxes[0].index, Some(3));
        assert_eq!(boxes.len(), 3);
        assert!(boxes.iter().all(|b| b.fp.len() == 8));
        start();
        group_decrypt(messages[0].serialized(), &mut b, &sender).await?;
        assert!(
            take()
                .iter()
                .any(|e| e.kind == "skipped_key_used" && e.index == Some(0))
        );
        start();
        assert!(matches!(
            group_decrypt(messages[0].serialized(), &mut b, &sender).await,
            Err(SignalProtocolError::DuplicatedMessage(..))
        ));
        let duplicate = take();
        assert!(
            duplicate
                .iter()
                .any(|e| e.kind == "duplicate_rejected" && e.index == Some(0))
        );
        assert!(has_stage(&duplicate, "signature", "ok"));
        assert!(has_stage(&duplicate, "key_prepare", "error"));
        assert!(has_stage(&duplicate, "receive", "error"));
        assert!(
            !duplicate
                .iter()
                .any(|e| matches!(e.stage, Some("plaintext" | "sender_key_save")))
        );
        Ok(())
    }
    .now_or_never()
    .expect("sync")
}

#[test]
fn group_late_joiner_cannot_read_older_message() -> TestResult {
    async {
        let mut rng = OsRng.unwrap_err();
        let sender = ProtocolAddress::new("alice".into(), DeviceId::new(1).unwrap());
        let id = uuid::Uuid::from_u128(1);
        let mut a = test_in_memory_protocol_store()?;
        let mut c = test_in_memory_protocol_store()?;
        create_sender_key_distribution_message(&sender, id, &mut a, &mut rng).await?;
        let old = group_encrypt(&mut a, &sender, id, b"old", &mut rng).await?;
        start();
        assert!(matches!(
            group_decrypt(old.serialized(), &mut c, &sender).await,
            Err(SignalProtocolError::NoSenderKeyState { .. })
        ));
        let missing = take();
        assert!(missing.iter().any(|e| e.kind == "no_sender_key"));
        assert!(has_stage(&missing, "parse", "ok"));
        assert!(has_stage(&missing, "sender_key_load", "error"));
        assert!(has_stage(&missing, "receive", "error"));
        assert!(!missing.iter().any(|e| matches!(
            e.stage,
            Some("signature" | "key_prepare" | "plaintext" | "sender_key_save")
        )));
        let current = create_sender_key_distribution_message(&sender, id, &mut a, &mut rng).await?;
        process_sender_key_distribution_message(&sender, &current, &mut c).await?;
        start();
        assert!(matches!(
            group_decrypt(old.serialized(), &mut c, &sender).await,
            Err(SignalProtocolError::DuplicatedMessage(..))
        ));
        assert!(take().iter().any(|e| e.kind == "duplicate_rejected"));
        Ok(())
    }
    .now_or_never()
    .expect("sync")
}

#[test]
fn malformed_group_message_is_rejected_before_loading_keys() -> TestResult {
    async {
        let sender = ProtocolAddress::new("alice".into(), DeviceId::new(1).unwrap());
        let mut receiver = test_in_memory_protocol_store()?;
        start();
        assert!(matches!(
            group_decrypt(&[], &mut receiver, &sender).await,
            Err(SignalProtocolError::CiphertextMessageTooShort(..))
        ));
        let rejected = take();
        assert!(has_stage(&rejected, "parse", "error"));
        assert!(has_stage(&rejected, "receive", "error"));
        assert!(!rejected.iter().any(|e| matches!(
            e.stage,
            Some("sender_key_load" | "signature" | "key_prepare" | "plaintext" | "sender_key_save")
        )));
        Ok(())
    }
    .now_or_never()
    .expect("sync")
}

async fn setup() -> Result<
    (
        InMemSignalProtocolStore,
        InMemSignalProtocolStore,
        ProtocolAddress,
        ProtocolAddress,
    ),
    SignalProtocolError,
> {
    let device = DeviceId::new(1).expect("valid device");
    let alice = ProtocolAddress::new("alice".into(), device);
    let bob = ProtocolAddress::new("bob".into(), device);
    let mut alice_store = TestStoreBuilder::new().store;
    let bob_builder = TestStoreBuilder::new()
        .with_pre_key(IdChoice::Next)
        .with_signed_pre_key(IdChoice::Next)
        .with_kyber_pre_key(IdChoice::Next);
    start();
    process_prekey_bundle(
        &bob,
        &alice,
        &mut alice_store.session_store,
        &mut alice_store.identity_store,
        &bob_builder.make_bundle_with_latest_keys(device),
        SystemTime::now(),
        &mut OsRng.unwrap_err(),
    )
    .await?;
    assert!(take().iter().any(|e| e.kind == "session_created"));
    Ok((alice_store, bob_builder.store, alice, bob))
}

#[test]
fn send_receive_and_reply() -> TestResult {
    async {
        let (mut a, mut b, alice, bob) = setup().await?;
        for index in 0..2 {
            start();
            let msg = encrypt(&mut a, &bob, &alice, "hello").await?;
            let events = take();
            assert!(events.iter().any(|e| e.kind == "message_key"
                && e.chain == Some("send")
                && e.index == Some(index)));
            assert!(events.iter().any(|e| e.kind == "chain_step"
                && e.chain == Some("send")
                && e.from == Some(index)
                && e.to == Some(index + 1)));
            start();
            assert_eq!(decrypt(&mut b, &alice, &bob, &msg).await?, b"hello");
            let events = take();
            assert!(
                events
                    .iter()
                    .any(|e| e.kind == "message_key" && e.chain == Some("recv"))
            );
            assert!(has_stage(&events, "authentication", "ok"));
            assert!(has_stage(&events, "plaintext", "ok"));
            assert!(has_stage(&events, "prekey_process", "ok"));
            assert!(has_stage(&events, "session_save", "ok"));
            assert!(has_stage(&events, "receive", "ok"));
            if index == 0 {
                assert!(events.iter().any(|e| e.kind == "session_created"));
            }
        }
        let reply = encrypt(&mut b, &alice, &bob, "reply").await?;
        start();
        decrypt(&mut a, &bob, &alice, &reply).await?;
        let events = take();
        assert!(events.iter().any(|e| e.kind == "dh_ratchet"));
        assert_eq!(events.iter().filter(|e| e.kind == "root_step").count(), 2);
        let boxes = session_boxes(&a.load_session(&bob).await?.expect("session"));
        assert_eq!(boxes[0].label, "ルート鍵");
        assert_eq!(boxes[1].label, "自分のDH鍵");
        assert_eq!(boxes[2].label, "送信の鎖");
        assert!(boxes.iter().all(|b| b.fp.len() == 8));
        Ok(())
    }
    .now_or_never()
    .expect("sync")
}

#[test]
fn skipped_keys_and_replay() -> TestResult {
    async {
        let (mut a, mut b, alice, bob) = setup().await?;
        let mut messages = Vec::new();
        for _ in 0..3 {
            messages.push(encrypt(&mut a, &bob, &alice, "hello").await?);
        }
        start();
        decrypt(&mut b, &alice, &bob, &messages[2]).await?;
        let events = take();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == "skipped_key_stored")
                .count(),
            2
        );
        let boxes = session_boxes(&b.load_session(&alice).await?.expect("session"));
        assert_eq!(
            boxes
                .iter()
                .filter(|b| b.id.starts_with("skipped:"))
                .count(),
            2
        );
        assert!(boxes.iter().any(|b| b.label == "取り置きの鍵 #0"));
        start();
        decrypt(&mut b, &alice, &bob, &messages[0]).await?;
        assert!(
            take()
                .iter()
                .any(|e| e.kind == "skipped_key_used" && e.index == Some(0))
        );
        start();
        let result = decrypt(&mut b, &alice, &bob, &messages[0]).await;
        assert!(matches!(
            result,
            Err(SignalProtocolError::DuplicatedMessage(..))
        ));
        assert!(
            take()
                .iter()
                .any(|e| e.kind == "duplicate_rejected" && e.index == Some(0))
        );
        Ok(())
    }
    .now_or_never()
    .expect("sync")
}

#[test]
fn tamper_records_one_mac_rejection_without_mutating_session() -> TestResult {
    async {
        let (mut a, mut b, alice, bob) = setup().await?;
        let first = encrypt(&mut a, &bob, &alice, "hello").await?;
        decrypt(&mut b, &alice, &bob, &first).await?;
        let reply = encrypt(&mut b, &alice, &bob, "reply").await?;
        decrypt(&mut a, &bob, &alice, &reply).await?;
        let msg = encrypt(&mut a, &bob, &alice, "payload").await?;
        let CiphertextMessage::SignalMessage(signal) = &msg else {
            panic!("whisper after reply")
        };
        let mut bytes = signal.serialized().to_vec();
        let body = signal.body();
        let offset = bytes
            .windows(body.len())
            .position(|w| w == body)
            .expect("body present");
        bytes[offset] ^= 1;
        let tampered = CiphertextMessage::SignalMessage(SignalMessage::try_from(bytes.as_slice())?);
        let before = b
            .load_session(&alice)
            .await?
            .expect("session")
            .serialize()?;
        start();
        assert!(matches!(
            decrypt(&mut b, &alice, &bob, &tampered).await,
            Err(SignalProtocolError::InvalidMessage(..))
        ));
        let events = take();
        assert_eq!(
            events.iter().filter(|e| e.kind == "mac_rejected").count(),
            1
        );
        assert!(has_stage(&events, "key_prepare", "ok"));
        assert!(has_stage(&events, "authentication", "error"));
        assert!(has_stage(&events, "receive", "error"));
        assert!(!events.iter().any(|e| {
            e.kind == "decrypt_stage"
                && matches!(
                    e.stage,
                    Some("plaintext" | "identity_save" | "session_save")
                )
        }));
        assert_eq!(
            before,
            b.load_session(&alice)
                .await?
                .expect("session")
                .serialize()?
        );
        Ok(())
    }
    .now_or_never()
    .expect("sync")
}

#[test]
fn recorder_is_opt_in_and_thread_local() {
    assert_eq!(fp(b"hello"), "2cf24dba");
    start();
    std::thread::spawn(|| {
        start();
        assert!(take().is_empty());
    })
    .join()
    .expect("thread");
    assert!(take().is_empty());
    assert!(take().is_empty());
    assert!(session_boxes(&SessionRecord::new_fresh()).is_empty());
}

#[test]
fn successful_archived_session_does_not_report_mac_rejection() -> TestResult {
    async {
        let (mut a, mut b, alice, bob) = setup().await?;
        let first = encrypt(&mut a, &bob, &alice, "first").await?;
        decrypt(&mut b, &alice, &bob, &first).await?;
        let second = encrypt(&mut a, &bob, &alice, "second").await?;
        let CiphertextMessage::PreKeySignalMessage(prekey) = second else {
            panic!("unacknowledged session");
        };
        let signal = SignalMessage::try_from(prekey.message().serialized())?;
        let mut record = b.load_session(&alice).await?.expect("session");
        let key = record
            .get_receiver_chain_key_bytes(signal.sender_ratchet_key())?
            .expect("chain");
        let mut bytes = record.serialize()?;
        let offset = bytes
            .windows(key.len())
            .position(|w| w == key.as_ref())
            .expect("chain in serialized record");
        bytes[offset] ^= 1;
        record.archive_current_state()?;
        // RecordStructure's current_session is field 1, previous_sessions is
        // repeated field 2. Concatenation preserves the corrupted current trial
        // and the valid archived copy, including identical SPQR state.
        bytes.extend(record.serialize()?);
        b.store_session(&alice, &SessionRecord::deserialize(&bytes)?)
            .await?;
        start();
        assert_eq!(
            decrypt(
                &mut b,
                &alice,
                &bob,
                &CiphertextMessage::SignalMessage(signal)
            )
            .await?,
            b"second"
        );
        let events = take();
        assert!(!events.iter().any(|e| e.kind == "mac_rejected"));
        let failed = events
            .iter()
            .find(|e| e.stage == Some("candidate") && e.status == Some("error"))
            .expect("current trial failed");
        let succeeded = events
            .iter()
            .find(|e| e.stage == Some("candidate") && e.status == Some("ok"))
            .expect("archived trial succeeded");
        assert_ne!(failed.candidate, succeeded.candidate);
        assert_eq!(failed.candidate_kind, Some("current"));
        assert_eq!(succeeded.candidate_kind, Some("previous"));
        assert!(has_stage(&events, "session_save", "ok"));
        assert!(has_stage(&events, "receive", "ok"));
        Ok(())
    }
    .now_or_never()
    .expect("sync")
}
