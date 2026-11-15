use std::io::Error as IoError;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{
    self, GeneralPurpose, GeneralPurposeConfig, URL_SAFE_NO_PAD,
};
use hmac::{KeyInit, Mac};
use serde_json::{Map, Value};
use subtle::ConstantTimeEq;
use zphira_common::fs::{self, AtomicWriteOptions};

pub const EXPIRATION_MILLIS: i64 = 24 * 60 * 60 * 1000;
pub const SECRET_FILE: &str = "jwt-secret";
const NEW_SECRET_BYTES: usize = 32;
const B64_SECRET: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_encode_padding(true)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JwtAlgorithm {
    Hs256,
    Hs384,
    Hs512,
}

impl JwtAlgorithm {
    pub fn name(self) -> &'static str {
        match self {
            Self::Hs256 => "HS256",
            Self::Hs384 => "HS384",
            Self::Hs512 => "HS512",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "HS256" => Some(Self::Hs256),
            "HS384" => Some(Self::Hs384),
            "HS512" => Some(Self::Hs512),
            _ => None,
        }
    }

    fn min_key_bytes(self) -> usize {
        match self {
            Self::Hs256 => 32,
            Self::Hs384 => 48,
            Self::Hs512 => 64,
        }
    }

    fn for_key(len: usize) -> Option<Self> {
        match len {
            64.. => Some(Self::Hs512),
            48.. => Some(Self::Hs384),
            32.. => Some(Self::Hs256),
            _ => None,
        }
    }

    fn sign(self, key: &[u8], input: &[u8]) -> Vec<u8> {
        match self {
            Self::Hs256 => mac::<hmac::Hmac<sha2::Sha256>>(key, input),
            Self::Hs384 => mac::<hmac::Hmac<sha2::Sha384>>(key, input),
            Self::Hs512 => mac::<hmac::Hmac<sha2::Sha512>>(key, input),
        }
    }
}

fn mac<M: Mac + KeyInit>(key: &[u8], input: &[u8]) -> Vec<u8> {
    let mut mac = <M as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(input);
    mac.finalize().into_bytes().to_vec()
}

#[derive(Debug)]
pub enum JwtError {
    Io { path: PathBuf, source: IoError },
    NotBase64 { path: PathBuf },
    TooShort { path: PathBuf, bytes: usize },
}

impl std::fmt::Display for JwtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, .. } => write!(f, "Failed to load JWT secret ({})", path.display()),
            Self::NotBase64 { path } => write!(
                f,
                "The JWT secret in {} is not valid Base64; fix or delete the file (a new one is generated, which signs everyone out)",
                path.display()
            ),
            Self::TooShort { path, bytes } => write!(
                f,
                "The JWT secret in {} is {} bits; HMAC-SHA signing needs at least 256 bits (32 bytes); fix or delete the file (a new one is generated, which signs everyone out)",
                path.display(),
                bytes * 8
            ),
        }
    }
}

impl std::error::Error for JwtError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct JwtService {
    key: Vec<u8>,
    algorithm: JwtAlgorithm,
}

impl std::fmt::Debug for JwtService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwtService")
            .field("algorithm", &self.algorithm)
            .finish_non_exhaustive()
    }
}

impl JwtService {
    /// Reads `<data_dir>/jwt-secret`, creating it with a random key when missing. Blocking.
    pub fn load(data_dir: impl AsRef<Path>) -> Result<Self, JwtError> {
        let path = data_dir.as_ref().join(SECRET_FILE);
        let io = |source| JwtError::Io {
            path: path.clone(),
            source,
        };
        if let Err(e) = fs::remove_stale_temp_files(&path) {
            tracing::warn!(error = %e, "Could not remove stale temporary files of {}", path.display());
        }
        match fs::read_to_string_if_exists(&path).map_err(io)? {
            Some(text) => {
                let key = B64_SECRET
                    .decode(text.trim())
                    .map_err(|_| JwtError::NotBase64 { path: path.clone() })?;
                if let Err(e) = fs::set_mode(&path, 0o600) {
                    tracing::warn!(error = %e, "Could not restrict the permissions of {}", path.display());
                }
                Self::from_secret(key).map_err(|bytes| JwtError::TooShort { path, bytes })
            }
            None => {
                let mut key = vec![0u8; NEW_SECRET_BYTES];
                getrandom::fill(&mut key).map_err(|e| io(std::io::Error::other(e.to_string())))?;
                let text = general_purpose::STANDARD.encode(&key);
                fs::write_atomic_with(
                    &path,
                    &AtomicWriteOptions::new().mode(0o600),
                    |w: &mut std::io::BufWriter<std::fs::File>| {
                        std::io::Write::write_all(w, text.as_bytes())
                    },
                )
                .map_err(io)?;
                Self::from_secret(key).map_err(|bytes| JwtError::TooShort { path, bytes })
            }
        }
    }

    /// `Err(len)` when the key is shorter than 32 bytes.
    pub fn from_secret(key: Vec<u8>) -> Result<Self, usize> {
        let algorithm = JwtAlgorithm::for_key(key.len()).ok_or(key.len())?;
        Ok(Self { key, algorithm })
    }

    pub fn algorithm(&self) -> JwtAlgorithm {
        self.algorithm
    }

    pub fn issue_token(&self, user_id: i32, is_admin: bool) -> String {
        self.issue_token_at(user_id, is_admin, zphira_common::time::now_millis())
    }

    pub fn issue_token_at(&self, user_id: i32, is_admin: bool, now_millis: i64) -> String {
        let header = format!("{{\"alg\":\"{}\"}}", self.algorithm.name());
        let iat = now_millis.div_euclid(1000);
        let exp = now_millis
            .saturating_add(EXPIRATION_MILLIS)
            .div_euclid(1000);
        let payload =
            format!("{{\"sub\":\"{user_id}\",\"isAdmin\":{is_admin},\"iat\":{iat},\"exp\":{exp}}}");
        let signing_input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header),
            URL_SAFE_NO_PAD.encode(payload)
        );
        let signature = self.algorithm.sign(&self.key, signing_input.as_bytes());
        format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(signature))
    }

    /// Verifies the token; `None` for any invalid, expired or tampered one.
    pub fn parse_user_id(&self, token: &str) -> Option<i32> {
        self.parse_user_id_at(token, zphira_common::time::now_millis())
    }

    pub fn parse_user_id_at(&self, token: &str, now_millis: i64) -> Option<i32> {
        let claims = self.verify(token, now_millis)?;
        claims.get("sub")?.as_str()?.parse().ok()
    }

    fn verify(&self, token: &str, now_millis: i64) -> Option<Map<String, Value>> {
        let mut parts = token.split('.');
        let (header_b64, payload_b64, signature_b64) =
            (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        let header = decode_object(header_b64)?;
        let algorithm = JwtAlgorithm::from_name(header.get("alg")?.as_str()?)?;
        if self.key.len() < algorithm.min_key_bytes() {
            return None;
        }
        let signature = URL_SAFE_NO_PAD.decode(signature_b64).ok()?;
        let expected = algorithm.sign(&self.key, format!("{header_b64}.{payload_b64}").as_bytes());
        if !bool::from(expected.ct_eq(&signature)) {
            return None;
        }
        let claims = decode_object(payload_b64)?;
        let millis = |name: &str| match claims.get(name) {
            None | Some(Value::Null) => Some(None),
            Some(value) => value
                .as_f64()
                .map(|seconds| Some((seconds * 1000.0) as i64)),
        };
        if millis("exp")?.is_some_and(|exp| now_millis > exp)
            || millis("nbf")?.is_some_and(|nbf| now_millis < nbf)
        {
            return None;
        }
        Some(claims)
    }
}

fn decode_object(part: &str) -> Option<Map<String, Value>> {
    match serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).ok()?).ok()? {
        Value::Object(object) => Some(object),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> JwtService {
        JwtService::from_secret((0u8..32).collect()).unwrap()
    }

    #[test]
    fn round_trip() {
        let jwt = service();
        let token = jwt.issue_token_at(42, true, 1_000_000);
        assert_eq!(jwt.parse_user_id_at(&token, 1_000_000), Some(42));
        assert_eq!(
            jwt.parse_user_id_at(&token, 1_000_000 + EXPIRATION_MILLIS),
            Some(42)
        );
        assert_eq!(
            jwt.parse_user_id_at(&token, 1_000_000 + EXPIRATION_MILLIS + 1),
            None
        );
    }

    #[test]
    fn key_lengths() {
        assert!(JwtService::from_secret(vec![0; 31]).is_err());
        assert_eq!(
            JwtService::from_secret(vec![0; 32]).unwrap().algorithm(),
            JwtAlgorithm::Hs256
        );
        assert_eq!(
            JwtService::from_secret(vec![0; 48]).unwrap().algorithm(),
            JwtAlgorithm::Hs384
        );
        assert_eq!(
            JwtService::from_secret(vec![0; 64]).unwrap().algorithm(),
            JwtAlgorithm::Hs512
        );
    }
}
