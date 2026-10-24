//! Inert source vectors only. No SDK, account, token capture or network input.
//! All execution is reserved to the runtime owner.
use super::*;

const VECTORS: &str = include_str!("../../../reference/fixtures/data/v2_frame_vectors.json");

fn vector(name: &str) -> &'static str {
    VECTORS.lines().find_map(|line| {
        let (key, value) = line.trim().split_once(':')?;
        if key.strip_prefix('"')?.strip_suffix('"')? != name { return None; }
        value.trim().trim_end_matches(',').strip_prefix('"')?.strip_suffix('"')
    }).expect("named synthetic source vector")
}

fn hex(value: &str) -> Vec<u8> {
    let text: String = value.chars().filter(|c| !c.is_whitespace()).collect();
    text.as_bytes().chunks_exact(2).map(|pair|
        u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect()
}

fn field(id: u8, kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![(id << 3) | kind];
    out.extend_from_slice(payload);
    out
}

fn section(selected: usize, fields: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for group in [0, 4, 7, 8, 9] {
        let count = if group == selected { fields.len() } else { 0 };
        out.extend_from_slice(&(((group << 5) | count) as u16).to_be_bytes());
        if group == selected { out.extend(fields.iter().flatten().copied()); }
    }
    out
}

fn entry(group_id: usize, field_id: usize, kind: usize, byte_length: usize) -> Entry {
    Entry { group_id, field_id, kind, byte_length }
}

fn encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::new();
    for chunk in data.chunks(3) {
        let word = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..chunk.len() + 1 {
            output.push(ALPHABET[((word >> (18 - 6 * index)) & 63) as usize] as char);
        }
    }
    output
}

fn masked(data: &[u8], key: &[u8], width: usize, at: usize) -> Vec<u8> {
    let digits: Vec<u8> = key.iter().flat_map(|b| [b >> 4, b & 15]).collect();
    let rotation = usize::from(digits[at]) % width;
    data.iter().flat_map(|byte| [byte >> 4, byte & 15]).enumerate()
        .map(|(index, nibble)| nibble ^ digits[(index + rotation) % width])
        .collect::<Vec<_>>().chunks_exact(2).map(|pair| pair[0] * 16 + pair[1]).collect()
}

// Test-only forward cipher; production has no token construction entry point.
fn encrypt(data: &[u8]) -> Vec<u8> {
    let key = [0x45678907_u32, 0xe7f2a9b0, 0x0d0d0d0d, 0xa3b5c5d6];
    let mut words: Vec<u32> = data.chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap())).collect();
    let count = words.len();
    let mut z = words[count - 1];
    let mut total = 0_u32;
    for _ in 0..(6 + 52 / count) {
        total = total.wrapping_add(0x9e3779b9);
        let selector = ((total >> 2) & 3) as usize;
        for index in 0..count {
            let y = words[(index + 1) % count];
            let change = ((z >> 5) ^ (y << 2)).wrapping_add((y >> 3) ^ (z << 4))
                ^ (total ^ y).wrapping_add(key[(index & 3) ^ selector] ^ z);
            words[index] = words[index].wrapping_add(change);
            z = words[index];
        }
    }
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}

fn wrap_plain(mut plain: Vec<u8>, padding_byte: u8) -> String {
    let padding = (4 - plain.len() % 4) % 4;
    plain.extend(std::iter::repeat(padding_byte).take(padding));
    let mut frame = vec![13, padding as u8];
    frame.extend(encrypt(&plain));
    frame.push(((frame.len() * 2) & 255) as u8);
    let mut packet = vec![0x42];
    packet.extend(frame.into_iter().map(|byte| byte ^ 0x42));
    encode(&packet)
}

fn plain(inner: &[u8]) -> Vec<u8> {
    let device = hex("0123456789abcdef1032547698badcfe");
    let time = hex("123756789abc");
    let mut body = time.clone();
    body.extend(masked(inner, &time, 4, 3));
    let mut plain = vec![0; 56];
    plain[6..8].copy_from_slice(&(5_u16 << 11).to_be_bytes());
    plain[40..56].copy_from_slice(&device);
    plain.extend(masked(&body, &device, 8, 9));
    plain
}

fn token_for(section: &[u8]) -> String {
    let mut inner = section.to_vec();
    inner.extend([0, 0, 255]);
    wrap_plain(plain(&inner), 0)
}

fn expect_section(data: &[u8], entries: Vec<Entry>) {
    let expected = Structure { entries, consumed: data.len() };
    assert_eq!(parse(data).as_ref(), Some(&expected));
    assert_eq!(describe(&token_for(data)), Some(expected));
}

#[test]
fn saved_empty_vector_preserves_unconsumed_trailer() {
    let inner = hex(vector("empty_groups_inner"));
    assert_eq!(&inner[10..], &[0, 0, 255]);
    let expected = Structure { entries: vec![], consumed: 10 };
    assert_eq!(parse(&inner).as_ref(), Some(&expected));
    assert_eq!(describe(vector("empty_groups_token")), Some(expected));
    for name in ["wrong_group_count", "bad_padding", "missing_terminator"] {
        assert_eq!(describe(vector(name)), None);
    }
}

#[test]
fn published_golden_frame_matches_inner_before_group_validation() {
    // The public golden frame has valid framing but not the current five groups.
    let mut packet = base64url(vector("golden_token")).unwrap();
    let mask = packet[0];
    let frame = &mut packet[1..];
    for byte in frame.iter_mut() { *byte ^= mask; }
    let padding = usize::from(frame[1]);
    let end = frame.len() - 1;
    let cipher = &mut frame[2..end];
    decrypt(cipher).unwrap();
    let plain_len = cipher.len() - padding;
    let (head, body) = cipher[..plain_len].split_at_mut(56);
    xor_nibbles(body, &head[40..56], 8, 9).unwrap();
    let (time, inner) = body.split_at_mut(6);
    xor_nibbles(inner, time, 4, 3).unwrap();
    assert_eq!(&*inner, hex(vector("golden_inner")).as_slice());
    assert_eq!(describe(vector("golden_token")), None);
}

#[test]
fn canonical_alphabet_unused_bits_and_input_limit() {
    assert_eq!(base64url("AA"), Some(vec![0]));
    assert_eq!(base64url("AAA"), Some(vec![0, 0]));
    assert_eq!(base64url("AAAA"), Some(vec![0, 0, 0]));
    for text in ["", "A", "AAAAA", "AB", "AAB", "AA=", "+AAA", "/AAA", "é", "AA\n"] {
        assert_eq!(base64url(text), None);
        assert_eq!(describe(text), None);
    }
    assert!(base64url(&"A".repeat(16_384)).is_some());
    assert_eq!(base64url(&"A".repeat(16_385)), None);
    let token = vector("empty_groups_token");
    for invalid in [format!("{token}="), format!("{token}\n"), "a".repeat(16_385)] {
        assert_eq!(describe(&invalid), None);
    }
}

#[test]
fn frame_version_padding_marker_and_every_prefix_refuse() {
    let token = vector("empty_groups_token");
    for end in 0..token.len() { assert_eq!(describe(&token[..end]), None); }
    let packet = base64url(token).unwrap();
    for (offset, value) in [(1, 11), (1, 12), (2, 4), (packet.len() - 1, 0)] {
        let mut changed = packet.clone();
        changed[offset] = value ^ changed[0];
        assert_eq!(describe(&encode(&changed)), None);
    }
    let mut short = packet.clone();
    short.remove(3);
    assert_eq!(describe(&encode(&short)), None);
    assert_eq!(describe(&encode(&packet[..64])), None);
}

#[test]
fn inner_counts_padding_and_trailer_boundary_are_independent() {
    let empty = section(99, &[]);
    for tail in 0..4 {
        let mut inner = empty.clone();
        inner.extend(std::iter::repeat(0x66).take(tail));
        inner.push(255);
        let bytes = plain(&inner);
        assert!(describe(&wrap_plain(bytes.clone(), 0)).is_some());
        if bytes.len() % 4 != 0 {
            assert_eq!(describe(&wrap_plain(bytes.clone(), 1)), None);
        }
        let mut wrong = bytes.clone();
        wrong[6..8].copy_from_slice(&(4_u16 << 11).to_be_bytes());
        assert_eq!(describe(&wrap_plain(wrong, 0)), None);
        let mut integration = bytes;
        integration[6..8].copy_from_slice(&((5_u16 << 11) | 0x7ff).to_be_bytes());
        assert!(describe(&wrap_plain(integration, 0)).is_some());
    }
    let consumed_marker = section(9, &[field(31, 4, &[1, 255])]);
    assert_eq!(parse(&consumed_marker).unwrap().consumed, consumed_marker.len());
    assert_eq!(describe(&wrap_plain(plain(&consumed_marker), 0)), None);
    assert!(describe(&token_for(&consumed_marker)).is_some());
    for length in [56, 61, 62] {
        let mut too_short = vec![0; length];
        too_short[6..8].copy_from_slice(&(5_u16 << 11).to_be_bytes());
        assert_eq!(describe(&wrap_plain(too_short, 0)), None);
    }
}

#[test]
fn literal_five_groups_cover_all_generic_types() {
    let data = hex("0007 01 0a 13ff 1c03616263 257f 2d8080 362a 0081 fc00 00e0 0100 0120");
    expect_section(&data, vec![entry(0, 0, 1, 1), entry(0, 1, 2, 1),
        entry(0, 2, 3, 2), entry(0, 3, 4, 5), entry(0, 4, 5, 2),
        entry(0, 5, 5, 3), entry(0, 6, 6, 2), entry(4, 31, 4, 2)]);
    assert_eq!(data.len(), 28);
}

#[test]
fn every_fixed_type_seven_layout_matches_python() {
    let cases = [(0, 8, 2), (0, 9, 5), (0, 10, 5), (0, 28, 1),
        (0, 30, 5), (0, 31, 2), (4, 19, 5), (4, 21, 4), (4, 24, 4),
        (7, 19, 4), (7, 22, 4), (7, 23, 6), (8, 6, 6), (8, 30, 10), (8, 31, 10)];
    for (g, f, length) in cases {
        let data = section(g, &[field(f, 7, &vec![0; length])]);
        expect_section(&data, vec![entry(g, usize::from(f), 7, length + 1)]);
        assert_eq!(parse(&data[..2 + length]), None);
    }
}

#[test]
fn all_bitset_layouts_cover_byte_and_chunk_boundaries() {
    let keys = [(0, 11), (0, 14), (0, 21), (4, 10), (4, 13), (4, 16),
        (4, 17), (4, 20), (4, 23), (4, 27), (7, 7), (7, 14), (7, 17), (8, 7), (8, 16)];
    for (g, f) in keys {
        for bits in [0_u8, 1, 7, 8, 9, 23, 24, 25, 255] {
            let mut payload = vec![bits];
            payload.extend(vec![0; (usize::from(bits) + 7) / 8]);
            let data = section(g, &[field(f, 7, &payload)]);
            expect_section(&data, vec![entry(g, usize::from(f), 7, payload.len() + 1)]);
        }
    }
}

#[test]
fn dimension_pairs_read_each_compression_flag() {
    for (g, f) in [(0, 4), (7, 18)] {
        for a in [hex("8001"), hex("00020003")] {
            for b in [hex("8001"), hex("00020003")] {
                let payload = [a.as_slice(), b.as_slice()].concat();
                let data = section(g, &[field(f, 7, &payload)]);
                expect_section(&data, vec![entry(g, usize::from(f), 7, payload.len() + 1)]);
            }
        }
    }
}

#[test]
fn six_byte_arrays_require_exact_current_counts() {
    for (f, count) in [(12, 9_u8), (16, 2)] {
        let mut payload = vec![count];
        payload.extend(vec![0; usize::from(count) * 6]);
        let data = section(7, &[field(f, 7, &payload)]);
        expect_section(&data, vec![entry(7, usize::from(f), 7, payload.len() + 1)]);
        for wrong in [0, count - 1, count + 1, 255] {
            let mut payload = vec![wrong];
            payload.extend(vec![0; usize::from(wrong) * 6]);
            let data = section(7, &[field(f, 7, &payload)]);
            assert_eq!(parse(&data), None);
            assert_eq!(describe(&token_for(&data)), None);
        }
    }
}

fn long_payload(length: usize) -> Vec<u8> {
    let mut out = if length == 0 { vec![0] }
        else if length < 256 { vec![1, length as u8] }
        else { vec![2, (length >> 8) as u8, length as u8] };
    out.extend(vec![0; length]);
    out
}

#[test]
fn long_byte_fields_accept_only_canonical_bounded_lengths() {
    for (g, f, cap) in [(0, 12, 512), (7, 6, 2048)] {
        for length in [0, 1, 255, 256, cap] {
            let payload = long_payload(length);
            let data = section(g, &[field(f, 7, &payload)]);
            expect_section(&data, vec![entry(g, usize::from(f), 7, payload.len() + 1)]);
        }
        let too_long = section(g, &[field(f, 7, &long_payload(cap + 1))]);
        assert_eq!(parse(&too_long), None);
        assert_eq!(describe(&token_for(&too_long)), None);
        for prefix in [vec![7], vec![255], vec![1, 0], vec![2, 0, 1],
            vec![6, 255, 255, 255, 255, 255, 255]] {
            assert_eq!(parse(&section(g, &[field(f, 7, &prefix)])), None);
        }
        for width in 3..=6 {
            for leading in [0, 1, 255] {
                let mut prefix = vec![width as u8, leading];
                prefix.extend(vec![0; width - 1]);
                assert_eq!(parse(&section(g, &[field(f, 7, &prefix)])), None);
            }
        }
    }
}

#[test]
fn short_strings_and_packed_integer_boundaries() {
    for count in [0_u8, 1, 255] {
        let mut payload = vec![count];
        payload.extend(vec![0; usize::from(count)]);
        let data = section(0, &[field(0, 4, &payload)]);
        expect_section(&data, vec![entry(0, 0, 4, payload.len() + 1)]);
    }
    for payload in [vec![0], vec![127], vec![128, 128], vec![255, 255]] {
        let data = section(0, &[field(0, 5, &payload)]);
        expect_section(&data, vec![entry(0, 0, 5, payload.len() + 1)]);
    }
    for payload in [vec![128, 0], vec![128, 127]] {
        let data = section(0, &[field(0, 5, &payload)]);
        assert_eq!(parse(&data), None);
        assert_eq!(describe(&token_for(&data)), None);
    }
}

#[test]
fn unknown_types_and_unmapped_raw_fields_fail_closed() {
    for g in [0, 4, 7, 8, 9] {
        for kind in [0, 7] {
            let data = section(g, &[field(0, kind, &[0; 12])]);
            assert_eq!(parse(&data), None);
            assert_eq!(describe(&token_for(&data)), None);
        }
    }
}

#[test]
fn group_identity_order_and_field_order_are_required() {
    for ids in [[1, 4, 7, 8, 9], [2047, 4, 7, 8, 9], [0, 7, 4, 8, 9],
        [0, 4, 4, 8, 9], [0, 4, 7, 9, 8]] {
        let bytes: Vec<u8> = ids.into_iter().flat_map(|g| ((g as u16) << 5).to_be_bytes()).collect();
        assert_eq!(parse(&bytes), None);
        assert_eq!(describe(&token_for(&bytes)), None);
    }
    for ids in [[1, 1], [2, 1]] {
        let bytes = section(0, &[field(ids[0], 1, &[]), field(ids[1], 1, &[])]);
        assert_eq!(parse(&bytes), None);
        assert_eq!(describe(&token_for(&bytes)), None);
    }
    for claimed in [0, 2, 31] {
        let mut bytes = section(0, &[field(0, 1, &[])]);
        bytes[1] = claimed;
        assert_eq!(parse(&bytes), None);
        assert_eq!(describe(&token_for(&bytes)), None);
    }
}

#[test]
fn all_five_groups_allow_the_full_field_count() {
    let mut bytes = Vec::new();
    let mut entries = Vec::new();
    for group in [0, 4, 7, 8, 9] {
        bytes.extend_from_slice(&(((group << 5) | 31) as u16).to_be_bytes());
        for field in 0..31 {
            bytes.push(((field << 3) | 1) as u8);
            entries.push(entry(group, field, 1, 1));
        }
    }
    assert_eq!(entries.len(), 155);
    expect_section(&bytes, entries);
}

#[test]
fn every_incomplete_group_prefix_refuses_without_partial_output() {
    let cases = [section(99, &[]), section(0, &[field(1, 4, b"\x03abc")]),
        section(0, &[field(1, 5, &[255, 255])]),
        section(0, &[field(4, 7, &hex("800100020003"))]),
        section(4, &[field(16, 7, &[9, 0, 0])]),
        section(7, &[field(12, 7, &[vec![9], vec![0; 54]].concat())]),
        section(7, &[field(6, 7, b"\x01\x03abc")]),
        section(8, &[field(30, 7, &[0; 10]), field(31, 7, &[0; 10])])];
    for data in cases {
        assert_eq!(parse(&data).unwrap().consumed, data.len());
        for end in 0..data.len() { assert_eq!(parse(&data[..end]), None); }
    }
}

#[test]
fn section_size_and_checked_cursor_arithmetic() {
    for length in [0, 9, 16_385] { assert_eq!(parse(&vec![0; length]), None); }
    let mut data = section(99, &[]);
    data.resize(16_384, 0);
    assert_eq!(parse(&data), Some(Structure { entries: vec![], consumed: 10 }));
    let mut cursor = Cursor { data: &[1, 2], pos: 0 };
    assert_eq!(cursor.take(usize::MAX), None);
    assert_eq!(cursor.pos, 0);
    assert_eq!(cursor.uint(2), Some(258));
    assert_eq!(cursor.byte(), None);
    cursor.pos = usize::MAX;
    assert_eq!(cursor.take(1), None);
    assert_eq!(word(&[], usize::MAX), None);
    assert_eq!(set_word(&mut [], usize::MAX, 0), None);
    assert_eq!(xor_nibbles(&mut [0], &[], 0, 0), None);
    assert_eq!(xor_nibbles(&mut [0], &[], 8, 9), None);
}

#[test]
fn equal_width_payloads_remain_indistinguishable_but_boolean_tags_do_not() {
    let marker = b"synthetic-private-marker";
    let mut first = vec![marker.len() as u8];
    first.extend(marker);
    let mut second = vec![marker.len() as u8];
    second.extend(vec![b'x'; marker.len()]);
    let a = token_for(&section(0, &[field(0, 4, &first)]));
    let b = token_for(&section(0, &[field(0, 4, &second)]));
    assert_ne!(a, b);
    let result = describe(&a).unwrap();
    assert_eq!(Some(&result), describe(&b).as_ref());
    assert!(!format!("{result:?}").contains("synthetic-private-marker"));
    assert_eq!(result.consumed, 10 + result.entries.iter().map(|e| e.byte_length).sum::<usize>());
    let low = describe(&token_for(&section(0, &[field(0, 1, &[])]))).unwrap();
    let high = describe(&token_for(&section(0, &[field(0, 2, &[])]))).unwrap();
    assert_eq!(low.consumed, high.consumed);
    assert_ne!(low, high);
}

#[test]
fn adversarial_bytes_and_ciphertext_mutations_do_not_panic() {
    let mut seed = 0x1234abcd_u32;
    for length in 0..=256 {
        let bytes: Vec<u8> = (0..length).map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        }).collect();
        let token = encode(&bytes);
        assert!(std::panic::catch_unwind(|| describe(&token)).is_ok());
        assert!(std::panic::catch_unwind(|| parse(&bytes)).is_ok());
    }
    let original = base64url(vector("empty_groups_token")).unwrap();
    for index in 0..original.len() {
        for bit in 0..8 {
            let mut altered = original.clone();
            altered[index] ^= 1 << bit;
            let token = encode(&altered);
            let result = std::panic::catch_unwind(|| describe(&token)).unwrap();
            if let Some(structure) = result {
                // Structural validity is not authentication of ciphertext.
                assert!(structure.entries.len() <= 155);
                assert!(structure.consumed <= MAX_INPUT);
                assert_eq!(structure.consumed, 10 + structure.entries.iter()
                    .map(|e| e.byte_length).sum::<usize>());
            }
        }
    }
}
