//! The seqlock: a reader never sees a value half written, and sees the writes in order.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use columbina_audio::rt::{seqlock, Words};

#[test]
fn readers_never_see_a_value_half_written() {
    const WRITES: u32 = 2_000_000;
    let (mut writer, reader) = seqlock([0_u32; 16]);
    let done = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..3)
        .map(|_| {
            let reader = reader.clone();
            let done = Arc::clone(&done);
            thread::spawn(move || {
                let mut last = 0;
                let mut reads = 0_u64;
                let mut missed = 0_u64;
                while !done.load(Ordering::Relaxed) {
                    if let Some(value) = reader.try_read() {
                        assert!(value.iter().all(|&word| word == value[0]), "{value:?}");
                        assert!(value[0] >= last, "{} after {last}", value[0]);
                        last = value[0];
                        reads += 1;
                    } else {
                        missed += 1;
                    }
                    let value = reader.read();
                    assert!(value.iter().all(|&word| word == value[0]), "{value:?}");
                    assert!(value[0] >= last);
                    last = value[0];
                }
                (reads, missed)
            })
        })
        .collect();
    for n in 1..=WRITES {
        writer.write([n; 16]);
    }
    done.store(true, Ordering::Relaxed);
    for reader in readers {
        let (reads, _missed) = reader.join().unwrap();
        assert!(reads > 0);
    }
    assert_eq!(reader.read(), [WRITES; 16]);
}

#[test]
fn a_reader_reads_the_last_value_written() {
    let (mut writer, reader) = seqlock((1.5_f64, -3_i64, true));
    assert_eq!(reader.read(), (1.5, -3, true));
    writer.write((f64::NEG_INFINITY, i64::MIN, false));
    assert_eq!(
        reader.try_read(),
        Some((f64::NEG_INFINITY, i64::MIN, false))
    );
    let another = writer.reader();
    writer.write((0.25, 7, true));
    assert_eq!(another.read(), (0.25, 7, true));
}

#[test]
fn values_are_kept_word_by_word() {
    assert_eq!(<[f32; 24]>::WORDS, 24);
    assert_eq!(<(u64, [f32; 2], u32)>::WORDS, 5);
    let value = (u64::MAX - 5, [-0.0_f32, f32::MIN_POSITIVE], 9_u32);
    let words: Vec<u32> = (0..5).map(|index| value.word(index)).collect();
    let back = <(u64, [f32; 2], u32)>::from_words(|index| words[index]);
    assert_eq!(back.0, value.0);
    assert_eq!(back.1[0].to_bits(), value.1[0].to_bits());
    assert_eq!(back.1[1], value.1[1]);
    assert_eq!(back.2, 9);
    assert_eq!(i32::from_words(|_| (-7_i32).word(0)), -7);
}
