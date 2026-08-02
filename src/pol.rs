use sha2::{Digest, Sha256};

pub fn meets_difficulty(digest: &[u8], bits: u32) -> bool {
    let mut remaining = bits;
    for byte in digest {
        if remaining == 0 {
            return true;
        }
        let zeros = byte.leading_zeros();
        if zeros >= remaining.min(8) {
            remaining = remaining.saturating_sub(8);
        } else {
            return false;
        }
    }
    remaining == 0
}

pub fn digest_for(challenge: &str, nonce: u64) -> Vec<u8> {
    Sha256::digest(format!("{challenge}:{nonce}").as_bytes()).to_vec()
}

pub fn verify(challenge: &str, nonce: u64, bits: u32) -> bool {
    meets_difficulty(&digest_for(challenge, nonce), bits)
}

pub fn solve(challenge: &str, bits: u32) -> u64 {
    let mut nonce = 0u64;
    loop {
        if verify(challenge, nonce, bits) {
            return nonce;
        }
        nonce += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difficulty_bits() {
        assert!(meets_difficulty(&[0b0000_1000, 0xFF], 4));
        assert!(!meets_difficulty(&[0b0000_1000, 0xFF], 5));
        assert!(meets_difficulty(&[0, 0x0F, 0xFF], 12));
        assert!(!meets_difficulty(&[0, 0x0F, 0xFF], 13));
        assert!(meets_difficulty(&[0xFF], 0));
    }

    #[test]
    fn solve_and_verify() {
        let nonce = solve("test-challenge", 10);
        assert!(verify("test-challenge", nonce, 10));
        assert!(!verify("other-challenge", nonce, 10));
    }
}
