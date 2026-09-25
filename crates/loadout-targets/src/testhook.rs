//! Crash points for crash-safety tests. With the
//! `test-support` feature, `LOADOUT_TEST_CRASH=<point>` aborts the
//! process when execution reaches `<point>` (`<point>:<n>` at its n-th
//! arrival). Without the feature this does nothing.

/// Aborts here if the test hook asks for it.
#[allow(unused_variables)]
pub fn crash_point(point: &str) {
    #[cfg(feature = "test-support")]
    {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static HITS: AtomicUsize = AtomicUsize::new(0);
        let Ok(want) = std::env::var("LOADOUT_TEST_CRASH") else {
            return;
        };
        let (name, nth) = match want.split_once(':') {
            Some((n, k)) => (n.to_owned(), k.parse::<usize>().unwrap_or(1)),
            None => (want.clone(), 1),
        };
        if name == point && HITS.fetch_add(1, Ordering::SeqCst) + 1 == nth {
            eprintln!("LOADOUT_TEST_CRASH: aborting at {point}");
            std::process::abort();
        }
    }
}

/// With the `test-support` feature, `LOADOUT_TEST_SCHEDULER=<file>`
/// makes `lo schedule` record the scheduler commands it would run in
/// `<file>` instead of touching the real OS scheduler.
pub fn scheduler_log() -> Option<std::path::PathBuf> {
    #[cfg(feature = "test-support")]
    {
        std::env::var_os("LOADOUT_TEST_SCHEDULER").map(Into::into)
    }
    #[cfg(not(feature = "test-support"))]
    {
        None
    }
}
