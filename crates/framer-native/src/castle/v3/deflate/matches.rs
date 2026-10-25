#[derive(Clone, Copy)]
pub(super) enum Token {
    Literal(u8),
    Match { length: u16, distance: u16 },
}

fn hash(input: &[u8], at: usize) -> usize {
    (251 * input[at] as usize + 11 * input[at + 1] as usize + input[at + 2] as usize)
        & 32767
}

fn insert(input: &[u8], at: usize, heads: &mut [usize], previous: &mut [usize]) {
    if at + 2 < input.len() {
        let key = hash(input, at);
        previous[at] = heads[key];
        heads[key] = at + 1;
    }
}

fn find(input: &[u8], at: usize, heads: &[usize], previous: &[usize]) -> (usize, usize) {
    if at + 2 >= input.len() {
        return (0, 0);
    }
    let oldest = at.saturating_sub(1024);
    let limit = (input.len() - at).min(258);
    let mut link = heads[hash(input, at)];
    let mut best = (0, 0);
    for _ in 0..64 {
        let Some(candidate) = link.checked_sub(1) else { break };
        if candidate < oldest {
            break;
        }
        if input[candidate..candidate + 3] == input[at..at + 3] {
            let mut length = 3;
            while length < limit && input[candidate + length] == input[at + length] {
                length += 1;
            }
            if length > best.0 {
                best = (length, at - candidate);
                if length == 258 {
                    break;
                }
            }
        }
        link = previous[candidate];
    }
    best
}

pub(super) fn tokenize(input: &[u8]) -> Vec<Token> {
    if input.len() < 3 {
        return input.iter().copied().map(Token::Literal).collect();
    }
    let mut heads = vec![0; 32768];
    let mut previous = vec![0; input.len()];
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < input.len() {
        let (length, distance) = find(input, at, &heads, &previous);
        let consumed = if length >= 3 {
            tokens.push(Token::Match { length: length as u16, distance: distance as u16 });
            length
        } else {
            tokens.push(Token::Literal(input[at]));
            1
        };
        for offset in 0..consumed {
            insert(input, at + offset, &mut heads, &mut previous);
        }
        at += consumed;
    }
    tokens
}
