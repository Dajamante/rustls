#![allow(clippy::duplicate_mod)]

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::crypto;
use crate::crypto::cipher::{
    make_tls13_aad, AeadKey, Iv, MessageDecrypter, MessageEncrypter, Nonce, Tls13AeadAlgorithm,
    UnsupportedOperationError,
};
use crate::crypto::tls13::{Hkdf, HkdfExpander, OkmBlock, OutputLengthError};
use crate::enums::{CipherSuite, ContentType, ProtocolVersion};
use crate::error::Error;
use crate::msgs::codec::Codec;
use crate::msgs::message::{BorrowedOpaqueMessage, BorrowedPlainMessage, OpaqueMessage};
use crate::suites::{CipherSuiteCommon, ConnectionTrafficSecrets, SupportedCipherSuite};
use crate::tls13::Tls13CipherSuite;

use super::ring_like::hkdf::KeyType;
use super::ring_like::{aead, hkdf, hmac};

/// The TLS1.3 ciphersuite TLS_CHACHA20_POLY1305_SHA256
pub static TLS13_CHACHA20_POLY1305_SHA256: SupportedCipherSuite =
    SupportedCipherSuite::Tls13(TLS13_CHACHA20_POLY1305_SHA256_INTERNAL);

pub(crate) static TLS13_CHACHA20_POLY1305_SHA256_INTERNAL: &Tls13CipherSuite = &Tls13CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS13_CHACHA20_POLY1305_SHA256,
        hash_provider: &super::hash::SHA256,
        confidentiality_limit: u64::MAX,
        integrity_limit: 1 << 36,
    },
    hkdf_provider: &RingHkdf(hkdf::HKDF_SHA256, hmac::HMAC_SHA256),
    aead_alg: &Chacha20Poly1305Aead(AeadAlgorithm(&aead::CHACHA20_POLY1305)),
    quic: Some(&super::quic::KeyBuilder(
        &aead::CHACHA20_POLY1305,
        &aead::quic::CHACHA20,
    )),
};

/// The TLS1.3 ciphersuite TLS_AES_256_GCM_SHA384
pub static TLS13_AES_256_GCM_SHA384: SupportedCipherSuite =
    SupportedCipherSuite::Tls13(&Tls13CipherSuite {
        common: CipherSuiteCommon {
            suite: CipherSuite::TLS13_AES_256_GCM_SHA384,
            hash_provider: &super::hash::SHA384,
            confidentiality_limit: 1 << 23,
            integrity_limit: 1 << 52,
        },
        hkdf_provider: &RingHkdf(hkdf::HKDF_SHA384, hmac::HMAC_SHA384),
        aead_alg: &Aes256GcmAead(AeadAlgorithm(&aead::AES_256_GCM)),
        quic: Some(&super::quic::KeyBuilder(
            &aead::AES_256_GCM,
            &aead::quic::AES_256,
        )),
    });

/// The TLS1.3 ciphersuite TLS_AES_128_GCM_SHA256
pub static TLS13_AES_128_GCM_SHA256: SupportedCipherSuite =
    SupportedCipherSuite::Tls13(TLS13_AES_128_GCM_SHA256_INTERNAL);

pub(crate) static TLS13_AES_128_GCM_SHA256_INTERNAL: &Tls13CipherSuite = &Tls13CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS13_AES_128_GCM_SHA256,
        hash_provider: &super::hash::SHA256,
        confidentiality_limit: 1 << 23,
        integrity_limit: 1 << 52,
    },
    hkdf_provider: &RingHkdf(hkdf::HKDF_SHA256, hmac::HMAC_SHA256),
    aead_alg: &Aes128GcmAead(AeadAlgorithm(&aead::AES_128_GCM)),
    quic: Some(&super::quic::KeyBuilder(
        &aead::AES_128_GCM,
        &aead::quic::AES_128,
    )),
};

struct Chacha20Poly1305Aead(AeadAlgorithm);

impl Tls13AeadAlgorithm for Chacha20Poly1305Aead {
    fn encrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageEncrypter> {
        self.0.encrypter(key, iv)
    }

    fn decrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageDecrypter> {
        self.0.decrypter(key, iv)
    }

    fn key_len(&self) -> usize {
        self.0.key_len()
    }

    fn extract_keys(
        &self,
        key: AeadKey,
        iv: Iv,
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Ok(ConnectionTrafficSecrets::Chacha20Poly1305 { key, iv })
    }
}

struct Aes256GcmAead(AeadAlgorithm);

impl Tls13AeadAlgorithm for Aes256GcmAead {
    fn encrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageEncrypter> {
        self.0.encrypter(key, iv)
    }

    fn decrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageDecrypter> {
        self.0.decrypter(key, iv)
    }

    fn key_len(&self) -> usize {
        self.0.key_len()
    }

    fn extract_keys(
        &self,
        key: AeadKey,
        iv: Iv,
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Ok(ConnectionTrafficSecrets::Aes256Gcm { key, iv })
    }
}

struct Aes128GcmAead(AeadAlgorithm);

impl Tls13AeadAlgorithm for Aes128GcmAead {
    fn encrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageEncrypter> {
        self.0.encrypter(key, iv)
    }

    fn decrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageDecrypter> {
        self.0.decrypter(key, iv)
    }

    fn key_len(&self) -> usize {
        self.0.key_len()
    }

    fn extract_keys(
        &self,
        key: AeadKey,
        iv: Iv,
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Ok(ConnectionTrafficSecrets::Aes128Gcm { key, iv })
    }
}

// common encrypter/decrypter/key_len items for above Tls13AeadAlgorithm impls
struct AeadAlgorithm(&'static aead::Algorithm);

impl AeadAlgorithm {
    fn encrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageEncrypter> {
        // safety: the caller arranges that `key` is `key_len()` in bytes, so this unwrap is safe.
        Box::new(Tls13MessageEncrypter {
            enc_key: aead::LessSafeKey::new(aead::UnboundKey::new(self.0, key.as_ref()).unwrap()),
            iv,
        })
    }

    fn decrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageDecrypter> {
        // safety: the caller arranges that `key` is `key_len()` in bytes, so this unwrap is safe.
        Box::new(Tls13MessageDecrypter {
            dec_key: aead::LessSafeKey::new(aead::UnboundKey::new(self.0, key.as_ref()).unwrap()),
            iv,
        })
    }

    fn key_len(&self) -> usize {
        self.0.key_len()
    }
}

struct Tls13MessageEncrypter {
    enc_key: aead::LessSafeKey,
    iv: Iv,
}

struct Tls13MessageDecrypter {
    dec_key: aead::LessSafeKey,
    iv: Iv,
}

impl MessageEncrypter for Tls13MessageEncrypter {
    fn encrypt(&self, msg: BorrowedPlainMessage, seq: u64) -> Result<OpaqueMessage, Error> {
        let total_len = self.encrypted_payload_len(msg.payload.len());
        let mut payload = Vec::with_capacity(total_len);
        payload.extend_from_slice(msg.payload);
        msg.typ.encode(&mut payload);

        let nonce = aead::Nonce::assume_unique_for_key(Nonce::new(&self.iv, seq).0);
        let aad = aead::Aad::from(make_tls13_aad(total_len));
        self.enc_key
            .seal_in_place_append_tag(nonce, aad, &mut payload)
            .map_err(|_| Error::EncryptError)?;

        Ok(OpaqueMessage::new(
            ContentType::ApplicationData,
            ProtocolVersion::TLSv1_2,
            payload,
        ))
    }

    fn encrypted_payload_len(&self, payload_len: usize) -> usize {
        payload_len + 1 + self.enc_key.algorithm().tag_len()
    }
}

impl MessageDecrypter for Tls13MessageDecrypter {
    fn decrypt<'p>(
        &self,
        mut msg: BorrowedOpaqueMessage<'p>,
        seq: u64,
    ) -> Result<BorrowedPlainMessage<'p>, Error> {
        let payload = &mut msg.payload;
        if payload.len() < self.dec_key.algorithm().tag_len() {
            return Err(Error::DecryptError);
        }

        let nonce = aead::Nonce::assume_unique_for_key(Nonce::new(&self.iv, seq).0);
        let aad = aead::Aad::from(make_tls13_aad(payload.len()));
        let plain_len = self
            .dec_key
            .open_in_place(nonce, aad, payload)
            .map_err(|_| Error::DecryptError)?
            .len();

        payload.truncate(plain_len);
        msg.into_tls13_unpadded_message()
    }
}

struct RingHkdf(hkdf::Algorithm, hmac::Algorithm);

impl Hkdf for RingHkdf {
    fn extract_from_zero_ikm(&self, salt: Option<&[u8]>) -> Box<dyn HkdfExpander> {
        let zeroes = [0u8; OkmBlock::MAX_LEN];
        let salt = match salt {
            Some(salt) => salt,
            None => &zeroes[..self.0.len()],
        };
        Box::new(RingHkdfExpander {
            alg: self.0,
            prk: hkdf::Salt::new(self.0, salt).extract(&zeroes[..self.0.len()]),
        })
    }

    fn extract_from_secret(&self, salt: Option<&[u8]>, secret: &[u8]) -> Box<dyn HkdfExpander> {
        let zeroes = [0u8; OkmBlock::MAX_LEN];
        let salt = match salt {
            Some(salt) => salt,
            None => &zeroes[..self.0.len()],
        };
        Box::new(RingHkdfExpander {
            alg: self.0,
            prk: hkdf::Salt::new(self.0, salt).extract(secret),
        })
    }

    fn expander_for_okm(&self, okm: &OkmBlock) -> Box<dyn HkdfExpander> {
        Box::new(RingHkdfExpander {
            alg: self.0,
            prk: hkdf::Prk::new_less_safe(self.0, okm.as_ref()),
        })
    }

    fn hmac_sign(&self, key: &OkmBlock, message: &[u8]) -> crypto::hmac::Tag {
        crypto::hmac::Tag::new(hmac::sign(&hmac::Key::new(self.1, key.as_ref()), message).as_ref())
    }
}

struct RingHkdfExpander {
    alg: hkdf::Algorithm,
    prk: hkdf::Prk,
}

impl HkdfExpander for RingHkdfExpander {
    fn expand_slice(&self, info: &[&[u8]], output: &mut [u8]) -> Result<(), OutputLengthError> {
        self.prk
            .expand(info, Len(output.len()))
            .and_then(|okm| okm.fill(output))
            .map_err(|_| OutputLengthError)
    }

    fn expand_block(&self, info: &[&[u8]]) -> OkmBlock {
        let mut buf = [0u8; OkmBlock::MAX_LEN];
        let output = &mut buf[..self.hash_len()];
        self.prk
            .expand(info, Len(output.len()))
            .and_then(|okm| okm.fill(output))
            .unwrap();
        OkmBlock::new(output)
    }

    fn hash_len(&self) -> usize {
        self.alg.len()
    }
}

struct Len(usize);

impl KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}
