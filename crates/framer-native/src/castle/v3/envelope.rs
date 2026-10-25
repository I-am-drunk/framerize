//! Output wrapping from SDK3337eba7, source bytes 276403..281222.
const FNV: u32 = 0x01000193;
const MASK_MULTIPLIER: u32 = 0xf48153f1;

#[derive(Debug, PartialEq, Eq)]
pub enum EnvelopeError {
    TimestampOutsideDateRange,
    PayloadTooLarge,
}

/// Wrap compressed bytes using an explicit wall-clock timestamp in milliseconds.
/// The timestamp must be in JavaScript Date's integral range. This function
/// does not collect device data, establish provenance, or perform network I/O.
pub fn envelope(payload: &[u8], timestamp_ms: i64) -> Result<String, EnvelopeError> {
    if !(-8_640_000_000_000_000..=8_640_000_000_000_000).contains(&timestamp_ms) {
        return Err(EnvelopeError::TimestampOutsideDateRange);
    }
    let size = u32::try_from(payload.len()).map_err(|_| EnvelopeError::PayloadTooLarge)?;
    let timestamp = timestamp_ms.to_string();
    let encoded_time = encode_time(&timestamp);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(encoded_time.len() as u16).to_le_bytes());
    bytes.extend_from_slice(encoded_time.as_bytes());
    bytes.extend(transform(payload, size, &timestamp));
    Ok(format!("cNz8rPYH|{}", base64(&bytes)))
}

fn encode_time(timestamp: &str) -> String {
    let mut bytes = Vec::new();
    for unit in timestamp.encode_utf16() {
        let value = unit.rotate_left(1).wrapping_add(0xc481).rotate_left(2);
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    base64(&bytes)
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        result.push(ALPHABET[(a >> 2) as usize] as char);
        result.push(ALPHABET[((a & 3) << 4 | b >> 4) as usize] as char);
        result.push(if chunk.len() > 1 {
            ALPHABET[((b & 15) << 2 | c >> 6) as usize] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            ALPHABET[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    result
}

fn seed(timestamp: &str) -> u32 {
    let mut state: u32 = 0x751dac79;
    for unit in timestamp.encode_utf16() {
        state = (state ^ unit as u32).wrapping_mul(FNV);
    }
    if state == 0 {
        0x9e3779b9
    } else {
        state
    }
}

fn absorb(state: &mut u32, byte: u8, index: u32, offset: u32, salt: u32) {
    *state ^= byte as u32 + (index.wrapping_add(offset) & 255);
    *state ^= salt;
    *state ^= *state >> 13;
}

fn finish_hash(value: u32, salt: u32, multiplier: u32, rotation_salt: u32) -> u32 {
    let value = value ^ salt;
    let value = (value ^ (value >> 16)).wrapping_mul(multiplier);
    let value = value.rotate_left(13) ^ rotation_salt;
    (value ^ (value >> 15)).wrapping_mul(FNV)
}

fn mask(seed: u32, index: u32) -> u32 {
    let value = (seed ^ index).wrapping_mul(FNV);
    (value ^ (value >> 16)).wrapping_mul(MASK_MULTIPLIER)
}

fn encrypt(payload: &[u8], half: u32, seed: u32) -> (Vec<u8>, u32, u32, u32) {
    let mut all = seed ^ 0x71441d33;
    let mut first = seed ^ 0x46eeff50 ^ 0xfcd08408;
    let mut second = seed ^ 0xe4b77583 ^ 0xfcd08408;
    let mut stream = seed ^ 0x973d8749;
    let mut ciphertext = Vec::with_capacity(payload.len());

    for (index, &byte) in payload.iter().enumerate() {
        let index = index as u32;
        absorb(&mut all, byte, index, 0x6bd6e647, 0x71441d33);
        if index < half {
            absorb(&mut first, byte, index, 0x1db488d0, 0xd8d986e1);
        } else {
            absorb(&mut second, byte, index - half, 0x1db488d0, 0xd8d986e1);
        }
        if index & 3 == 0 {
            stream = stream
                .wrapping_add(index)
                .wrapping_add(0x7f039dda)
                .wrapping_add(0x973d8749);
            stream ^= stream << 13;
            stream ^= stream >> 17;
            stream ^= stream << 5;
            stream = stream.wrapping_add(0x93cf09bd);
        }
        ciphertext.push(byte ^ (stream >> ((index & 3) * 8)) as u8);
    }
    (ciphertext, all, first, second)
}

fn transform(payload: &[u8], size: u32, timestamp: &str) -> Vec<u8> {
    let seed = seed(timestamp);
    let half = size / 2;
    let (ciphertext, all, first, second) = encrypt(payload, half, seed);
    let hashes = [
        finish_hash(all, 0x6abbc1f8, 0x289b52eb, 0x27364664),
        finish_hash(all, 0x01e2f914, 0x76707cd5, 0xa055eaba),
        finish_hash(first, 0xfcd0840b, 0x1db488d1, 0xd8d986e1),
        finish_hash(second, 0xfcd0840d, 0x1db488d1, 0xd8d986e1),
    ];
    let mut output = Vec::new();
    output.extend_from_slice(&(size - half).to_be_bytes());
    output.extend_from_slice(&half.to_be_bytes());
    for (index, hash) in hashes.iter().enumerate() {
        output.extend_from_slice(&(hash ^ mask(seed, index as u32)).to_be_bytes());
    }
    output.extend(ciphertext);
    output
}
