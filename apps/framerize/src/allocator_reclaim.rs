//! Return free glibc pages under host pressure without changing allocation policy.
use std::time::{Duration, Instant};
use framer_core::HOST_RESERVE_BYTES;

pub const FLAG: &str = "FRAMERIZE_RUST_PRESSURE_RECLAIM";
pub const SUPPORTED: bool = cfg!(all(target_os = "linux", target_env = "gnu"));
const MIB: u64 = 1024 * 1024;
const MIN_FREE: u64 = 64 * MIB;
const MIN_GROWTH: u64 = 16 * MIB;
const COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Clone, Copy)]
pub struct Snapshot {
    pub total: u64,
    pub available: u64,
    pub rss: u64,
    pub free: u64,
}

#[derive(Default)]
pub struct Reclaimer {
    last: Option<(Instant, u64)>,
}

pub struct Collection {
    pub before_rss: u64,
    pub after_rss: u64,
    pub free_heap: u64,
    pub elapsed: Duration,
    pub allocator_reported_release: bool,
}

impl Reclaimer {
    pub fn poll(&mut self) -> Option<Collection> {
        let snapshot = current_snapshot()?;
        self.run_with(snapshot, Instant::now(), || {
            let released = trim_heap().unwrap_or(false);
            (released, resident_bytes())
        })
    }

    pub fn run_with<F>(&mut self, s: Snapshot, now: Instant, trim: F) -> Option<Collection>
    where F: FnOnce() -> (bool, Option<u64>) {
        if !self.eligible(s, now) { return None; }
        let started = Instant::now();
        let (released, after) = trim();
        let after = after.unwrap_or(s.rss);
        self.last = Some((now, after));
        Some(Collection {
            before_rss: s.rss, after_rss: after, free_heap: s.free,
            elapsed: started.elapsed(), allocator_reported_release: released,
        })
    }

    pub fn eligible(&self, s: Snapshot, now: Instant) -> bool {
        if s.total == 0 || s.available >= HOST_RESERVE_BYTES
            || s.rss < MIN_FREE || s.free < MIN_FREE {
            return false;
        }
        if let Some((at, rss)) = self.last {
            if now.saturating_duration_since(at) < COOLDOWN
                || s.rss.saturating_sub(rss) < MIN_GROWTH {
                return false;
            }
        }
        true
    }
}

fn kib_field(text: &str, wanted: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name != wanted { return None; }
        let mut words = value.split_whitespace();
        let count = words.next()?.parse::<u64>().ok()?;
        if words.next()? != "kB" { return None; }
        count.checked_mul(1024)
    })
}

pub fn resident_bytes() -> Option<u64> {
    kib_field(&std::fs::read_to_string("/proc/self/status").ok()?, "VmRSS")
}

pub fn current_snapshot() -> Option<Snapshot> {
    let memory = std::fs::read_to_string("/proc/meminfo").ok()?;
    Some(Snapshot {
        total: kib_field(&memory, "MemTotal")?,
        available: kib_field(&memory, "MemAvailable")?,
        rss: resident_bytes()?,
        free: free_heap_bytes()?,
    })
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod native {
    #[repr(C)]
    struct MallInfo {
        arena: usize, ordblks: usize, smblks: usize, hblks: usize, hblkhd: usize,
        usmblks: usize, fsmblks: usize, used: usize, free: usize, keepcost: usize,
    }
    unsafe extern "C" {
        fn mallinfo2() -> MallInfo;
        fn malloc_trim(pad: usize) -> i32;
    }
    pub fn free_bytes() -> u64 {
        // Read counters only. No allocation contents or credentials are read.
        unsafe { mallinfo2().free as u64 }
    }
    pub fn trim() -> bool {
        // glibc synchronizes allocator access and retains every live allocation.
        unsafe { malloc_trim(0) != 0 }
    }
}

pub fn free_heap_bytes() -> Option<u64> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    { Some(native::free_bytes()) }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    { None }
}

pub fn trim_heap() -> Option<bool> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    { Some(native::trim()) }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pressure() -> Snapshot {
        Snapshot { total: 16 * 1024 * MIB, available: 32 * MIB,
            rss: 256 * MIB, free: 160 * MIB }
    }
    #[test]
    fn no_operation_with_host_reserve_intact() {
        let mut r = Reclaimer::default();
        let s = Snapshot { available: HOST_RESERVE_BYTES, ..pressure() };
        assert!(r.run_with(s, Instant::now(), || panic!("must not trim")).is_none());
    }
    #[test]
    fn no_operation_without_substantial_free_heap() {
        let mut r = Reclaimer::default();
        let s = Snapshot { free: MIN_FREE - 1, ..pressure() };
        assert!(r.run_with(s, Instant::now(), || panic!("must not trim")).is_none());
    }
    #[test]
    fn cooldown_and_new_resident_growth_are_both_required() {
        let mut r = Reclaimer::default();
        let now = Instant::now();
        assert!(r.run_with(pressure(), now, || (true, Some(128 * MIB))).is_some());
        assert!(!r.eligible(pressure(), now + Duration::from_secs(59)));
        let s = Snapshot { rss: 128 * MIB + MIN_GROWTH - 1, ..pressure() };
        assert!(!r.eligible(s, now + COOLDOWN));
        assert!(r.eligible(Snapshot { rss: s.rss + 1, ..s }, now + COOLDOWN));
    }
    #[test]
    fn no_op_attempt_is_also_rate_limited() {
        let mut r = Reclaimer::default();
        let now = Instant::now();
        let result = r.run_with(pressure(), now, || (false, None)).unwrap();
        assert!(!result.allocator_reported_release);
        assert_eq!(result.after_rss, pressure().rss);
        assert!(!r.eligible(pressure(), now + Duration::from_secs(600)));
    }
    #[test]
    fn unknown_or_small_residency_is_not_trimmed() {
        let r = Reclaimer::default();
        let now = Instant::now();
        assert!(!r.eligible(Snapshot { total: 0, ..pressure() }, now));
        assert!(!r.eligible(Snapshot { rss: MIN_FREE - 1, ..pressure() }, now));
    }
    #[test]
    fn pressure_boundary_preserves_fixed_host_reserve() {
        let r = Reclaimer::default();
        let s = pressure();
        let now = Instant::now();
        for total in [8 * 1024 * MIB, 32 * 1024 * MIB] {
            assert!(!r.eligible(Snapshot { total, available: HOST_RESERVE_BYTES, ..s }, now));
            assert!(r.eligible(Snapshot { total, available: HOST_RESERVE_BYTES - 1, ..s }, now));
        }
    }
    #[test]
    fn kernel_counters_require_named_valid_kib_values() {
        assert_eq!(kib_field("Other: 999 kB\nMemTotal: 12 kB\n", "MemTotal"), Some(12288));
        for text in ["MemTotal: bad kB", "MemTotal: 10 MB", "MemTotal: 18446744073709551615 kB"] {
            assert_eq!(kib_field(text, "MemTotal"), None);
        }
        assert_eq!(kib_field("MemFree: 123 kB", "MemAvailable"), None);
    }
}
