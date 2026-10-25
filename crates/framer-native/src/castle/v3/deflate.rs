//! Exact raw-DEFLATE fallback from SDK3337eba7, bytes 575295..581410.
//! The SDK can also select a platform CompressionStream with different bytes.
mod huffman;
mod matches;
use matches::Token;

const LENGTHS: [u16; 29] = [3,4,5,6,7,8,9,10,11,13,15,17,19,23,27,31,35,43,51,59,67,83,99,115,131,163,195,227,258];
const LENGTH_BITS: [u8; 29] = [0,0,0,0,0,0,0,0,1,1,1,1,2,2,2,2,3,3,3,3,4,4,4,4,5,5,5,5,0];
const DISTANCES: [u16; 30] = [1,2,3,4,5,7,9,13,17,25,33,49,65,97,129,193,257,385,513,769,1025,1537,2049,3073,4097,6145,8193,12289,16385,24577];
const DISTANCE_BITS: [u8; 30] = [0,0,0,0,1,1,2,2,3,3,4,4,5,5,6,6,7,7,8,8,9,9,10,10,11,11,12,12,13,13];
const ORDER: [usize; 19] = [16,17,18,0,8,7,9,6,10,5,11,4,12,3,13,2,14,1,15];

#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    accumulator: u32,
    used: u8,
}

impl Bits {
    fn write(&mut self, value: u32, width: u8) {
        if width == 0 { return; }
        self.accumulator |= (value & ((1 << width) - 1)) << self.used;
        self.used += width;
        while self.used >= 8 {
            self.bytes.push(self.accumulator as u8);
            self.accumulator >>= 8;
            self.used -= 8;
        }
    }

    fn code(&mut self, value: u32, width: u8) {
        if width > 0 {
            self.write(value.reverse_bits() >> (32 - width), width);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.used > 0 {
            self.bytes.push(self.accumulator as u8);
        }
        self.bytes
    }
}

fn range(value: u16, bases: &[u16], widths: &[u8]) -> (usize, u8, u16) {
    bases.iter().zip(widths).enumerate().find_map(|(code, (&base, &width))| {
        (value >= base && value <= base + ((1 << width) - 1))
            .then(|| (code, width, value - base))
    }).expect("match is within the SDK's length/distance range")
}

fn encode_lengths(lengths: &[u8]) -> Vec<(u8, u8, u8)> {
    let mut output = Vec::new();
    let mut at = 0;
    while at < lengths.len() {
        let value = lengths[at];
        let mut end = at + 1;
        while end < lengths.len() && lengths[end] == value { end += 1; }
        let mut count = end - at;
        if value != 0 {
            output.push((value, 0, 0));
            count -= 1;
            while count >= 3 {
                let take = count.min(6);
                output.push((16, (take - 3) as u8, 2));
                count -= take;
            }
        } else {
            while count >= 11 {
                let take = count.min(138);
                output.push((18, (take - 11) as u8, 7));
                count -= take;
            }
            while count >= 3 {
                let take = count.min(10);
                output.push((17, (take - 3) as u8, 3));
                count -= take;
            }
        }
        output.extend(std::iter::repeat_n((value, 0, 0), count));
        at = end;
    }
    output
}

/// Compress bytes using the pinned SDK's deterministic fallback algorithm.
/// Produces one final dynamic-Huffman DEFLATE block without a zlib wrapper.
pub fn deflate(input: &[u8]) -> Vec<u8> {
    let tokens = matches::tokenize(input);
    let mut literal_frequency = vec![0; 286];
    let mut distance_frequency = vec![0; 30];
    for token in &tokens {
        match *token {
            Token::Literal(byte) => literal_frequency[byte as usize] += 1,
            Token::Match { length, distance } => {
                literal_frequency[257 + range(length, &LENGTHS, &LENGTH_BITS).0] += 1;
                distance_frequency[range(distance, &DISTANCES, &DISTANCE_BITS).0] += 1;
            }
        }
    }
    literal_frequency[256] += 1;
    let literals = huffman::lengths(&literal_frequency, 15);
    let mut distances = huffman::lengths(&distance_frequency, 15);
    if distance_frequency.iter().all(|&count| count == 0) {
        distances[0] = 1;
        distances[1] = 1;
    }
    let literal_count = (literals.iter().rposition(|&n| n > 0).unwrap() + 1).max(257);
    let distance_count = distances.iter().rposition(|&n| n > 0).unwrap() + 1;
    finish_block(&tokens, &literals[..literal_count], &distances[..distance_count])
}

fn finish_block(tokens: &[Token], literals: &[u8], distances: &[u8]) -> Vec<u8> {
    let mut combined = literals.to_vec();
    combined.extend_from_slice(distances);
    let runs = encode_lengths(&combined);
    let mut frequency = vec![0; 19];
    for &(symbol, _, _) in &runs { frequency[symbol as usize] += 1; }
    let lengths = huffman::lengths(&frequency, 7);
    let count = (ORDER.iter().rposition(|&symbol| lengths[symbol] > 0).unwrap() + 1).max(4);
    let codes = huffman::codes(&lengths, 7);
    let literal_codes = huffman::codes(literals, 15);
    let distance_codes = huffman::codes(distances, 15);
    let mut bits = Bits::default();
    bits.write(1, 1);
    bits.write(2, 2);
    bits.write((literals.len() - 257) as u32, 5);
    bits.write((distances.len() - 1) as u32, 5);
    bits.write((count - 4) as u32, 4);
    for &symbol in &ORDER[..count] { bits.write(lengths[symbol] as u32, 3); }
    for (symbol, extra, width) in runs {
        bits.code(codes[symbol as usize], lengths[symbol as usize]);
        bits.write(extra as u32, width);
    }
    write_tokens(&mut bits, tokens, literals, distances, &literal_codes, &distance_codes);
    bits.code(literal_codes[256], literals[256]);
    bits.finish()
}

fn write_tokens(
    bits: &mut Bits,
    tokens: &[Token],
    literals: &[u8],
    distances: &[u8],
    literal_codes: &[u32],
    distance_codes: &[u32],
) {
    for token in tokens {
        match *token {
            Token::Literal(byte) => bits.code(literal_codes[byte as usize], literals[byte as usize]),
            Token::Match { length, distance } => {
                let (code, width, extra) = range(length, &LENGTHS, &LENGTH_BITS);
                bits.code(literal_codes[257 + code], literals[257 + code]);
                bits.write(extra as u32, width);
                let (code, width, extra) = range(distance, &DISTANCES, &DISTANCE_BITS);
                bits.code(distance_codes[code], distances[code]);
                bits.write(extra as u32, width);
            }
        }
    }
}
