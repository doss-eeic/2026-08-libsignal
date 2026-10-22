mod support;

use futures_util::FutureExt;
use libsignal_protocol::*;
use rand::TryRngCore as _;
use rand::rngs::OsRng;
use support::*;
use uuid::Uuid;

type TestResult = Result<(), SignalProtocolError>;

use assert_matches::assert_matches;
// use libsignal_protocol::{CiphertextMessageType, SignalProtocolError};
use libsignal_protocol::{SignalProtocolError};


#[test]
fn group_decrypt_signature_validation_failed() -> Result<(), SignalProtocolError> {
    async {
        let mut csprng = OsRng.unwrap_err();

        let sender_address =
            ProtocolAddress::new("+14159999111".to_owned(), DeviceId::new(1).unwrap());
        let distribution_id = Uuid::from_u128(0xd1d1d1d1_7000_11eb_b32a_33b8a8a487a6);

        let mut alice_store = test_in_memory_protocol_store()?;
        let mut bob_store = test_in_memory_protocol_store()?;

        let sent_distribution_message = create_sender_key_distribution_message(
            &sender_address,
            distribution_id,
            &mut alice_store,
            &mut csprng,
        )
        .await?;

        let recv_distribution_message =
            SenderKeyDistributionMessage::try_from(sent_distribution_message.serialized())?;

        let alice_ciphertext = group_encrypt(
            &mut alice_store,
            &sender_address,
            distribution_id,
            "ninja camp".as_bytes(),
            &mut csprng,
        )
        .await?;

        process_sender_key_distribution_message(
            &sender_address,
            &recv_distribution_message,
            &mut bob_store,
        )
        .await?;

        let mut tampered_bytes = alice_ciphertext.serialized().to_vec();
        let data_len = tampered_bytes.len();
        tampered_bytes[data_len - 1] ^= 0xFF;
        
        let tampered_msg = SenderKeyMessage::try_from(tampered_bytes.as_slice())?;

        let result = group_decrypt(
            tampered_msg.serialized(),
            &mut bob_store,
            &sender_address,
        )
        .await;

        assert_matches!(result, Err(SignalProtocolError::SignatureValidationFailed { .. }));

        let bob_plaintext = group_decrypt(
            alice_ciphertext.serialized(),
            &mut bob_store,
            &sender_address,
        )
        .await?;

        assert_eq!(
            String::from_utf8(bob_plaintext).expect("valid utf8"),
            "ninja camp"
        );

        Ok(())
    }
    .now_or_never()
    .expect("sync")
}


