# LightFormOpen

An open-source Rust replacement for Lightform Creator, the desktop app for Lightform LF1/LF2 projection-mapping projectors. Lightform shut down and its cloud services are offline, but the devices still work over the local network. The goal is to keep them usable.

**Status:** early. The reverse-engineering spec is complete enough to start building. `lf-lit` reads and writes project files. `lf-device` finds and controls projectors, so far only against a mock device.

## What's here

| Path | Contents |
|---|---|
| `docs/SPEC.md` | Architecture of Creator 2.2.12 and the phased port plan. Start here. |
| `docs/findings/` | Detailed findings: `.lit` format, device protocol, engine, UI. Each claim is tagged Observed, Inferred or Unknown. |
| `proto/` | gRPC schema recovered from the app's embedded protobuf descriptors. |
| `tools/re/` | Python tools: `.lit` reader (`litdump.py`), Qt resource extractor, descriptor and RTTI extractors. |
| `data/rtti/` | Class lists and inheritance edges recovered from the app's binaries. |
| `crates/lf-lit` | `.lit` project reader/writer: reads both containers, writes binary, byte-identical round trips. |
| `crates/lf-device` | Device discovery and control over the recovered gRPC protocol. Tested against a mock device; not yet against real hardware. |
| `apps/lf-cli` | Command-line driver, binary name `lf`. |

## Plan

1. Capture a real publish package and one upload from the original app.
2. `lf-device`: discover, connect, control playback, upload packages.
3. `lf-lit`: read and round-trip `.lit` projects.
4. Rendering and publishing: warp, masks, blend modes, ISF effects, video encode.
5. A Slint editor.

Details and open questions are in `docs/SPEC.md`.

## Building

```bash
cargo build
cargo run --bin lf -- lit dump "My Project.lit"
cargo run --bin lf -- lit roundtrip "My Project.lit"
cargo run --bin lf -- device discover
cargo run --bin lf -- device status 192.168.1.50 --trace
```

The tools in `tools/re/` need only Python 3.

## What this repository does not contain

Lightform's own code, UI files, icons, fonts, built-in effects and media are copyrighted and are not distributed here. The findings describe how the app works so it can be reimplemented. To inspect the original files, extract them from your own copy of Lightform Creator with the scripts in `tools/re/`.

This project is not affiliated with or endorsed by Lightform, Inc.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
