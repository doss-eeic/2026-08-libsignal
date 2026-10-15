#![cfg(feature = "viz-trace")]

mod support;

use std::time::SystemTime;

use futures_util::FutureExt;
use libsignal_protocol::{viz_trace::*, *};
use rand::{TryRngCore as _, rngs::OsRng};
use support::*;

type TestResult = Result<(), SignalProtocolError>;

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
        assert_eq!(
            take().iter().filter(|e| e.kind == "mac_rejected").count(),
            1
        );
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
        assert!(!take().iter().any(|e| e.kind == "mac_rejected"));
        Ok(())
    }
    .now_or_never()
    .expect("sync")
}
