//! The memory that the reader takes is bounded by the length of the file, whatever sizes the
//! file declares: counted by an allocator of this test's own, around each call. One test in
//! this file, so that no other thread allocates while it counts.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};

use columbina_lut::parse_cube;
use proptest::prelude::*;

struct Counting;

static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

#[allow(unsafe_code)]
// SAFETY: every call is passed on to the system's allocator as it came; the counters are only
// counters.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now = NOW.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
        PEAK.fetch_max(now, Ordering::SeqCst);
        // SAFETY: as the caller's contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        NOW.fetch_sub(layout.size(), Ordering::SeqCst);
        // SAFETY: as the caller's contract.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The most bytes that reading `bytes` takes at any moment, beyond what was taken before.
fn peak_of_reading(bytes: &[u8]) -> usize {
    let before = NOW.load(Ordering::SeqCst);
    PEAK.store(before, Ordering::SeqCst);
    let read = parse_cube(bytes);
    let peak = PEAK.load(Ordering::SeqCst) - before;
    drop(read);
    peak
}

fn header() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("LUT_3D_SIZE 256\n"),
        Just("LUT_1D_SIZE 65536\n"),
        Just("LUT_1D_SIZE 65536\nLUT_3D_SIZE 256\n"),
        Just("LUT_3D_SIZE 2\n"),
    ]
    .prop_map(str::to_owned)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn reading_takes_memory_in_proportion_to_the_file(
        header in header(),
        rows in 0_usize..200,
        long in prop::option::of("[0-9 ]{0,3000}"),
        title in "[a-z]{0,200}",
    ) {
        let mut file = format!("TITLE \"{title}\"\n{header}");
        for i in 0..rows {
            let _ = writeln!(file, "{i} 0.5 1e-3");
        }
        if let Some(long) = long {
            file.push_str(&long);
        }
        let peak = peak_of_reading(file.as_bytes());
        // A row of 6 bytes or more is 12 in a table, which may grow to twice what it holds;
        // the rest is a few small vectors and the title.
        let bound = 8 * file.len() + 4096;
        prop_assert!(peak <= bound, "{peak} bytes for a file of {}", file.len());
    }
}
