#[derive(Clone)]
struct Package {
    weight: u64,
    symbols: Vec<usize>,
}

pub(super) fn lengths(frequencies: &[u64], maximum: usize) -> Vec<u8> {
    let mut result = vec![0; frequencies.len()];
    let mut leaves: Vec<_> = frequencies.iter().enumerate()
        .filter(|(_, weight)| **weight > 0)
        .map(|(symbol, &weight)| Package { weight, symbols: vec![symbol] }).collect();
    if leaves.len() <= 1 {
        if let Some(leaf) = leaves.first() {
            result[leaf.symbols[0]] = 1;
        }
        return result;
    }
    leaves.sort_by_key(|leaf| leaf.weight);
    let mut level = leaves.clone();
    for _ in 2..=maximum {
        let packages: Vec<_> = level.chunks_exact(2).map(|pair| {
            let mut symbols = pair[0].symbols.clone();
            symbols.extend_from_slice(&pair[1].symbols);
            Package { weight: pair[0].weight + pair[1].weight, symbols }
        }).collect();
        level = merge(&leaves, &packages);
    }
    for package in level.iter().take(2 * leaves.len() - 2) {
        for &symbol in &package.symbols {
            result[symbol] += 1;
        }
    }
    result
}

fn merge(leaves: &[Package], packages: &[Package]) -> Vec<Package> {
    let (mut leaf, mut package) = (0, 0);
    let mut result = Vec::with_capacity(leaves.len() + packages.len());
    while leaf < leaves.len() || package < packages.len() {
        if package == packages.len()
            || leaf < leaves.len() && leaves[leaf].weight <= packages[package].weight
        {
            result.push(leaves[leaf].clone());
            leaf += 1;
        } else {
            result.push(packages[package].clone());
            package += 1;
        }
    }
    result
}

pub(super) fn codes(lengths: &[u8], maximum: usize) -> Vec<u32> {
    let mut counts = vec![0; maximum + 1];
    for &length in lengths.iter().filter(|&&length| length > 0) {
        counts[length as usize] += 1;
    }
    let mut next = vec![0; maximum + 1];
    for bits in 1..=maximum {
        next[bits] = (next[bits - 1] + counts[bits - 1]) << 1;
    }
    lengths.iter().map(|&length| {
        if length == 0 { return 0; }
        let slot = &mut next[length as usize];
        let code = *slot;
        *slot += 1;
        code
    }).collect()
}
