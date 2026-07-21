use crate::browser_runtime::BrowserError;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const BROKER_HELLO_SCHEMA: u16 = 1;
const BROKER_HELLO_MAX_BYTES: usize = 16 * 1024;
const BROKER_DOMAIN: &[u8] = b"cys.browser.private.v1/supervise\0";

type HmacSha256 = Hmac<Sha256>;

/// First broker-to-supervisor command. The key used to authenticate this
/// envelope is transferred as raw bytes over the inherited pipe before the
/// JSON line; it is never accepted through argv, env, or a caller path.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerHello {
    pub schema_version: u16,
    pub broker_pid: u32,
    pub broker_session_id: String,
    pub command_sequence: u64,
    pub consumed_receipt_id: String,
    pub normalized_request_hash: String,
    pub subject_hash: String,
    pub expected_runtime_id: String,
    pub attestation_id: String,
    pub policy_epoch: u64,
    pub policy_hash: String,
    pub mac: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedBrokerHello {
    pub broker_pid: u32,
    pub broker_session_id: String,
    pub command_sequence: u64,
    pub consumed_receipt_id: String,
    pub normalized_request_hash: String,
    pub subject_hash: String,
    pub expected_runtime_id: String,
    pub attestation_id: String,
    pub policy_epoch: u64,
    pub policy_hash: String,
}

impl BrokerHello {
    #[allow(clippy::too_many_arguments)]
    pub fn signed(
        broker_pid: u32,
        command_sequence: u64,
        consumed_receipt_id: impl Into<String>,
        normalized_request_hash: impl Into<String>,
        subject_hash: impl Into<String>,
        expected_runtime_id: impl Into<String>,
        attestation_id: impl Into<String>,
        policy_epoch: u64,
        policy_hash: impl Into<String>,
        session_key: [u8; 32],
    ) -> Result<Self, BrowserError> {
        let mut hello = Self {
            schema_version: BROKER_HELLO_SCHEMA,
            broker_pid,
            broker_session_id: format!("sha256:{:x}", Sha256::digest(session_key)),
            command_sequence,
            consumed_receipt_id: consumed_receipt_id.into(),
            normalized_request_hash: normalized_request_hash.into(),
            subject_hash: subject_hash.into(),
            expected_runtime_id: expected_runtime_id.into(),
            attestation_id: attestation_id.into(),
            policy_epoch,
            policy_hash: policy_hash.into(),
            mac: String::new(),
        };
        hello.validate_shape(broker_pid)?;
        hello.mac = hex(&hello.calculate_mac(&session_key)?);
        Ok(hello)
    }

    pub fn parse(input: &str) -> Result<Self, BrowserError> {
        if input.len() > BROKER_HELLO_MAX_BYTES {
            return Err(BrowserError::authority_rejected(
                "private broker hello exceeds 16 KiB",
            ));
        }
        serde_json::from_str(input).map_err(|error| {
            BrowserError::authority_rejected(format!("invalid private broker hello: {error}"))
        })
    }

    pub fn to_line(&self) -> Result<Vec<u8>, BrowserError> {
        let mut line = serde_json::to_vec(self).map_err(|error| {
            BrowserError::authority_rejected(format!("private broker hello encode: {error}"))
        })?;
        if line.len() >= BROKER_HELLO_MAX_BYTES {
            return Err(BrowserError::authority_rejected(
                "private broker hello exceeds 16 KiB",
            ));
        }
        line.push(b'\n');
        Ok(line)
    }

    pub fn verify(
        &self,
        expected_parent_pid: u32,
        session_key: [u8; 32],
    ) -> Result<VerifiedBrokerHello, BrowserError> {
        self.validate_shape(expected_parent_pid)?;
        let expected_session = format!("sha256:{:x}", Sha256::digest(session_key));
        if self.broker_session_id != expected_session {
            return Err(BrowserError::authority_rejected(
                "broker session identity mismatch",
            ));
        }
        let supplied = decode_hex_32(&self.mac)?;
        let mut verifier = HmacSha256::new_from_slice(&session_key)
            .map_err(|_| BrowserError::authority_rejected("invalid broker session key"))?;
        verifier.update(&self.mac_input());
        verifier
            .verify_slice(&supplied)
            .map_err(|_| BrowserError::authority_rejected("broker command MAC mismatch"))?;
        Ok(VerifiedBrokerHello {
            broker_pid: self.broker_pid,
            broker_session_id: self.broker_session_id.clone(),
            command_sequence: self.command_sequence,
            consumed_receipt_id: self.consumed_receipt_id.clone(),
            normalized_request_hash: self.normalized_request_hash.clone(),
            subject_hash: self.subject_hash.clone(),
            expected_runtime_id: self.expected_runtime_id.clone(),
            attestation_id: self.attestation_id.clone(),
            policy_epoch: self.policy_epoch,
            policy_hash: self.policy_hash.clone(),
        })
    }

    fn validate_shape(&self, expected_parent_pid: u32) -> Result<(), BrowserError> {
        if self.schema_version != BROKER_HELLO_SCHEMA
            || self.broker_pid == 0
            || self.broker_pid != expected_parent_pid
            || self.command_sequence != 1
            || self.consumed_receipt_id.is_empty()
            || self.consumed_receipt_id.len() > 256
            || self.policy_epoch == 0
        {
            return Err(BrowserError::authority_rejected(
                "private broker identity, sequence, receipt, or policy is invalid",
            ));
        }
        for (label, value) in [
            ("broker session", &self.broker_session_id),
            ("request", &self.normalized_request_hash),
            ("subject", &self.subject_hash),
            ("runtime", &self.expected_runtime_id),
            ("attestation", &self.attestation_id),
            ("policy", &self.policy_hash),
        ] {
            if !is_sha256_id(value) {
                return Err(BrowserError::authority_rejected(format!(
                    "private broker {label} hash is malformed"
                )));
            }
        }
        Ok(())
    }

    fn calculate_mac(&self, session_key: &[u8; 32]) -> Result<[u8; 32], BrowserError> {
        let mut signer = HmacSha256::new_from_slice(session_key)
            .map_err(|_| BrowserError::authority_rejected("invalid broker session key"))?;
        signer.update(&self.mac_input());
        Ok(signer.finalize().into_bytes().into())
    }

    fn mac_input(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(1024);
        bytes.extend_from_slice(BROKER_DOMAIN);
        put(&mut bytes, &self.schema_version.to_be_bytes());
        put(&mut bytes, &self.broker_pid.to_be_bytes());
        put(&mut bytes, self.broker_session_id.as_bytes());
        put(&mut bytes, &self.command_sequence.to_be_bytes());
        put(&mut bytes, self.consumed_receipt_id.as_bytes());
        put(&mut bytes, self.normalized_request_hash.as_bytes());
        put(&mut bytes, self.subject_hash.as_bytes());
        put(&mut bytes, self.expected_runtime_id.as_bytes());
        put(&mut bytes, self.attestation_id.as_bytes());
        put(&mut bytes, &self.policy_epoch.to_be_bytes());
        put(&mut bytes, self.policy_hash.as_bytes());
        bytes
    }
}

fn put(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u32).to_be_bytes());
    output.extend_from_slice(value);
}

fn is_sha256_id(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn decode_hex_32(input: &str) -> Result<[u8; 32], BrowserError> {
    if input.len() != 64 || !input.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BrowserError::authority_rejected(
            "private broker MAC is malformed",
        ));
    }
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&input[index * 2..index * 2 + 2], 16)
            .map_err(|_| BrowserError::authority_rejected("private broker MAC is malformed"))?;
    }
    Ok(output)
}

fn hex(input: &[u8]) -> String {
    input.iter().map(|byte| format!("{byte:02x}")).collect()
}
