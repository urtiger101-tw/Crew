//! Cached native system telemetry for the sidebar.
//!
//! Sampling runs on one worker thread. `refresh` only drains and submits channel
//! messages, so callers can invoke it while holding the application state lock.

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Memory {
    pub used: u64,
    pub total: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiskSpace {
    pub available: u64,
    pub total: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Snapshot {
    pub memory: Option<Memory>,
    pub disks: BTreeMap<String, Option<DiskSpace>>,
}

/// UI-owned snapshot plus the single background sampler.
#[derive(Default)]
pub struct Cache {
    pub snapshot: Snapshot,
    requests: Option<SyncSender<Vec<String>>>,
    results: Option<Receiver<(Snapshot, Instant)>>,
    in_flight: bool,
    sampled_at: Option<Instant>,
    retry_worker_at: Option<Instant>,
}

impl Cache {
    /// Poll completed samples and schedule a refresh when the five-second TTL expires.
    /// Returns true only when the visible snapshot changed.
    pub fn refresh(&mut self, mut directories: Vec<String>) -> bool {
        directories.sort_unstable();
        directories.dedup();

        let previous_snapshot = self.snapshot.clone();
        let mut worker_disconnected = false;
        if let Some(results) = &self.results {
            loop {
                match results.try_recv() {
                    Ok((snapshot, sampled_at)) => {
                        self.in_flight = false;
                        self.sampled_at = Some(sampled_at);
                        self.snapshot = snapshot;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        worker_disconnected = true;
                        break;
                    }
                }
            }
        }

        let now = Instant::now();
        if worker_disconnected {
            self.requests = None;
            self.results = None;
            self.in_flight = false;
            self.sampled_at = Some(now);
            self.retry_worker_at = Some(now + REFRESH_INTERVAL);
            self.snapshot = unavailable_snapshot(&directories);
        }

        if self.requests.is_none() && self.retry_worker_at.is_none_or(|retry_at| now >= retry_at) {
            let (request_tx, request_rx) = sync_channel::<Vec<String>>(1);
            let (result_tx, result_rx) = sync_channel::<(Snapshot, Instant)>(1);
            match thread::Builder::new()
                .name("crew-system-status".to_owned())
                .spawn(move || {
                    while let Ok(directories) = request_rx.recv() {
                        let snapshot = sample_snapshot(&directories);
                        if result_tx.send((snapshot, Instant::now())).is_err() {
                            break;
                        }
                    }
                }) {
                Ok(_) => {
                    self.requests = Some(request_tx);
                    self.results = Some(result_rx);
                    self.retry_worker_at = None;
                }
                Err(_) => {
                    self.sampled_at = Some(now);
                    self.retry_worker_at = Some(now + REFRESH_INTERVAL);
                    self.snapshot = unavailable_snapshot(&directories);
                }
            }
        }

        let ttl_expired = self
            .sampled_at
            .is_none_or(|sampled_at| sampled_at.elapsed() >= REFRESH_INTERVAL);
        if !self.in_flight
            && (self.sampled_at.is_none() || ttl_expired)
            && let Some(requests) = &self.requests
        {
            match requests.try_send(directories.clone()) {
                Ok(()) => self.in_flight = true,
                Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    self.requests = None;
                    self.results = None;
                    self.in_flight = false;
                    self.sampled_at = Some(now);
                    self.retry_worker_at = Some(now + REFRESH_INTERVAL);
                    self.snapshot = unavailable_snapshot(&directories);
                }
            }
        }

        self.snapshot.disks = directories
            .iter()
            .map(|directory| {
                (
                    directory.clone(),
                    self.snapshot.disks.get(directory).cloned().unwrap_or(None),
                )
            })
            .collect();
        self.snapshot != previous_snapshot
    }
}

fn unavailable_snapshot(directories: &[String]) -> Snapshot {
    Snapshot {
        memory: None,
        disks: directories
            .iter()
            .cloned()
            .map(|directory| (directory, None))
            .collect(),
    }
}

fn sample_snapshot(directories: &[String]) -> Snapshot {
    Snapshot {
        memory: sample_memory(),
        disks: directories
            .iter()
            .map(|directory| (directory.clone(), sample_disk(directory)))
            .collect(),
    }
}

#[cfg(windows)]
fn sample_memory() -> Option<Memory> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: status is a writable MEMORYSTATUSEX with the required structure size.
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return None;
    }

    Some(Memory {
        used: status.ullTotalPhys.saturating_sub(status.ullAvailPhys),
        total: status.ullTotalPhys,
    })
}

#[cfg(target_os = "linux")]
fn sample_memory() -> Option<Memory> {
    linux_memory_from_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn sample_memory() -> Option<Memory> {
    None
}

#[cfg(target_os = "linux")]
fn linux_memory_from_meminfo(contents: &str) -> Option<Memory> {
    fn kibibytes(contents: &str, key: &str) -> Option<u64> {
        contents.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name != key {
                return None;
            }
            let mut fields = value.split_whitespace();
            let amount = fields.next()?.parse::<u64>().ok()?;
            (fields.next().is_none_or(|unit| unit == "kB")).then_some(amount)
        })
    }

    let total = kibibytes(contents, "MemTotal")?.checked_mul(1024)?;
    let available = kibibytes(contents, "MemAvailable")
        .or_else(|| {
            ["MemFree", "Buffers", "Cached"]
                .into_iter()
                .try_fold(0u64, |sum, key| sum.checked_add(kibibytes(contents, key)?))
        })?
        .checked_mul(1024)?
        .min(total);

    Some(Memory {
        used: total - available,
        total,
    })
}

#[cfg(windows)]
fn sample_disk(directory: &str) -> Option<DiskSpace> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let mut path = std::ffi::OsStr::new(directory)
        .encode_wide()
        .collect::<Vec<_>>();
    if path.contains(&0) {
        return None;
    }
    path.push(0);

    let mut available = 0u64;
    let mut total = 0u64;
    // SAFETY: path is NUL-terminated UTF-16; both output pointers are writable u64s.
    // The unused free-bytes output is documented as optional.
    if unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut available,
            &mut total,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return None;
    }

    Some(DiskSpace { available, total })
}

#[cfg(unix)]
// statvfs field widths and signedness differ between supported Unix platforms.
#[allow(clippy::useless_conversion)]
fn sample_disk(directory: &str) -> Option<DiskSpace> {
    use std::os::unix::ffi::OsStrExt;

    let path = std::ffi::CString::new(std::ffi::OsStr::new(directory).as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: path is a valid C string and stats points to a full writable statvfs.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: successful statvfs initialized the structure before this read.
    let stats = unsafe { stats.assume_init() };
    let fragment_size = u64::try_from(stats.f_frsize).ok()?;
    let available = u64::try_from(stats.f_bavail)
        .ok()?
        .checked_mul(fragment_size)?;
    let total = u64::try_from(stats.f_blocks)
        .ok()?
        .checked_mul(fragment_size)?;

    Some(DiskSpace {
        available: available.min(total),
        total,
    })
}

#[cfg(not(any(windows, unix)))]
fn sample_disk(_: &str) -> Option<DiskSpace> {
    None
}

#[cfg(test)]
mod tests {
    use super::Cache;
    #[cfg(target_os = "linux")]
    use super::{Memory, linux_memory_from_meminfo};
    use std::time::{Duration, Instant};

    #[test]
    fn default_cache_does_not_start_a_worker() {
        let cache = Cache::default();
        assert!(cache.requests.is_none());
        assert!(cache.results.is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_linux_available_memory_in_bytes() {
        let sample = "MemTotal: 1000 kB\nMemFree: 100 kB\nMemAvailable: 250 kB\n";
        assert_eq!(
            linux_memory_from_meminfo(sample),
            Some(Memory {
                used: 750 * 1024,
                total: 1000 * 1024,
            })
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_incomplete_or_overflowing_memory_samples() {
        assert_eq!(linux_memory_from_meminfo("MemAvailable: 10 kB\n"), None);
        assert_eq!(
            linux_memory_from_meminfo("MemTotal: 18446744073709551615 kB\nMemAvailable: 1 kB\n"),
            None
        );
    }

    #[test]
    fn refresh_samples_a_unicode_directory_on_the_worker() {
        let existing = std::env::current_dir()
            .expect("current directory")
            .to_string_lossy()
            .into_owned();
        let missing = std::env::current_dir()
            .expect("current directory")
            .join("不存在的 Crew 目錄")
            .to_string_lossy()
            .into_owned();
        let mut cache = Cache::default();
        let started = Instant::now();

        while started.elapsed() < Duration::from_secs(2) {
            cache.refresh(vec![existing.clone(), missing.clone()]);
            if cache.sampled_at.is_some() && !cache.in_flight {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }

        assert!(
            cache.snapshot.disks[&existing]
                .as_ref()
                .is_some_and(|disk| disk.total > 0)
        );
        assert_eq!(cache.snapshot.disks.get(&missing), Some(&None));
    }
}
