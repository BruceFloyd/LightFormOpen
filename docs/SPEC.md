# Lightform Creator 2.2.12 — Architecture review and Rust port plan

Date: 2026-10-18. Every finding here comes from static analysis of the shipped app (`Lightform.app`, Intel-only, Qt 6.2) and your sample projects. Nothing has been run, and no device has been contacted yet.

Detailed findings (in `docs/findings/`), with each claim tagged Observed (O), Inferred (I) or Unknown (U):

| Topic | File |
|---|---|
| `.lit` project format + Python reader | `findings/lit-format.md`, `tools/re/litdump.py` |
| Device / cloud protocol, recovered gRPC schema | `findings/device-protocol.md`, `proto/MetaRPC.proto` |
| Engine: object model, renderer, effects, media, libraries | `findings/engine.md` |
| UI screens, features, backend API contract, Slint assessment | `findings/ui-and-features.md` |
| Extracted QML UI, icons, cursors | Not distributed (Lightform's copyrighted work). Regenerate from your own install with `tools/re/qrc_extract.py`; findings cite these paths as `re/qrc/...` |

## 1. What the app is

Creator is an authoring and control tool for Lightform LF1/LF2 projectors. The **device does the playback**. Creator builds the projection-mapped content, renders each slide to video or a still, uploads a zip, and remote-controls playback.

```
 ┌──────────── Creator (Mac) ─────────────┐            ┌──── LF1 / LF2 ────┐
 │ QML UI (Qt Quick, ~37k lines)          │  UDP 7350/7351 discovery          │
 │ Engine: AnyObject doc model + undo     │◄─────────────────────────────────►│
 │ Renderer: OpenGL 3.3 render graph      │  gRPC MetaRPC/Talk :50050         │
 │  warp = Bézier patch grid, contel = 3×3│  (control, state push, scan, peek)│
 │  homography, NanoVG masks, ISF effects │  HTTP :7346 POST playback/upload  │
 │ Media: FFmpeg 4.2.1 + VideoToolbox     │  HTTP :7343/7344/7345 frames/scans│
 │ Publish: slides → mp4/png + manifest   │  HTTP :7341 legacy wifi/OTA       │
 └────────────────────────────────────────┘            └───────────────────┘
```

## 2. Subsystems at a glance

| Subsystem | How it works today | Port verdict |
|---|---|---|
| Discovery | Custom JSON over UDP: query to 7350 every 15 s, device replies on 7351 (O) | Easy (`tokio` UDP) |
| Device control | One bidirectional gRPC stream `metarpc.MetaRPC/Talk`, no TLS, no auth. 71 commands, 13 state pushes. Schema recovered to `.proto` (O) | Easy–Med (`tonic` + `prost` straight from the recovered proto) |
| Publish/upload | Per-slide `content.mp4`/`content.png`, `thumb.jpg`, masks, optional live ISF shader, `manifest.json` → zip → `POST :7346/playback/upload` (O; manifest keys U) | Medium. Needs one real publish captured to pin down the format |
| `.lit` projects | Two containers: v9 lenient JSON (2020) and v23 binary chunk stream (2022), both holding one object graph. 424 class UUIDs mapped. Reader decodes all 6 samples (O) | Medium. Round-trip writer needs the slot tables, which are mostly recovered |
| Document model | `AnyObject` reflection system: Project → Canvas → MasterSurface/Slide → Surface → Contel → ContentEffect. Transactional undo (O) | **High**, because of how many object types it covers. Use plain Rust structs plus a property derive, not the C++ multiple-inheritance pattern |
| Renderer | OpenGL 3.3 render graph; warp grid of bicubic Bézier patches; per-contel homography; NanoVG stencil masks with distance-field feather; 16 blend modes (O). Draw order details U | **Med–High** (`wgpu`). It's the critical path |
| Effects | 77 built-in effects in an ISF dialect (`.liteffect` JSON + `.fs` GLSL); the wrapper template was recovered from strings (O) | Medium (`naga`/`shaderc`). The sources ship with the app, so the port can be checked pixel by pixel |
| Media | FFmpeg 4.2.1 built GPL **+ nonfree**, so it isn't redistributable. NDI SDK v4 (O) | Medium. Use VideoToolbox/AVFoundation via `objc2`; NDI via FFI |
| Scan | Done on the device; Creator downloads the images from :7345 and post-processes them with OpenCV (O) | Medium (`imageproc`) |
| UI | Native menu bar plus ~24 QML pages and dialogs; the main canvas is a custom C++ render item (O) | 3–5 dev-months in Slint, excluding the engine and canvas |
| Cloud | Login, remote devices, analytics, bug reports: NXDOMAIN, **dead**. Storyblocks catalog needs partner credentials (O) | Drop all of it |
| Scripting | Embedded Mono/C#, developer QA tooling only (O) | Drop |

## 3. Recommended scope: phased, hardware-first

Phase 1 gets a working Rust tool that drives your projector within weeks. Each later phase adds a verified layer on top and can stop wherever the value runs out.

**Phase 0: live capture with the original app (1–2 sessions, needs you + device).** Answers most of the Unknowns. It should be viable:
- `/Applications/Lightform.app` is byte-identical to the copy analysed: same main-binary and engine SHA-256.
- Rosetta is installed.
- The login screen has a **Skip** button (`LoginPage.qml:166`), so the dead login service doesn't block it.

Fallback if the app won't run: derive the package layout from `PublishManager`/`DeviceHTTPRequests` in Hopper.
1. Run Creator (under Rosetta) and use "Publish to local folder" on one sample project. That gives a real package and `manifest.json`.
2. Capture one discovery, connect and upload with a packet capture.
3. Work through the remaining live checks in `device-protocol.md` §12. **Don't trigger a firmware update**: whether the device's update servers still exist is unknown.

**Phase 1: `lf-device` crate + CLI.** Discover, connect, subscribe to state, list slides, play/pause/brightness, and upload a package produced by the original app. This proves hardware control end to end in Rust.

**Phase 2: `lf-lit` crate.** Read v9 and v23 projects. Round-trip v23 byte-for-byte on the samples, keeping unknown slots verbatim. Add asset resolution for `localassetlibrary:///` (Built_In zips + `.reference` redirects), `relativefile:///` and `file:///`.

**Phase 3: `lf-render` + publish.** A wgpu renderer covering the warp patch grid, homography, masks, blend modes and the ISF effect runtime. Render slides to H.264 via VideoToolbox and build the publish package. Verify against the Phase 0 output by pixel diff. At the end of this phase, projects authored in old Creator can be republished without it.

**Phase 4: Slint editor.** Rebuild the UI from the QML contract (`ui-and-features.md` §4), with the canvas as a wgpu surface inside Slint. Start with the Control page (small and high-value), then the Design page.

This repository is that workspace.
```
LightFormOpen/
  crates/lf-device   discovery, MetaRPC (tonic), HTTP side-channels, upload
  crates/lf-lit      chunk codec (binary + JSON), object graph, schema tables
  crates/lf-model    typed document model + undo
  crates/lf-render   wgpu renderer, ISF runtime, masks, warp
  crates/lf-publish  slide → video/png, package zip, manifest
  apps/lf-cli        Phase 1–3 driver
  apps/lf-studio     Slint app (Phase 4)
```

## 4. Constraints and risks

- **Licensing.**
  - The Atlas fonts are commercial, and the FFmpeg build is GPL+nonfree. Don't carry either into the port.
  - The QML, shaders and media are Lightform's copyrighted work. Treat them as a spec to reimplement from, not code to copy.
  - The 77 effect shaders are fine to load from your installed copy at runtime, but not to redistribute.
- **Firmware compatibility.** The gRPC schema matches what 2.2.12 expects. Older firmware may only speak the legacy HTTP API on :7341.
- **No more Lightform support.** No new firmware will ship, so the protocol is effectively frozen. That works in a port's favour. Whether the existing OTA path still works is unknown: S3 `lightform-ota` still resolves, but its contents weren't checked. Don't trigger an update until that's verified.
- **Decompiling.** Hopper is waiting on its first-run demo/license dialog. The remaining render-order questions (`engine.md` §10) need it, or Ghidra.

## 5. Open questions that block later phases

| Question | Blocks | How to answer |
|---|---|---|
| Publish zip layout + `manifest.json` keys, MD5 encoding, upload body form | Phase 1 upload, Phase 3 | Phase 0 local publish + pcap |
| Device's gRPC port / api-version, subscribe requirement | Phase 1 | Phase 0 connect |
| Surface draw order, feather/edge-offset pass placement | Phase 3 fidelity | Decompile `MasterSurfaceRenderProxy::getRenderPasses`, `ContelRenderProxy::renderContel` |
| Live-effect vs pre-rendered publish split | Phase 3 | Phase 0 publish of a slide with a LIVE effect |
| Default keyboard shortcuts | Phase 4 | Runtime dump or disassembly |
| A few v23 slot names (slide 11–13, surface 13, master-surface 9–10) | Phase 2 writer | Diff projects saved with one property changed |
