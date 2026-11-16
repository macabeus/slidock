//! A movie as plan-media and the video planner read it: an MP4 or QuickTime movie's tracks, edits and sample tables
//! (`read_movie`), what a browser needs to know of a video (`probe_video`), and a WebM's tracks and blocks
//! (`read_webm`). Each reads within its caps, so what it holds is a small multiple of the input.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fuzz_support::{Counting, measured};
use slidock_io::{BytesReader, MAX_MOOV_BYTES, probe_video, read_movie, read_webm};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fuzz_target!(|data: &[u8]| {
    let source = BytesReader::new(data);
    let read = measured(|| {
        let movie = read_movie(&source).expect("a byte source reads");
        let facts = probe_video(&source).expect("a byte source reads");
        let webm = read_webm(&source).expect("a byte source reads");
        (movie, facts, webm)
    });
    let input = data.len() as u64;
    assert!(
        read.peak <= 64 * input.min(MAX_MOOV_BYTES) + (4 << 20),
        "reading the movie held {} bytes",
        read.peak
    );
    let (movie, _, _) = read.value;
    if let Some(movie) = movie {
        let _ = movie.duration_ms();
        for track in &movie.tracks {
            let _ = (track.rotation(), track.unturned(), track.seconds());
        }
        assert!(movie.video_tracks().count() + movie.audio_tracks().count() <= movie.tracks.len());
    }
});
