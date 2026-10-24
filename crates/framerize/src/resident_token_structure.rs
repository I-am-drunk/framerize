//! Bounded V13 diagnostics, ported from v2_token_frame.py/v2_token_structure.py.
//! Only field structure escapes. No authentication, payload or retry authority.
//! At most 16,384 encoded bytes and 155 fields; event contents remain unparsed.
//! Python contract/base: be4ff76. Factory: df5a150be914e7d267126f35de92934993e7930f18c3fd00bf79a86bd788ec58.

const MAX_INPUT: usize = 16_384;
const GROUPS: [usize; 5] = [0, 4, 7, 8, 9];
const KEY: [u32; 4] = [0x45678907, 0xe7f2a9b0, 0x0d0d0d0d, 0xa3b5c5d6];
const DELTA: u32 = 0x9e3779b9;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub group_id: usize,
    pub field_id: usize,
    pub kind: usize,
    /// Includes the field header and payload framing, never a decoded value.
    pub byte_length: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Structure {
    pub entries: Vec<Entry>,
    /// Five group headers plus encoded field lengths, excluding events/trailer.
    pub consumed: usize,
}

/// Describe one token without exporting its decoded data or authenticating it.
/// The caller retains lease, identity, egress, deadline and cleanup ownership.
pub(super) fn describe(token: &str) -> Option<Structure> {
    let mut packet = base64url(token)?;
    structure_from_packet(&mut packet)
}

fn base64url(token: &str) -> Option<Vec<u8>> {
    if token.is_empty() || token.len() > MAX_INPUT || token.len() % 4 == 1 {
        return None;
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(token.len() * 3 / 4).ok()?;
    let (mut pending, mut bits) = (0_u32, 0_u32);
    for byte in token.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        pending = (pending << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((pending >> bits) as u8);
            pending &= (1 << bits) - 1;
        }
    }
    (pending == 0).then_some(bytes) // Reject nonzero unused base64 bits.
}

// Private in-place scratch only; the sole exported function returns structure.
fn structure_from_packet(packet: &mut [u8]) -> Option<Structure> {
    if packet.len() < 68 || packet.len() > MAX_INPUT || (packet.len() - 4) % 4 != 0 {
        return None;
    }
    let mask = *packet.first()?;
    let frame = packet.get_mut(1..)?;
    for byte in frame.iter_mut() {
        *byte ^= mask;
    }
    let padding = usize::from(*frame.get(1)?);
    let end = frame.len().checked_sub(1)?;
    if *frame.first()? != 13 || padding > 3
        || *frame.last()? != ((end * 2) & 255) as u8 {
        return None;
    }
    let cipher = frame.get_mut(2..end)?;
    decrypt(cipher)?;
    let plain_len = cipher.len().checked_sub(padding)?;
    if cipher.get(plain_len..)?.iter().any(|byte| *byte != 0) {
        return None;
    }
    let plain = cipher.get_mut(..plain_len)?;
    if plain.len() < 62
        || u16::from_be_bytes(plain.get(6..8)?.try_into().ok()?) >> 11 != 5 {
        return None;
    }
    let (head, body) = plain.split_at_mut(56);
    xor_nibbles(body, head.get(40..56)?, 8, 9)?;
    let (time, inner) = body.split_at_mut(6);
    xor_nibbles(inner, time, 4, 3)?;
    if inner.len() < 11 || inner.last() != Some(&255) {
        return None;
    }
    let structure = parse(inner)?;
    // The marker cannot be consumed as part of the final group's payload.
    (structure.consumed < inner.len()).then_some(structure)
}

fn word(data: &[u8], index: usize) -> Option<u32> {
    let start = index.checked_mul(4)?;
    Some(u32::from_le_bytes(data.get(start..start.checked_add(4)?)?.try_into().ok()?))
}

fn set_word(data: &mut [u8], index: usize, value: u32) -> Option<()> {
    let start = index.checked_mul(4)?;
    data.get_mut(start..start.checked_add(4)?)?.copy_from_slice(&value.to_le_bytes());
    Some(())
}

fn mix(z: u32, y: u32, total: u32, key: u32) -> u32 {
    ((z >> 5) ^ (y << 2)).wrapping_add((y >> 3) ^ (z << 4))
        ^ (total ^ y).wrapping_add(key ^ z)
}

fn decrypt(data: &mut [u8]) -> Option<()> {
    let count = data.len() / 4;
    if count < 2 || data.len() % 4 != 0 || data.len() > MAX_INPUT {
        return None;
    }
    let rounds = 6 + 52 / count;
    let mut total = DELTA.wrapping_mul(rounds as u32);
    for _ in 0..rounds {
        let selector = ((total >> 2) & 3) as usize;
        let mut y = word(data, 0)?;
        for index in (0..count).rev() {
            let z = word(data, if index == 0 { count - 1 } else { index - 1 })?;
            let key = *KEY.get((index & 3) ^ selector)?;
            y = word(data, index)?.wrapping_sub(mix(z, y, total, key));
            set_word(data, index, y)?;
        }
        total = total.wrapping_sub(DELTA);
    }
    Some(())
}

fn nibble(key: &[u8], index: usize) -> Option<u8> {
    let byte = *key.get(index / 2)?;
    Some(if index % 2 == 0 { byte >> 4 } else { byte & 15 })
}

fn xor_nibbles(data: &mut [u8], key: &[u8], length: usize, at: usize) -> Option<()> {
    if !matches!(length, 4 | 8) || data.len() > MAX_INPUT {
        return None;
    }
    let rotation = usize::from(nibble(key, at)?) % length;
    for (index, byte) in data.iter_mut().enumerate() {
        let high = nibble(key, (2 * index + rotation) % length)?;
        let low = nibble(key, (2 * index + 1 + rotation) % length)?;
        *byte ^= (high << 4) | low;
    }
    Some(())
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(count)?;
        let bytes = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(bytes)
    }
    fn byte(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }
    fn uint(&mut self, width: usize) -> Option<usize> {
        self.take(width)?.iter().try_fold(0_usize,
            |value, byte| value.checked_mul(256)?.checked_add(usize::from(*byte)))
    }
}

fn raw_field(cursor: &mut Cursor<'_>, key: (usize, usize)) -> Option<()> {
    // Exact current-source type-7 layouts from v2_token_structure.py.
    let fixed = match key {
        (0, 8) | (0, 31) => 2,
        (0, 9) | (0, 10) | (0, 30) | (4, 19) => 5,
        (0, 28) => 1,
        (4, 21) | (4, 24) | (7, 19) | (7, 22) => 4,
        (7, 23) | (8, 6) => 6,
        (8, 30) | (8, 31) => 10,
        _ => 0,
    };
    if fixed != 0 {
        cursor.take(fixed)?;
        return Some(());
    }
    match key {
        (0, 11) | (0, 14) | (0, 21) | (4, 10) | (4, 13) | (4, 16)
        | (4, 17) | (4, 20) | (4, 23) | (4, 27) | (7, 7) | (7, 14)
        | (7, 17) | (8, 7) | (8, 16) => {
            let bytes = (usize::from(cursor.byte()?) + 7) / 8;
            cursor.take(bytes)?;
        }
        (0, 4) | (7, 18) => {
            for _ in 0..2 {
                if cursor.uint(2)? & 0x8000 == 0 {
                    cursor.take(2)?;
                }
            }
        }
        (7, 12) | (7, 16) => {
            let count = usize::from(cursor.byte()?);
            let expected = if key.1 == 12 { 9 } else { 2 };
            if count != expected {
                return None;
            }
            cursor.take(count * 6)?;
        }
        (0, 12) | (7, 6) => {
            let width = usize::from(cursor.byte()?);
            if width > 6 {
                return None;
            }
            let count = cursor.uint(width)?;
            if width > 0 && (count as u64) < (1_u64 << (8 * (width - 1))) {
                return None;
            }
            let cap = if key.0 == 0 { 512 } else { 2048 };
            if count > cap {
                return None;
            }
            cursor.take(count)?;
        }
        _ => return None,
    }
    Some(())
}

fn payload(cursor: &mut Cursor<'_>, group: usize, field: usize, kind: usize) -> Option<()> {
    match kind {
        1 | 2 => {}
        3 | 6 => { cursor.take(1)?; }
        4 => {
            let count = usize::from(cursor.byte()?);
            cursor.take(count)?;
        }
        5 => {
            let first = usize::from(cursor.byte()?);
            if first & 0x80 != 0
                && (((first & 0x7f) << 8) | usize::from(cursor.byte()?)) <= 127 {
                return None;
            }
        }
        7 => raw_field(cursor, (group, field))?,
        _ => return None,
    }
    Some(())
}

fn parse(data: &[u8]) -> Option<Structure> {
    if !(10..=MAX_INPUT).contains(&data.len()) {
        return None;
    }
    let mut cursor = Cursor { data, pos: 0 };
    let mut entries = Vec::new();
    entries.try_reserve_exact(5 * 31).ok()?;
    for group in GROUPS {
        let header = cursor.uint(2)?;
        if header >> 5 != group {
            return None;
        }
        let mut previous = None;
        for _ in 0..(header & 31) {
            let start = cursor.pos;
            let tag = usize::from(cursor.byte()?);
            let (field, kind) = (tag >> 3, tag & 7);
            if previous.is_some_and(|previous| field <= previous) {
                return None;
            }
            previous = Some(field);
            payload(&mut cursor, group, field, kind)?;
            entries.push(Entry { group_id: group, field_id: field, kind,
                byte_length: cursor.pos.checked_sub(start)? });
        }
    }
    Some(Structure { entries, consumed: cursor.pos })
}

#[cfg(test)]
#[path = "resident_token_structure_tests.rs"]
mod tests;
