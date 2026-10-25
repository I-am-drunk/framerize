//! Sample only this measurement process and its owned descendants.
use std::{collections::BTreeMap, fs, io, sync::mpsc, thread, time::Duration};
use serde::Serialize;

#[derive(Clone, Copy, Default, Serialize)]
pub struct Memory { rss_kib: u64, pss_kib: u64, private_kib: u64 }

impl Memory {
    fn add(self, other: Self) -> io::Result<Self> {
        Ok(Self {
            rss_kib: self.rss_kib.checked_add(other.rss_kib).ok_or_else(invalid)?,
            pss_kib: self.pss_kib.checked_add(other.pss_kib).ok_or_else(invalid)?,
            private_kib: self.private_kib.checked_add(other.private_kib).ok_or_else(invalid)?,
        })
    }
    fn peak(&mut self, other: Self) {
        self.rss_kib = self.rss_kib.max(other.rss_kib);
        self.pss_kib = self.pss_kib.max(other.pss_kib);
        self.private_kib = self.private_kib.max(other.private_kib);
    }
}

fn invalid() -> io::Error { io::Error::other("incomplete memory observation") }

fn parse_memory(text: &str) -> io::Result<Memory> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let Some((name, value)) = line.split_once(':') else { continue };
        if !["Rss", "Pss", "Private_Clean", "Private_Dirty", "Private_Hugetlb"].contains(&name) {
            continue;
        }
        let mut parts = value.split_whitespace();
        let number: u64 = parts.next().ok_or_else(invalid)?.parse().map_err(|_| invalid())?;
        if parts.next() != Some("kB") || parts.next().is_some()
            || values.insert(name, number).is_some() { return Err(invalid()); }
    }
    let get = |key| values.get(key).copied().ok_or_else(invalid);
    let private_kib = get("Private_Clean")?.checked_add(get("Private_Dirty")?)
        .and_then(|n| n.checked_add(values.get("Private_Hugetlb").copied().unwrap_or(0)))
        .ok_or_else(invalid)?;
    Ok(Memory { rss_kib: get("Rss")?, pss_kib: get("Pss")?, private_kib })
}

fn process_tree(root: u32) -> io::Result<BTreeMap<u32, u64>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![root];
    while let Some(pid) = pending.pop() {
        if found.contains_key(&pid) { continue; }
        found.insert(pid, identity(pid)?);
        for task in fs::read_dir(format!("/proc/{pid}/task"))? {
            let children = fs::read_to_string(task?.path().join("children"))?;
            for child in children.split_whitespace() {
                pending.push(child.parse().map_err(|_| invalid())?);
            }
        }
    }
    Ok(found)
}

struct Sample { owner: Memory, descendants: Memory, total: Memory, processes: usize }

fn sample(root: u32) -> io::Result<Sample> {
    let before = process_tree(root)?;
    let mut owner = Memory::default();
    let mut descendants = Memory::default();
    for (&pid, &stamp) in &before {
        let memory = parse_memory(&fs::read_to_string(format!("/proc/{pid}/smaps_rollup"))?)?;
        if identity(pid)? != stamp { return Err(invalid()); }
        if pid == root { owner = memory; } else { descendants = descendants.add(memory)?; }
    }
    if process_tree(root)? != before { return Err(invalid()); }
    Ok(Sample { owner, descendants, total: owner.add(descendants)?, processes: before.len() })
}

#[derive(Default, Serialize)]
pub struct Summary {
    interval_ms: u64,
    samples: u64,
    incomplete_samples: u64,
    maximum_processes: usize,
    owner_before: Memory,
    owner_after: Memory,
    owner_sampled_peak: Memory,
    descendants_sampled_peak: Memory,
    total_sampled_peak: Memory,
}

impl Summary {
    fn record(&mut self, observation: io::Result<Sample>) {
        let Ok(value) = observation else { self.incomplete_samples += 1; return };
        self.samples += 1;
        self.maximum_processes = self.maximum_processes.max(value.processes);
        self.owner_after = value.owner;
        self.owner_sampled_peak.peak(value.owner);
        self.descendants_sampled_peak.peak(value.descendants);
        self.total_sampled_peak.peak(value.total);
    }
}

pub struct Sampler {
    stop: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<Summary>>,
}

impl Sampler {
    pub fn start() -> io::Result<Self> {
        let root = std::process::id();
        let first = sample(root)?;
        let mut summary = Summary { interval_ms: 50, owner_before: first.owner, ..Summary::default() };
        summary.record(Ok(first));
        let (stop, receive) = mpsc::channel();
        let worker = thread::Builder::new().name("memory-sampler".into()).spawn(move || {
            while let Err(mpsc::RecvTimeoutError::Timeout) = receive.recv_timeout(Duration::from_millis(50)) {
                summary.record(sample(root));
            }
            summary.record(sample(root));
            summary
        })?;
        Ok(Self { stop: Some(stop), worker: Some(worker) })
    }

    pub fn finish(mut self) -> io::Result<Summary> {
        drop(self.stop.take());
        self.worker.take().ok_or_else(invalid)?.join().map_err(|_| invalid())
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observes_allocated_child_and_stops_after_joined_exit() {
        use std::{io::{BufRead, Write}, process::{Child, Command, Stdio}};
        struct Owned(Child);
        impl Drop for Owned {
            fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
        }
        let mut child = Owned(Command::new("/usr/bin/python3")
            .args(["-I", "-B", "-c",
                "import sys; data=bytearray(12*1024*1024); print('ready',flush=True); sys.stdin.readline()"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap());
        let mut ready = String::new();
        std::io::BufReader::new(child.0.stdout.take().unwrap()).read_line(&mut ready).unwrap();
        assert_eq!(ready, "ready\n");
        let sampler = Sampler::start().unwrap();
        let sampled = sample(std::process::id()).unwrap();
        assert!(sampled.processes >= 2);
        assert!(sampled.descendants.pss_kib >= 12 * 1024);
        assert_eq!(sampled.total.pss_kib, sampled.owner.pss_kib + sampled.descendants.pss_kib);
        child.0.stdin.take().unwrap().write_all(b"stop\n").unwrap();
        assert!(child.0.wait().unwrap().success());
        let summary = sampler.finish().unwrap();
        assert!(summary.samples >= 2);
        assert!(summary.maximum_processes >= 2);
        assert_eq!(sample(std::process::id()).unwrap().processes, 1);
    }

    #[test]
    fn rollup_requires_complete_unambiguous_kib_accounting() {
        let text = "Rss: 31 kB\nPss: 23 kB\nPrivate_Clean: 4 kB\nPrivate_Dirty: 12 kB\nPrivate_Hugetlb: 2 kB\n";
        let memory = parse_memory(text).unwrap();
        assert_eq!((memory.rss_kib, memory.pss_kib, memory.private_kib), (31, 23, 18));
        for invalid in [text.replace("Pss: 23 kB\n", ""),
            text.replace("Pss: 23 kB", "Pss: 23 MB"),
            format!("{text}Pss: 23 kB\n"),
            text.replace("Private_Dirty: 12", "Private_Dirty: 18446744073709551615")] {
            assert!(parse_memory(&invalid).is_err());
        }
    }

    #[test]
    fn total_peak_uses_one_sample_and_does_not_sum_unrelated_peaks() {
        let memory = |pss_kib| Memory { pss_kib, ..Memory::default() };
        let mut summary = Summary::default();
        for (owner, child) in [(30, 5), (5, 40)] {
            summary.record(Ok(Sample { owner: memory(owner), descendants: memory(child),
                total: memory(owner + child), processes: 2 }));
        }
        summary.record(Err(invalid()));
        assert_eq!(summary.total_sampled_peak.pss_kib, 45);
        assert_eq!(summary.owner_sampled_peak.pss_kib, 30);
        assert_eq!(summary.descendants_sampled_peak.pss_kib, 40);
        assert_eq!((summary.samples, summary.incomplete_samples), (2, 1));
    }
}

fn identity(pid: u32) -> io::Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let tail = stat.rsplit_once(") ").ok_or_else(invalid)?.1;
    tail.split_whitespace().nth(19).ok_or_else(invalid)?
        .parse().map_err(|_| invalid())
}
