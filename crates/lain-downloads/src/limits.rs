//! The byte budget (Go `limits.go`, `free_*.go`). Both bounds are
//! settings, never constants. The type is `lain_api::downloads::Limits`;
//! the checks are free functions over it.

use std::path::Path;

use crate::{CODE_DISK_FULL, CODE_QUOTA, Error, Limits};

/// A free-space probe: bytes available to an unprivileged writer on the
/// filesystem holding a directory, `None` when unknown.
pub type FreeSpace = fn(&Path) -> Option<i64>;

/// The OS probe: `statvfs` on Unix; unknown elsewhere, where the floor
/// is skipped and only the byte budget applies.
pub fn disk_free(dir: &Path) -> Option<i64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: `c` is a valid NUL-terminated path and `st` a properly
        // sized out-parameter that statvfs fully writes on success.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)]
        Some((st.f_bavail as u64).saturating_mul(st.f_frsize as u64).min(i64::MAX as u64) as i64)
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

/// Whether a store already accounting for `used` bytes may grow by
/// `grow` more in `dir`: a typed quota-exceeded or disk-full error that
/// names the numbers, never a bare refusal.
pub fn check(l: &Limits, dir: &Path, used: i64, grow: i64) -> Result<(), Error> {
    check_with(l, dir, used, grow, disk_free)
}

pub fn check_with(l: &Limits, dir: &Path, used: i64, grow: i64, free: FreeSpace) -> Result<(), Error> {
    if grow <= 0 {
        return Ok(());
    }
    if l.max_bytes > 0 && used.saturating_add(grow) > l.max_bytes {
        return Err(Error::new(
            CODE_QUOTA,
            format!(
                "needs {}, {} of {} budget left",
                human_bytes(grow),
                human_bytes((l.max_bytes - used).max(0)),
                human_bytes(l.max_bytes)
            ),
        ));
    }
    if l.min_free_bytes > 0
        && let Some(f) = free(dir)
        && f - grow < l.min_free_bytes
    {
        return Err(Error::new(
            CODE_DISK_FULL,
            format!(
                "needs {}, {} free and {} must stay free",
                human_bytes(grow),
                human_bytes(f),
                human_bytes(l.min_free_bytes)
            ),
        ));
    }
    Ok(())
}

/// Whether a store already at `used` bytes has no room left at all: the
/// check a queue runs before accepting new work.
pub fn full(l: &Limits, dir: &Path, used: i64) -> Result<(), Error> {
    full_with(l, dir, used, disk_free)
}

pub fn full_with(l: &Limits, dir: &Path, used: i64, free: FreeSpace) -> Result<(), Error> {
    if l.max_bytes > 0 && used >= l.max_bytes {
        return Err(Error::new(
            CODE_QUOTA,
            format!("download budget used up: {} of {}", human_bytes(used), human_bytes(l.max_bytes)),
        ));
    }
    if l.min_free_bytes > 0
        && let Some(f) = free(dir)
        && f <= l.min_free_bytes
    {
        return Err(Error::new(
            CODE_DISK_FULL,
            format!("only {} free and {} must stay free", human_bytes(f), human_bytes(l.min_free_bytes)),
        ));
    }
    Ok(())
}

/// A byte count in binary units ("1.5 GiB").
pub fn human_bytes(n: i64) -> String {
    const UNIT: i64 = 1024;
    if n < UNIT {
        return format!("{n} B");
    }
    let (mut div, mut exp) = (UNIT, 0usize);
    let mut m = n / UNIT;
    while m >= UNIT && exp < 5 {
        div *= UNIT;
        exp += 1;
        m /= UNIT;
    }
    format!("{:.1} {}iB", n as f64 / div as f64, &"KMGTPE"[exp..exp + 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thousand(_: &Path) -> Option<i64> {
        Some(1000)
    }
    fn unknown(_: &Path) -> Option<i64> {
        None
    }

    #[test]
    fn limits_check() {
        let x = Path::new("/x");
        let lim = |max, min| Limits { max_bytes: max, min_free_bytes: min };
        assert!(check_with(&lim(0, 0), x, 1 << 40, 1 << 40, thousand).is_ok(), "zero limits are unlimited");
        assert!(check_with(&lim(100, 0), x, 60, 40, thousand).is_ok(), "exactly at the cap");
        assert_eq!(check_with(&lim(100, 0), x, 60, 41, thousand).unwrap_err().code, CODE_QUOTA);
        assert_eq!(check_with(&lim(0, 900), x, 0, 101, thousand).unwrap_err().code, CODE_DISK_FULL);
        assert!(check_with(&lim(0, 900), x, 0, 100, thousand).is_ok(), "at the floor");
        assert!(check_with(&lim(0, 900), x, 0, 1 << 30, unknown).is_ok(), "unknown free size never blocks");
    }

    #[test]
    fn limits_full() {
        let x = Path::new("/x");
        let lim = |max, min| Limits { max_bytes: max, min_free_bytes: min };
        assert!(full_with(&lim(100, 0), x, 99, thousand).is_ok());
        let e = full_with(&lim(100, 0), x, 100, thousand).unwrap_err();
        assert!(e.code == CODE_QUOTA && e.to_string().contains("used up"), "{e}");
        assert_eq!(full_with(&lim(0, 1000), x, 0, thousand).unwrap_err().code, CODE_DISK_FULL);
        assert!(full_with(&lim(0, 0), x, 1 << 50, thousand).is_ok());
    }

    #[test]
    fn human() {
        for (n, want) in [(0, "0 B"), (1023, "1023 B"), (1024, "1.0 KiB"), (3 << 30, "3.0 GiB"), (1536, "1.5 KiB")] {
            assert_eq!(human_bytes(n), want);
        }
    }
}
