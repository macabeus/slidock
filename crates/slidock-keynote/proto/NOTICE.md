# Notice: the Keynote schema

`keynote.proto` and `registry.tsv` are derived from the Protocol Buffers definitions and the type registry in
[psobot/keynote-parser](https://github.com/psobot/keynote-parser), version set `14.5` (commit
`56a4d3b0d38c999f4273d084acd45415665e2030`). Those definitions were extracted from the Keynote application by that
project's dumper; the format work it builds on is Sean Patrick O'Brien's
[iWorkFileFormat](https://github.com/obriensp/iWorkFileFormat).

The schema is trimmed to the messages this crate reads: a kept message keeps its scalar and enum fields and the
message fields whose type is also kept, `required` is read as optional, and every type is proto2, so presence is
explicit. Each namespace (`TSP`, `KN`, `TSWP`, …) is a wrapper message, so full names stay the originals
(`.KN.SlideArchive`) while the namespaces, which name each other in cycles, share one file. An extension field is a
field of the message it extends, named by its full name with `_` for `.`. `registry.tsv` lists the whole registry, so
types the schema leaves out are still named in reports.

`build.rs` compiles `keynote.proto` with `protox` and generates the `prost` types, the field tables of the wire
decoder and the `Archive` enum from it and `registry.tsv`.

Refreshing the schema for a new Keynote release: run keynote-parser's dumper on macOS with that Keynote installed
(its README, "Adding support for a new version of Keynote"), trim the new definitions to the same messages, and write
them in this layout.

## keynote-parser license

keynote-parser is licensed under the MIT License, as the License section of its README states
([README.md#license](https://github.com/psobot/keynote-parser/blob/56a4d3b0d38c999f4273d084acd45415665e2030/README.md#license)):

```
Copyright 2019-2025 Peter Sobot

Permission is hereby granted, free of charge, to any person obtaining a copy of this software
and associated documentation files (the "Software"), to deal in the Software without restriction,
including without limitation the rights to use, copy, modify, merge, publish, distribute,
sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```
