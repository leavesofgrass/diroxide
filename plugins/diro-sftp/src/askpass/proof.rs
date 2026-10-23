//! How the helper and the plugin prove themselves to each other (R24
//! section 5.3; answer 82).
//!
//! Per connection the plugin makes a random 32-byte token `T` and hands it
//! to ssh's environment for the helper. Neither side ever sends `T`; each
//! sends a keyed hash (HMAC-SHA-256) over fresh random numbers from both
//! sides, so a recorded exchange is worth nothing on the next connection:
//!
//! 1. the helper sends a hello with its nonce;
//! 2. the plugin answers with its own nonce and its proof over both;
//! 3. the helper checks that proof (an impostor endpoint learns nothing
//!    but a nonce), then sends the prompt with a proof over both nonces
//!    and the prompt's bytes;
//! 4. the plugin checks it in constant time, and only then reads the
//!    prompt and acts.
//!
//! What this stops: other users, a stray or leftover helper, a replay. It
//! does not stop a process of the same user, which could read ssh's
//! environment anyway (R24 section 5.3 says the same of any plugin).

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

/// The length of the token, the nonces and the proofs, in bytes.
pub const LEN: usize = 32;

/// The words that keep the plugin's proof from ever being the helper's.
const PLUGIN_LABEL: &[u8] = b"diro-askpass plugin v1";

/// The words that keep the helper's proof from ever being the plugin's.
const HELPER_LABEL: &[u8] = b"diro-askpass helper v1";

/// A connection's token, wiped when dropped and never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Token([u8; LEN]);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(hidden)")
    }
}

impl Drop for Token {
    fn drop(&mut self) {
        // Volatile-free wipe: the bytes are not read after this.
        self.0 = [0; LEN];
    }
}

/// Random bytes from the operating system; `None` only when it has none.
pub fn random() -> Option<[u8; LEN]> {
    let mut bytes = [0u8; LEN];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes)
}

impl Token {
    /// A new random token.
    pub fn new() -> Option<Self> {
        random().map(Self)
    }

    /// The token as the environment carries it: 64 hexadecimal digits.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Reads the environment's form; `None` for anything else.
    pub fn from_hex(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.len() != LEN * 2 || !text.is_ascii() {
            return None;
        }
        let mut bytes = [0u8; LEN];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(Self(bytes))
    }

    fn mac(&self, label: &[u8], parts: &[&[u8]]) -> Hmac<Sha256> {
        // A key of any length is accepted; 32 bytes always is.
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.0)
            .unwrap_or_else(|_| <Hmac<Sha256> as KeyInit>::new(&Default::default()));
        mac.update(label);
        for part in parts {
            // Each part's length first, so parts cannot run together.
            mac.update(&(part.len() as u64).to_be_bytes());
            mac.update(part);
        }
        mac
    }

    /// The plugin's proof over both nonces.
    pub fn plugin_proof(&self, helper_nonce: &[u8], plugin_nonce: &[u8]) -> [u8; LEN] {
        self.mac(PLUGIN_LABEL, &[helper_nonce, plugin_nonce])
            .finalize()
            .into_bytes()
            .into()
    }

    /// Checks the plugin's proof in constant time.
    pub fn check_plugin(&self, helper_nonce: &[u8], plugin_nonce: &[u8], proof: &[u8]) -> bool {
        self.mac(PLUGIN_LABEL, &[helper_nonce, plugin_nonce])
            .verify_slice(proof)
            .is_ok()
    }

    /// The helper's proof over both nonces, the prompt's kind and its
    /// bytes.
    pub fn helper_proof(
        &self,
        helper_nonce: &[u8],
        plugin_nonce: &[u8],
        kind: &[u8],
        prompt: &[u8],
    ) -> [u8; LEN] {
        self.mac(HELPER_LABEL, &[helper_nonce, plugin_nonce, kind, prompt])
            .finalize()
            .into_bytes()
            .into()
    }

    /// Checks the helper's proof in constant time.
    pub fn check_helper(
        &self,
        helper_nonce: &[u8],
        plugin_nonce: &[u8],
        kind: &[u8],
        prompt: &[u8],
        proof: &[u8],
    ) -> bool {
        self.mac(HELPER_LABEL, &[helper_nonce, plugin_nonce, kind, prompt])
            .verify_slice(proof)
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_travels_as_hex_and_hides_itself() {
        let token = Token::new().unwrap();
        let hex = token.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(Token::from_hex(&hex), Some(token.clone()));
        assert_eq!(format!("{token:?}"), "Token(hidden)");
        for bad in ["", "zz", &hex[..62], &format!("{hex}00"), &"é".repeat(32)] {
            assert!(Token::from_hex(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn proofs_hold_only_with_the_token_the_nonces_and_the_prompt() {
        let token = Token::new().unwrap();
        let other = Token::new().unwrap();
        let (h, p) = (random().unwrap(), random().unwrap());
        let plugin = token.plugin_proof(&h, &p);
        assert!(token.check_plugin(&h, &p, &plugin));
        assert!(!other.check_plugin(&h, &p, &plugin));
        assert!(!token.check_plugin(&p, &h, &plugin));
        let prompt = b"ada@build.example's password: ";
        let helper = token.helper_proof(&h, &p, b"", prompt);
        assert!(token.check_helper(&h, &p, b"", prompt, &helper));
        // Another token, a replay with new nonces, another prompt, another
        // kind, or the plugin's proof in the helper's place: all refused.
        assert!(!other.check_helper(&h, &p, b"", prompt, &helper));
        let fresh = random().unwrap();
        assert!(!token.check_helper(&h, &fresh, b"", prompt, &helper));
        assert!(!token.check_helper(&h, &p, b"", b"something else", &helper));
        assert!(!token.check_helper(&h, &p, b"confirm", prompt, &helper));
        assert!(!token.check_helper(&h, &p, b"", prompt, &plugin));
        assert!(!token.check_helper(&h, &p, b"", prompt, &helper[..31]));
    }
}
