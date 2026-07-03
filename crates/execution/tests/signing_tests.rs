//! Tests for the Solana `VersionedTransaction` deserialize/sign/serialize
//! mechanics used by `DefaultSigner` (real.rs).
//!
//! The recorded `tests/fixtures/jupiter_swap_response.json`'s
//! `swapTransaction` field is a 5-byte placeholder ("AQABAgM=" decodes to
//! `[1, 0, 1, 2, 3]`), not a real serialized `VersionedTransaction` -- it
//! cannot be deserialized or re-signed. So these tests build real,
//! single-signer `VersionedTransaction`s in-process (the same shape
//! Jupiter's `/swap` endpoint returns: one required signer, the fee payer)
//! and drive them through the exact code path `DefaultSigner::sign_and_send`
//! exercises: base64-decode, wincode-deserialize, sign the message bytes,
//! wincode-serialize, base64-encode, then submit via a mocked RPC.
//!
//! Both a legacy and a V0 (versioned, address-lookup-table) transaction are
//! covered. Production always requests a V0 transaction from Jupiter
//! (`jupiter.rs` sets `asLegacyTransaction: false`), so the V0 case with a
//! non-empty `address_table_lookups` section is the one that matches real
//! traffic; the legacy case guards the simpler shape.

use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::Verifier;
use serde_json::Value;
use solana_address::Address;
use solana_hash::Hash;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair as SolKeypair;
use solana_message::compiled_instruction::CompiledInstruction;
use solana_message::legacy::Message as LegacyMessage;
use solana_message::v0::{Message as MessageV0, MessageAddressTableLookup};
use solana_message::{MessageHeader, VersionedMessage};
use solana_signature::Signature;
use solana_signer::Signer as SolSigner;
use solana_transaction::versioned::VersionedTransaction;
use tradebot_data::SolanaRpcClient;
use tradebot_execution::{DefaultSigner, TxSigner};
use tradebot_wallet::{generate_bot_keypair, BotKeypair};
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

fn load_fixture(name: &str) -> Value {
    let text = std::fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("failed to parse fixture {name}: {e}"))
}

/// Builds an unsigned, single-signer legacy `VersionedTransaction` the shape
/// Jupiter's `/swap` endpoint would return for `bot_keypair`, base64-encoded
/// exactly as `swapTransaction` would be.
fn unsigned_swap_tx_b64(bot_keypair: &BotKeypair) -> String {
    let fee_payer_bytes: [u8; 32] = bs58::decode(&bot_keypair.address)
        .into_vec()
        .unwrap()
        .try_into()
        .unwrap();
    let fee_payer = Address::new_from_array(fee_payer_bytes);
    let program = SolKeypair::new().pubkey();

    let ix =
        Instruction::new_with_bytes(program, &[9, 9, 9], vec![AccountMeta::new(fee_payer, true)]);
    let message = LegacyMessage::new(&[ix], Some(&fee_payer));
    assert_eq!(message.header.num_required_signatures, 1);

    let unsigned_tx = VersionedTransaction {
        signatures: vec![Signature::default()],
        message: VersionedMessage::Legacy(message),
    };
    BASE64.encode(wincode::serialize(&unsigned_tx).unwrap())
}

/// The `bot_keypair`'s address as a Solana `Address` (fee payer / sole
/// required signer).
fn fee_payer_address(bot_keypair: &BotKeypair) -> Address {
    let bytes: [u8; 32] = bs58::decode(&bot_keypair.address)
        .into_vec()
        .unwrap()
        .try_into()
        .unwrap();
    Address::new_from_array(bytes)
}

/// Builds an unsigned, single-signer **V0** (versioned) `VersionedTransaction`
/// with a non-empty `address_table_lookups` section, the shape Jupiter's
/// `/swap` endpoint actually returns (`asLegacyTransaction: false`). Returns
/// the base64 encoding (as `swapTransaction` would carry it) alongside the
/// original unsigned transaction so tests can assert the ALT data survives
/// the deserialize/sign/reserialize round trip.
fn unsigned_v0_swap_tx(bot_keypair: &BotKeypair) -> (String, VersionedTransaction) {
    let fee_payer = fee_payer_address(bot_keypair);
    let program = SolKeypair::new().pubkey();

    // Static account keys: [fee payer (writable signer), program (readonly
    // unsigned)]. Additional accounts are loaded via the address table
    // lookup below, exactly like a Jupiter route through an ALT.
    let header = MessageHeader {
        num_required_signatures: 1,
        num_readonly_signed_accounts: 0,
        num_readonly_unsigned_accounts: 1,
    };
    let account_keys = vec![fee_payer, program];

    // program_id_index 1 -> program (a static key, as required). The
    // remaining account indexes (2, 3) resolve into addresses loaded from
    // the lookup table.
    let instruction = CompiledInstruction {
        program_id_index: 1,
        accounts: vec![0, 2, 3],
        data: vec![7, 7, 7],
    };

    let lookups = vec![MessageAddressTableLookup {
        account_key: SolKeypair::new().pubkey(),
        writable_indexes: vec![10, 20],
        readonly_indexes: vec![30],
    }];

    let message = MessageV0 {
        header,
        account_keys,
        recent_blockhash: Hash::new_from_array([9u8; 32]),
        instructions: vec![instruction],
        address_table_lookups: lookups,
    };

    let unsigned_tx = VersionedTransaction {
        signatures: vec![Signature::default()],
        message: VersionedMessage::V0(message),
    };
    let b64 = BASE64.encode(wincode::serialize(&unsigned_tx).unwrap());
    (b64, unsigned_tx)
}

/// Extracts the `address_table_lookups` from a `VersionedMessage`, panicking
/// if it is not a V0 message.
fn alt_lookups(message: &VersionedMessage) -> &[MessageAddressTableLookup] {
    match message {
        VersionedMessage::V0(m) => &m.address_table_lookups,
        other => panic!("expected a V0 message, got {other:?}"),
    }
}

#[test]
fn deserialize_sign_reserialize_roundtrip() {
    let bot_keypair = generate_bot_keypair();
    let signing_key = bot_keypair.signing_key().unwrap();
    let unsigned_b64 = unsigned_swap_tx_b64(&bot_keypair);

    // --- The code path DefaultSigner::sign_and_send exercises, minus the
    //     network call. ---
    let raw = BASE64.decode(&unsigned_b64).unwrap();
    let mut tx: VersionedTransaction = wincode::deserialize(&raw).unwrap();
    assert_eq!(tx.message.header().num_required_signatures, 1);

    let message_bytes = tx.message.serialize();
    let sig_bytes = bot_keypair.sign(&message_bytes).unwrap();
    tx.signatures[0] = Signature::from(sig_bytes);

    let signed_bytes = wincode::serialize(&tx).unwrap();
    let signed_b64 = BASE64.encode(&signed_bytes);

    // Round-trip through base64 + wincode once more, as sendTransaction
    // would on the validator side, and verify the signature is valid for
    // the message bytes and the bot's own public key.
    let final_raw = BASE64.decode(&signed_b64).unwrap();
    let final_tx: VersionedTransaction = wincode::deserialize(&final_raw).unwrap();
    assert_eq!(final_tx.signatures.len(), 1);

    let sig: [u8; 64] = final_tx.signatures[0].into();
    let verify_sig = ed25519_dalek::Signature::from_bytes(&sig);
    assert!(signing_key
        .verifying_key()
        .verify(&final_tx.message.serialize(), &verify_sig)
        .is_ok());
}

#[tokio::test]
async fn default_signer_sign_and_send_submits_signed_transaction() {
    let sig_payload = load_fixture("rpc_send_tx.json");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("sendTransaction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&sig_payload))
        .mount(&server)
        .await;

    let bot_keypair = generate_bot_keypair();
    let unsigned_b64 = unsigned_swap_tx_b64(&bot_keypair);
    let rpc = SolanaRpcClient::new(server.uri());

    let signer = DefaultSigner;
    let signature = signer
        .sign_and_send(&unsigned_b64, &bot_keypair, &rpc)
        .await
        .expect("sign_and_send should succeed");

    assert!(signature.starts_with("5J7q9X3z2Y8w4"));
}

#[test]
fn v0_deserialize_sign_reserialize_roundtrip() {
    let bot_keypair = generate_bot_keypair();
    let signing_key = bot_keypair.signing_key().unwrap();
    let (unsigned_b64, original) = unsigned_v0_swap_tx(&bot_keypair);
    let original_lookups = alt_lookups(&original.message).to_vec();
    assert!(
        !original_lookups.is_empty(),
        "test fixture must exercise a non-empty ALT section"
    );

    // --- The code path DefaultSigner::sign_and_send exercises, minus the
    //     network call. ---
    let raw = BASE64.decode(&unsigned_b64).unwrap();
    let mut tx: VersionedTransaction = wincode::deserialize(&raw).unwrap();
    assert_eq!(tx.message.header().num_required_signatures, 1);
    // The address-lookup-table section must survive the deserialize step.
    assert_eq!(alt_lookups(&tx.message), original_lookups.as_slice());

    let message_bytes = tx.message.serialize();
    let sig_bytes = bot_keypair.sign(&message_bytes).unwrap();
    tx.signatures[0] = Signature::from(sig_bytes);

    let signed_bytes = wincode::serialize(&tx).unwrap();
    let signed_b64 = BASE64.encode(&signed_bytes);

    // Round-trip through base64 + wincode once more, as sendTransaction
    // would on the validator side.
    let final_raw = BASE64.decode(&signed_b64).unwrap();
    let final_tx: VersionedTransaction = wincode::deserialize(&final_raw).unwrap();
    assert_eq!(final_tx.signatures.len(), 1);

    // The ALT data survived the full deserialize -> sign -> reserialize ->
    // deserialize round trip intact.
    assert_eq!(alt_lookups(&final_tx.message), original_lookups.as_slice());

    // The signature verifies against the message bytes and the bot pubkey.
    let sig: [u8; 64] = final_tx.signatures[0].into();
    let verify_sig = ed25519_dalek::Signature::from_bytes(&sig);
    assert!(signing_key
        .verifying_key()
        .verify(&final_tx.message.serialize(), &verify_sig)
        .is_ok());
}

#[tokio::test]
async fn default_signer_sign_and_send_submits_signed_v0_transaction() {
    let sig_payload = load_fixture("rpc_send_tx.json");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("sendTransaction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&sig_payload))
        .mount(&server)
        .await;

    let bot_keypair = generate_bot_keypair();
    let (unsigned_b64, _original) = unsigned_v0_swap_tx(&bot_keypair);
    let rpc = SolanaRpcClient::new(server.uri());

    let signer = DefaultSigner;
    let signature = signer
        .sign_and_send(&unsigned_b64, &bot_keypair, &rpc)
        .await
        .expect("sign_and_send should succeed for a V0 transaction");

    assert!(signature.starts_with("5J7q9X3z2Y8w4"));
}
