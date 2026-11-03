//! The rings between one thread that writes and one that reads: in order, nothing lost, nothing
//! twice, a frame never cut in two, under load on four threads.

use std::thread;

use columbina_audio::rt::{messages, samples};

const COUNT: u64 = 10_000_000;

#[test]
fn ten_million_messages_and_ten_million_samples_come_through_in_order_on_four_threads() {
    let (mut message_producer, mut message_consumer) = messages::<u64>(1_024);
    let (mut sample_producer, mut sample_consumer) = samples(4_096, 1);

    let writes_messages = thread::spawn(move || {
        for n in 0..COUNT {
            let mut message = n;
            while let Err(back) = message_producer.push(message) {
                message = back;
                thread::yield_now();
            }
        }
    });
    let reads_messages = thread::spawn(move || {
        let mut expected = 0;
        while expected < COUNT {
            if let Some(message) = message_consumer.pop() {
                assert_eq!(message, expected, "message {expected}");
                expected += 1;
            } else {
                thread::yield_now();
            }
        }
        assert_eq!(message_consumer.pop(), None);
    });
    // Samples count up to below 2^24, so that every one is exact in f32; written and read in
    // pieces of lengths that change.
    #[allow(clippy::cast_precision_loss, reason = "below 2^24")]
    let writes_samples = thread::spawn(move || {
        let mut next = 0_u64;
        let mut piece = [0.0_f32; 700];
        let mut size = 1;
        while next < COUNT {
            let size_now = size.min(usize::try_from(COUNT - next).unwrap());
            for (k, sample) in piece[..size_now].iter_mut().enumerate() {
                *sample = (next + k as u64) as f32;
            }
            let written = sample_producer.push(&piece[..size_now]);
            next += written as u64;
            if written == 0 {
                thread::yield_now();
            }
            size = size % 700 + 1;
        }
    });
    #[allow(clippy::cast_precision_loss, reason = "below 2^24")]
    let reads_samples = thread::spawn(move || {
        let mut expected = 0_u64;
        let mut piece = [0.0_f32; 513];
        let mut size = 513;
        while expected < COUNT {
            let read = sample_consumer.pop(&mut piece[..size]);
            for &sample in &piece[..read] {
                assert_eq!(sample, expected as f32, "sample {expected}");
                expected += 1;
            }
            if read == 0 {
                thread::yield_now();
            }
            size = if size == 1 { 513 } else { size - 1 };
        }
        assert_eq!(sample_consumer.available(), 0);
    });
    for thread in [
        writes_messages,
        reads_messages,
        writes_samples,
        reads_samples,
    ] {
        thread.join().unwrap();
    }
}

/// Frames of `channels` samples, each sample of frame `n` being `n` and its channel apart, go
/// through whole whatever the lengths written and read: a frame is never cut in two.
#[allow(clippy::cast_precision_loss, reason = "below 2^24")]
fn frames_come_through_whole(channels: usize, frames: u64) {
    let (mut producer, mut consumer) = samples(1_000, channels);
    assert_eq!(producer.capacity(), 1_024);
    assert_eq!(producer.channels(), channels);
    let sample = move |frame: u64, channel: usize| frame as f32 + channel as f32 / 32.0;
    let writes = thread::spawn(move || {
        let mut piece = vec![0.0_f32; 1_000];
        let mut next = 0_u64;
        let mut size = 1;
        while next < frames {
            // Whole frames and then part of one, which must not go in.
            let take = usize::try_from(frames - next).unwrap().min(size / channels);
            for k in 0..take {
                for channel in 0..channels {
                    piece[k * channels + channel] = sample(next + k as u64, channel);
                }
            }
            let written = producer.push(&piece[..take * channels + size % channels]);
            assert!(written <= take);
            next += written as u64;
            if written == 0 {
                thread::yield_now();
            }
            size = size % 997 + 1;
        }
    });
    let reads = thread::spawn(move || {
        let mut piece = vec![0.0_f32; 1_000];
        let mut expected = 0_u64;
        let mut size = 997;
        while expected < frames {
            let read = consumer.pop(&mut piece[..size]);
            assert!(read * channels <= size);
            for frame in piece[..read * channels].chunks_exact(channels) {
                for (channel, &got) in frame.iter().enumerate() {
                    assert_eq!(
                        got,
                        sample(expected, channel),
                        "frame {expected}, {channel}"
                    );
                }
                expected += 1;
            }
            if read == 0 {
                thread::yield_now();
            }
            size = if size == 1 { 997 } else { size - 1 };
        }
        assert_eq!(consumer.available(), 0);
    });
    writes.join().unwrap();
    reads.join().unwrap();
}

#[test]
fn frames_of_six_channels_are_never_cut_in_two() {
    frames_come_through_whole(6, 300_000);
}

#[test]
fn frames_of_three_five_and_twenty_four_channels_are_never_cut_in_two() {
    for channels in [3, 5, 24] {
        frames_come_through_whole(channels, 50_000);
    }
}

#[test]
fn a_ring_of_six_channels_takes_whole_frames_only() {
    let (mut producer, mut consumer) = samples(1_000, 6);
    assert_eq!(producer.capacity(), 1_024);
    assert_eq!(producer.free(), 1_024);
    // 1024 × 6 + 1 samples offered: 1024 frames go in, and not the sample over.
    let offered = vec![1.0_f32; 6 * 1_024 + 1];
    assert_eq!(producer.push(&offered), 1_024);
    assert_eq!(producer.free(), 0);
    assert_eq!(consumer.available(), 1_024);
    // Room for one frame and a bit: one frame comes out.
    let mut out = [0.0_f32; 10];
    assert_eq!(consumer.pop(&mut out), 1);
    assert_eq!(consumer.available(), 1_023);
    assert_eq!(producer.free(), 1);
    // Five samples are not a frame.
    assert_eq!(producer.push(&[2.0; 5]), 0);
    assert_eq!(producer.push(&[2.0; 11]), 1);
    assert_eq!(consumer.discard(1_000), 1_000);
    assert_eq!(consumer.available(), 24);
    let mut out = [0.0_f32; 5];
    assert_eq!(consumer.pop(&mut out), 0);
    let mut out = vec![0.0_f32; 6 * 24];
    assert_eq!(consumer.pop(&mut out), 24);
    assert_eq!(out[6 * 23..], [2.0; 6]);
}

#[test]
fn a_ring_holds_a_power_of_two() {
    let (producer, consumer) = samples(1_000, 1);
    assert_eq!(producer.capacity(), 1_024);
    assert_eq!(consumer.capacity(), 1_024);
    let (producer, _consumer) = messages::<u8>(0);
    assert_eq!(producer.free(), 1);
}

#[test]
fn a_full_ring_takes_what_fits_and_gives_back_the_message() {
    let (mut producer, mut consumer) = samples(4, 1);
    assert_eq!(producer.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), 4);
    assert_eq!(producer.free(), 0);
    assert_eq!(producer.push(&[7.0]), 0);
    assert_eq!(consumer.discard(1), 1);
    let mut out = [0.0; 8];
    assert_eq!(consumer.pop(&mut out), 3);
    assert_eq!(out[..3], [2.0, 3.0, 4.0]);
    assert_eq!(consumer.pop(&mut out), 0);
    assert_eq!(consumer.discard(10), 0);

    let (mut producer, mut consumer) = messages::<&str>(2);
    assert_eq!(producer.push("a"), Ok(()));
    assert_eq!(producer.push("b"), Ok(()));
    assert_eq!(producer.push("c"), Err("c"));
    assert_eq!(consumer.available(), 2);
    assert_eq!(consumer.pop(), Some("a"));
    assert_eq!(consumer.pop(), Some("b"));
    assert_eq!(consumer.pop(), None);
}

#[test]
fn an_end_knows_when_the_other_is_gone() {
    let (producer, consumer) = samples(8, 2);
    assert!(!producer.is_abandoned());
    drop(consumer);
    assert!(producer.is_abandoned());
    let (producer, consumer) = messages::<u8>(8);
    drop(producer);
    assert!(consumer.is_abandoned());
}
