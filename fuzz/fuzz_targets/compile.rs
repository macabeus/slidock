//! A whole deck from untrusted bytes: sniffed (PPTX, ODP or Keynote), lowered, every pass, printed and bundled into
//! a zip that is thrown away, under limits scaled to the inputs. What the compile holds at its peak must stay within
//! a fixed multiple of the input: a source that amplifies into memory past what the limits count (a slide listed
//! many times, presets and text that expand without a budget, rasters decoded whole) fails here, as the 61 KB
//! deck that reached 26 GB and the 46 KB of TIFFs that reached 4 GB would have.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock::{
    BundleError, BundleOptions, BundleSink, BytesReader, Frontends, Input, Limits, MediaPolicy,
    Options, Output,
};
use slidock_fuzz_support::{Counting, measured};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// A sink that keeps nothing.
struct Discard(u64);

impl BundleSink for Discard {
    fn write(&mut self, _path: &str, bytes: &[u8]) -> Result<(), BundleError> {
        self.0 += bytes.len() as u64;
        Ok(())
    }
}

/// About a twelfth of the defaults, so an input of `-max_len` (512 KiB) reaches them.
fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.slides = 50;
    limits.xml.presentation_nodes = 300_000;
    limits.output = Output {
        elements: 2_000,
        path_segments: 80_000,
        text_runs: 8_000,
        gradient_stops: 40_000,
    };
    limits
}

fn policy() -> MediaPolicy {
    MediaPolicy {
        transcode_pixels: 1 << 20,
        presentation_transcode_pixels: 4 << 20,
        host_decoded_pixels: 1 << 20,
        ..MediaPolicy::default()
    }
}

fuzz_target!(|data: &[u8]| {
    let reader = BytesReader::new(data);
    let input = Input::new("input.pptx", &reader);
    let options = Options {
        limits: limits(),
        policy: policy(),
        native_motion: true,
        ..Options::default()
    };
    let mut sink = Discard(0);
    let compiled = measured(|| {
        slidock::compile(
            &input,
            Frontends::default(),
            &options,
            &BundleOptions::default(),
            &mut sink,
        )
        .is_ok()
    });
    let bound = (96 << 20) + 1024 * data.len() as u64;
    assert!(
        compiled.peak <= bound,
        "a {}-byte input held {} bytes at its peak (bound {bound})",
        data.len(),
        compiled.peak
    );
});
