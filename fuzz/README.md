# Fuzz targets

cargo-fuzz targets for the readers that take untrusted input, each driving the entry points the frontends call:

| Target                                        | Reads                                                                                                                                                                                                                        | Seeds                                                                               |
| --------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `zip`                                         | `slidock_io::ZipArchive` over a `BytesReader`: the central directory, each entry looked up by name, opened as a range reader, read into memory, then all read at once (`read_many`)                                          | `seeds/zip/`, `../probes/out/{fonts-probe.pptx,motion-probe.pptx,motion-probe.odp}` |
| `xml`                                         | `decode_xml` and `parse_xml` under the default limits, twice against one small presentation budget, then the `slidock_io::xml` helpers over the tree                                                                         | `seeds/xml/`                                                                        |
| `iwa`                                         | Keynote's index: the chunk framing and raw Snappy blocks (`unframe_iwa`), then the object stream through `read_object_graph`; every decoded object dumps                                                                     | `seeds/iwa/`                                                                        |
| `proto`                                       | the protobuf pre-pass on every type the trimmed schema keeps: the first two bytes pick the type, the rest is one payload, read through `read_iwa`; a decoded object dumps, and its canonical bytes decode to the same object | `seeds/proto/`                                                                      |
| `sniff`, `eot`, `woff`, `woff2`, `mtx`, `ctf` | `slidock_fonts::container`: sniffing and each decoder (`ctf` is MicroType Express without its LZCOMP layer)                                                                                                                  | the container fixtures, `seeds/ctf/`                                                |
| `unwrap`                                      | every font decoder behind the container entry point, then the sfnt reader; the first byte picks the hint                                                                                                                     | the container fixtures                                                              |
| `tiff`, `bmp`                                 | `decode_tiff` and `decode_bmp` within a pixel cap, then `encode_png` (and `transcode_to_png`), whose header gives back the decoded size                                                                                      | `seeds/media/`                                                                      |
| `svg`                                         | `check_svg`, `check_svg_source` from a range source, and `picture_svg`, whose output passes the check with the same size                                                                                                     | `seeds/media/`                                                                      |
| `image_size`                                  | `sniff_mime`, `image_size` and `read_image_size` for every type, `jpeg_orientation`, `icc_colour`                                                                                                                            | `seeds/media/`                                                                      |
| `heif`                                        | `read_heif` (the decode a host is asked for) and `sps_video_signal` on its HEVC configurations                                                                                                                               | `seeds/media/`                                                                      |
| `movie`                                       | `read_movie`, `probe_video` and `read_webm`                                                                                                                                                                                  | `seeds/media/`                                                                      |
| `compile`                                     | the whole compile, `slidock::compile`: sniff, lower, every pass, print and bundle into a discarded zip, under limits scaled to the inputs                                                                                    | `seeds/compile/`                                                                    |

`iwa` and `proto` decode within a budget scaled to their inputs (`iwa` 2 MiB and 20,000 messages, `proto` 1 MiB and
20,000), so the inputs reach it, and measure what the decode holds with a counting allocator: at most 4 times the
memory the stats count once it returns, and at most 8 times that plus 48 times the stream's bytes at its peak (for
a file that fails to unframe, the output its chunks declare, allocated within the budget before one fails). Bytes
that amplify into memory the budget does not count (an object header of many empty `MessageInfo`s, a packed run of
numbers) fail there, well before `-rss_limit_mb`.

The media targets and `compile` measure memory the same way. `tiff` holds at most the RGBA image and one strip of
samples (12 bytes a pixel), never the samples whole beside the image; `bmp` decodes straight into RGBA; the header
readers hold a small multiple of their input whatever size a header claims. `compile` runs under a twelfth of the
default limits (50 slides, 300,000 XML nodes, an output budget of 2,000 elements, 80,000 path segments, 8,000 runs and
40,000 gradient stops, 1 million pixels a transcode) and fails any input whose compile holds more than 96 MiB plus 1,024
times its size: a slide listed many times, presets and text that expand past what the budgets count, or rasters decoded
whole fail there, as the 61 KB deck that reached 26 GB and the 46 KB of TIFFs that reached 4 GB would have.

This directory is a workspace of its own (the targets, `seedgen`, and `support`, the allocator and checks the
targets share) and needs a nightly toolchain with cargo-fuzz (`nightly-2026-10-25` is known to work). `support`
tests on the stable toolchain: `cargo test --manifest-path fuzz/Cargo.toml -p slidock-fuzz-support`.

## Smoke run

```sh
cargo xtask fuzz-smoke                       # every target for 60 s, one at a time
cargo xtask fuzz-smoke --seconds 1200 --only zip,xml --jobs 2
```

It builds every target, seeds `corpus/<target>/` (gitignored) from the seeds in the table (a copy whose seed changed is
refreshed), and runs each for `--seconds` (at least 1: libFuzzer reads 0 as no limit) with
`-rss_limit_mb=2048 -timeout=10` and the target's `-max_len` (zip and compile 512 KiB, xml, iwa and the media targets
64 KiB, proto 16 KiB). Any
crash, timeout or out-of-memory fails it, with the input in `artifacts/<target>/`; it prints each target's
executions, coverage, features and corpus size. `SLIDOCK_FUZZ_TOOLCHAIN` picks another nightly.

## Seeds

`seeds/{zip,xml,iwa,proto,media,compile}/` are generated by `seedgen` from our own probe decks and the Keynote schema:
an XML part of each kind in the probes, small zips (stored, deflated, ZIP64, a comment holding a signature, a
Keynote-like package), a payload of every kept type with each of its fields set, IWA files of those objects in
compressed, literal and small chunks, every media file of the probes up to 16 KiB with TIFFs, BMPs and a WebM of the
forms the decoders read (the probes hold none), and the small probe decks whole with one that lists its slide 40
times. Regenerate them (deterministically) with:

```sh
cargo run --manifest-path fuzz/Cargo.toml -p slidock-fuzz-seedgen
```

The private corpus may seed a local corpus, which is never committed:

```sh
cargo run --manifest-path fuzz/Cargo.toml -p slidock-fuzz-seedgen -- private "$PWD/fuzz/corpus-private"
cargo xtask fuzz-smoke --extra "$PWD/fuzz/corpus-private"
```

That writes the small decks whole (`zip`), their XML parts (`xml`), Keynote index files (`iwa`) and payloads from
them (`proto`) into the gitignored `corpus-private/<target>/`, which libFuzzer reads but does not write. A run with
`--extra` writes the inputs it finds into the gitignored `corpus-extra/<target>/` and only reads `corpus/<target>/`,
so `corpus/` never holds data derived from the private corpus; `corpus-extra/` does, and is never committed or
shared. A finding from either becomes a synthetic regression test in the crate that owns the code, never a copy of
corpus bytes.

## Running one target

```sh
cd fuzz
cargo +nightly-2026-10-25 fuzz run zip corpus/zip -- -max_total_time=300 -rss_limit_mb=2048 -timeout=10 -max_len=524288
```

The font targets' seeds by hand: `woff2` from the `*.woff2` fixtures, `eot` from `*.eot`, `woff` from `*.woff`, and
`sniff` from any fixtures:

```sh
mkdir -p corpus/woff2 && cp ../crates/slidock-fonts/tests/data/container/*.woff2 corpus/woff2/
```

An `mtx` input is an EOT's font data, so its seeds are the tail of each `*-mtx.eot` (`FontDataSize` bytes before
`EOTSize`):

```sh
mkdir -p corpus/mtx && for f in ../crates/slidock-fonts/tests/data/container/*-mtx.eot; do
  python3 -I -c 'import sys, struct; d = open(sys.argv[1], "rb").read(); n, m = struct.unpack("<II", d[:8]); sys.stdout.buffer.write(d[n - m:n])' "$f" > "corpus/mtx/$(basename "$f" .eot)"
done
```

A `ctf` input is the three blocks `mtx::decompress` returns for an `mtx` seed, the first two each after its 3-byte
big-endian length. `seeds/ctf/` holds framed compact fonts with `VDMX` and `hdmx`, which the fixtures lack. An
`unwrap` input starts with one byte that picks the hint (even: none, odd: a GUID-named `.odttf` part), so its seeds
are the fixtures with a zero byte in front. `cargo +nightly-2026-10-25 fuzz run <target> <file>` replays one input.
