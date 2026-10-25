# Lightform Creator 2.2.12 — Engine architecture (worker C)

Scope: object model, undo, rendering, effects, media, scan-image processing, scripting, third-party
libraries. Out of scope here: `.lit` file format (worker A), device/cloud protocol (worker B).

Legend: **[O]** observed directly in the shipped binaries/resources · **[I]** inferred from names,
strings or structure · **[U]** unknown / not established.

Evidence sources (all static; nothing was executed):
- Exported C++ symbols (`nm -gU | c++filt`): Engine 35,768 · App 19,151 (mostly gRPC/AWS/protobuf) ·
  Common 4,363 · Renderer 1,349 · CreatorLibrary 27.
- Class inheritance recovered from Itanium RTTI (`typeinfo` structs + bind records): 694 edges Engine,
  278 Common, 75 Renderer. Script: `tools/re/rtti.py`; edge and class lists in `data/rtti/`.
- String tables of each binary (type IDs, enum value names, embedded GLSL, build paths).
- Effect packages `Resources/Lightform-Built_In-{1..4}_0_0.zip` (150 `.liteffect`+`.fs`, 77 unique effects).
- Extracted QML in `re/qrc/resources/src/qml`.

No Hopper decompilation was needed for this pass; symbol + RTTI + string coverage was sufficient.

---

## 1. Module layout

Build paths embedded in the binaries reconstruct the original source tree **[O]**
(`/Users/server/.jenkins/workspace/lightform-creator-release-macos/…`):

| Binary | Source module | Role |
|---|---|---|
| `libLightformCommon` | `external/lit-common/common/cpp/lightform/common/{cache,chrono,filesystem,log,memory,serialization,undoableaction,utility}` | Foundation: logging (Boost.Log), chunked binary/JSON serialization, ZIP read/write, value types, undo manager, file system, MD5, UUID, XML (tinyxml2) |
| `libLightformEngine` (53 MB) | `external/lit-engine/engine/cpp/lightform/engine/{anyobject,application,asset,cache,effects,image,property,scripting/mono,valuecontroller,video}` | Document/object model, assets, ISF effect parsing, images, video codec (static FFmpeg+encoders), OpenCV, Mono runtime |
| `libLightformRenderer` | `external/lit-engine/renderer/cpp/lightform/renderer/{gl,macos/gl,primitive,renderer,video}` | Render graph, OpenGL backend, render proxies, built-in shaders, NanoVG |
| `libLightformCreatorLibrary` | — | Tiny (27 symbols): `Application`, `ApplicationLibraryAnyObjects` (type registration glue) |
| `MacOS/Lightform` | `src/app`, `src/app/firmware`, `external/lit-engine/qtengine/…`, `external/device-client`, `external/lf-rpc`, `external/lf-ndi`, `external/cloud-api/{lightform,storyblocks,updater}`, `external/webservices` | Qt/QML app shell, Qt wrappers of engine interfaces, device client, publish, NDI, Storyblocks stock media, login, analytics, crash reporting |

The `lit-engine` name and the `.lit` extension coincide **[O]**. The engine is portable C++. The
`RenderAPI` enum lists `OPENGL`, `OPENGL_ES` and `VULKAN`, and ISF inputs include
`externalImageOES`/`samplerExternalOES` (an Android GLES texture type) **[O]**. This strongly
suggests the same engine runs on the Lightform device under GLES **[I]**.

---

## 2. Object model ("AnyObject")

### 2.1 Core mechanism
- **COM-like interface system [O].** Every document object derives from `AnyObject` and implements
  one or more `*Interface` classes, all rooted at `AnyObjectInterface`. Type and interface IDs are
  string names plus UUIDs, e.g. `lightform.object.contel` / `D2644D83-A32F-4BEE-8B11-6F4F9E5E5EEB`,
  `lightform.interface.surface` / `62DB9913-…`. There are 88 `lightform.object.*` IDs,
  76 `lightform.interface.*` IDs and 150 `lightform.enum.*` IDs.
  Lookup happens through `getInterfacePointer` and `AnyObjectInterfaceLookupID`.
  Registries: `AnyObjectDescriptionDatabase`, `AnyObjectInterfaceDescriptionDatabase`,
  `EnumDescriptionDatabase`.
- **Reflection [O].** `PropertyCollectionInterface` exposes properties by index and by *local ID*
  (`getPropertyByLocalID`, `getPropertyDescriptionByLocalID`). Object-to-object references are
  separate (`getReferenceByLocalID`, `setReferenceByLocalID`).
- **PropertyDescription [O]** stores name, group name, local ID, value type, enum ID, default,
  flags, object/interface/enum description IDs, value-controller description ID, precision,
  min/max, control type, value names/values, and a "purpose" enum (from its serializer's error
  strings).
  - Flags: `KEY_FRAMES`, `READ_ONLY`, `CREATE_INSTANCE_OF_OBJECT_DESCRIPTION`, `UI_SUBOBJECT`, `NO_UNDO`.
  - Visibility: `public` / `internal`.
  - Control type: `default` / `checkbox` / `hue_saturation`.
- **Value types [O].** The `ValueTypeVariant` covers: bool, int32, int64, float, double, string,
  UUID, URL, file-system path, date-time, angle, angle-axis, distance, Color4f, Color4u8,
  Vector2/3/4 (+ integer variants), Matrix3/4, Quaternion, Transform2/3, TimelineTime,
  TimelineDuration, enum and object reference. Each value type has a `*ValueController` plus a
  `Plain*ValueController` (stored constant) **[O]**.
  - The non-plain controllers plus the `KEY_FRAMES` flag and `KeyFrameManager`/`KeyFrameContext`
    imply keyframe animation was designed in **[I]**.
  - `KeyFrameContext` exposes only `getTime`, so animation looks minimal or unfinished in 2.2.12 **[I]**.
- **Dynamic properties [O].** `Property::createDynamicProperty` exists, and `Property` is itself an
  `AnyObject`. Effect shader parameters are stored this way: `EffectObject.shaderProperties`,
  `retiredShaderProperties` **[I]**.
- **PropertyBag [O]** aggregates properties across multi-selection (`getCommonObjectDescription`,
  `allDescriptionsMatch`) and drives the generic properties panel (`QtPropertyBag`,
  `DynamicPropertiesPanel.qml`).
- **AnyObjectManager [O]** owns objects. It handles create, clone and remove, unique naming, dirty
  counting, reference-change notification (`notifyReferencers`, depth-first propagation,
  `AnyObjectReferenceMessage`), and read/write (`readHeader`, `readIDLookup`, `readObjects`).
  It also owns the `AssetManager`, `KeyFrameManager` and `UndoableActionManager`.
- **Lifecycle hook interfaces [O]:** `PostCreate`, `PostRead`, `PreRemoveAll`,
  `RemoveAdditionalObjects`, `DelegateRemoveObject`, `OneTimeUpdate` (data migration),
  `Validity` (cache invalidation tied to timeline intervals), `Cloneable`, `SerializableState`,
  `ExecuteCommand`.

### 2.2 Document classes (RTTI bases → meaning)

| Class (type ID) | Bases (besides AnyObject + PropertyCollectionInterface) | Key properties (strings) | Projection-mapping meaning |
|---|---|---|---|
| `ProjectObject` (`lightform.object.project`) | Project, PostCreate, PostRead, OneTimeUpdate | — | Root document **[O]** |
| `CanvasObject` (`…canvas`) | Canvas, PostCreate, Validity | masterSlides, slides, activeSlide, activeSurface, artboardSize, artboardRotation/FlipH/FlipV, backgroundDisplayMode/Opacity/Fill/Brightness, playbackMode (Play/Pause/Stop), playbackTime, globalEffectEdgeThreshold | The projector's output frame ("artboard") plus the timeline **[O]/[I]** |
| `MasterSlideObject` (`…master-slide`) | Slide, RemoveAdditionalObjects | surfaces, geometries | Geometry shared by all slides: the mapped surfaces **[O]/[I]** |
| `SlideObject` (`…slide`) | Slide, Cloneable | masterSlide, duration, outOfBoundsBehavior, relativePlaybackTime, fadeIn/Out(+Duration), publishingEnabled, thumbnailUrl, audioReactivitySettingsString, skipTranscodeOnPublish, category | One timed scene in the slideshow; published to the device as a rendered video **[O]/[I]** |
| `MasterSurfaceObject` (`…master-surface`) | Surface, HasTransform, Transformable, Cloneable, Validity, PostCreate/Read, RemoveAdditionalObjects | geometries, structure, geometryFeatherWidth, geometryEdgeOffset, perspective, autoPerspectiveFit, autoPathNamingEnabled | A physical surface's mask geometry (paths) plus its warp structure, shared across slides **[O]/[I]** |
| `SurfaceObject` (`…surface`) | Surface, Blendable, ContentEffectCollection, ExecuteCommand, Cloneable | masterSurface, contels, geometryFill/BorderColor, geometryBorderWidth, audioReactive | Per-slide instance of a master surface that holds the content layers **[O]/[I]** |
| `StructureObject` (`…structure`) | Structure, Bezier, Geometry, Transformable, Validity | gridSize, envelope, uneditedEnvelope, cornerPinFlags, hasBeenCornerPinned, hasLinearGrid | The warp: a grid of bicubic Bézier patches (`StructurePrimitive::calcBezierPatches`, `bezierSurfacePoint`, `bernsteinCoefficient`) with corner-pin and envelope editing; outputs projected UVs (`setUvBufferData`, `getProjectedStructureCoordinates`) **[O]** |
| `PathObject` (`…path`) | Path, Bezier, Geometry, Transformable, CreateByInputMode, Validity | — | Vector mask shape: rectangle, ellipse and n-gon creation modes, Bézier editing **[O]** |
| `CompositePathObject` (`…composite-path`) | CompositePath, EditablePoints, Geometry, Transformable | — | Multi-contour path. `PathBooleanOperator` gives boolean ops; `expand` gives edge offset **[O]** |
| `ContelObject` (`…contel`) | Contel, Blendable, ContentEffectCollection, Geometry, HasProjection, HasTransform, Transformable, ExecuteCommand | contelType (NONE/IMAGE/VIDEO/EFFECT/TEXT), opacity, blendMode, inheritAlpha, contentEffects, projection (3×3 matrix), borderWidth/Feather, textImageAsset, videoSpeed, videoTrimStart/End, videoDuration, visibleInCanvas, locked | "Content element": one layer of media (image, video, ISF effect or text) inside a surface, placed by a homography (`setProjection(Matrix3f)`, `fitPerspective`, `resetWarp`) **[O]** |
| `ContentEffectObject` (`…content-effect`) | ContentEffect (→ Blendable) | type (Stroke, Color Fill, Color Stroke, HSV Fill, Brightness/Contrast), strokeWidth, saturation, luminosity, brightness, contrast | Per-layer colour filters, compiled into the contel's fragment shader (`applyContentEffects`) **[O]** |
| `EffectObject` (`…effect`) | Effect, Cloneable, Validity | effectAsset, shaderProperties, retiredShaderProperties | Instance of an ISF effect with parameter values **[O]** |
| `EffectPresetObject` (`…effect-preset`) | EffectPreset | presetName, effectExternalAssetReferenceUrl, effectShaderProperties | Named parameter set (`PRESETS` in `.liteffect`) **[O]** |
| `AssetObject` (`…asset`) | Asset, AssetReferencer, Cloneable, TimelineTick, PostRead, PreRemoveAll | displayName, scanImageID, publishedPresence | Project reference to an image, video or effect file **[O]** |
| `ScanSessionObject` (`…scan-session`) | ScanSession | scanImageAssets, timestamp, deviceID, deviceScanID, projectorDimensions, disparityValid, deviceRemote | One device scan: the camera images and disparity map used as backdrop and effect inputs **[O]** |
| `BackgroundObject` (`…background`) | Background | visual; display modes COLOR_RGB/GRAY, DISPARITY_GRAY, DISPARITY_COLORMAP_JET, PROJECTOR_PREVIEW | Editor backdrop (scan photo, depth view or projector preview) **[O]** |
| `ActionObject` / `ActionManagerObject` / `KeyCombinationObject` | Action(Manager), KeyCombination | — | Command and shortcut registry (also used for key-triggered slide actions **[I]**) |
| `ApplicationModelObject`, `ApplicationSettingsObject`, `DialogApplicationSettingsObject`, `EditorSettingsObject` | … | isFirstRun, labsFeaturesEnabled, developerFeaturesEnabled, stream quality, UI sizing | App state and settings, persisted through the same object system **[O]** |
| `AnyObjectTransientUnorderedCollectionObject`, `AnyObjectSelectionSetCollectionWrapperObject` | Collection, SelectionSet | — | Selection sets (not saved) **[I]** |

Hierarchy summary **[O]**: `Project → Canvas → {MasterSlide → MasterSurface → (Path/CompositePath geometry, Structure)} + {Slide → Surface(→masterSurface) → Contel → (Effect | Asset) + ContentEffects}`.
The edges come from reference properties; the exact containment is A's serialization concern **[I]**.

Project types: `STANDARD`, `OBJECT_REFERENCE_IMAGE` ("Auto-Align Object"), `REFERENCE_IMAGE` **[O]**.

### 2.3 Editing tools
- `InteractiveViewport` hosts `InputMode`s **[O]**: `CreatorMoveTool`, `CreatorEditTool`, the
  rectangle/ellipse/n-gon path creation modes, `CreateProjectSurfaceGeometryInputMode` and
  `PixelInputMode`.
- `PixelInputMode` provides brush and magic-wand selection on the scan image
  (`MagicWandAlgorithm`, a scanline flood fill) and vectorization **[O]**. Vectorization runs
  OpenCV `findContours`/`approxPolyDP` **[I]**, then `SimplifyPath` (Ramer–Douglas–Peucker style
  **[I]**), then `FitCurve` **[O]**. `FitCurve` is Schneider's *Graphics Gems* cubic Bézier
  fitting **[O]**: its method names `chordLengthParameterize`, `reparameterize` and
  `newtonRaphson` match that algorithm.
- Pick/selection: `PickPath`, `ComponentType` (OBJECT, CONTROL_POINT, CORNER_PIN, ENVELOPE),
  and state flags (SELECTED, HIGHLIGHTED, PICKABLE) **[O]**.

---

## 3. Undo / command system **[O]**
- `UndoableActionManager` and `UndoableActionContext` live in Common.
- **Action contexts are transactions.** `addActionContext` → `pushDoneAction`* →
  `commitActionContext` or `cancelActionContext`.
- **RAII guards:** `AutoUndoableActionContext`, `AutoCommitUndoableActionContext`,
  `AutoCancelUndoableActionContext`.
- **Modes nest contexts:** `addActionMode`, `commitActionMode`, `cancelActionMode`, with a
  per-mode maximum action count.
- **Navigation:** `goToPreviousActionContext`, `goToNextActionContext`, `rollBackTo`,
  `mergeBackTo`, `mergeBackToFirst`, plus title queries for the menu labels.
- Each context stores `undoActions`/`doActions` closures (`callErasingActions`) **[I]**: command
  pattern with closures rather than snapshots.
- The property flag `NO_UNDO` opts properties out. `InteractiveAnyObjectManagerActionState`
  coalesces drags **[I]**.

---

## 4. Rendering pipeline

### 4.1 API and threading
- **OpenGL only in this build [O]:** `OpenGLRenderAPI`, `GLContext`, `GLShader`, `GLTexture`,
  `GLFramebuffer`, and `renderer/macos/gl`. Shaders are `#version 330` and `#version 150 core`.
- Vulkan and GLES appear only as enum values.
- The App binary links Metal, but no Metal renderer symbols exist **[O]**. The Metal link is
  likely for GPU info or Qt **[I]**.
- `RenderManager` runs a render command thread (`RenderThreadMode` DISABLED/RESOURCE/ENABLED) with
  resource pooling and caching and fences **[O]**.
- Qt Quick integration: `QtGraphicsQuickControl` with RendererFrontEnd/BackEnd, and
  `CreatorRendererFactory` / `CreatorViewRendererFactory` / `SlideViewRendererFactory`
  (App RTTI) **[O]**. The engine renders into textures composited by QML **[I]**.
- `RenderDocManager` supports frame capture (debug) **[O]**.

### 4.2 Render graph **[O]**
- **Frame-graph design.** `RenderGraph` takes registered passes (`registerRenderPass`,
  `registerUpdatePass`, named dependencies, explicit textures) and compiles them into a
  `CompiledRenderGraph`. That stage resolves aliasing, variables and targets, then executes.
- **Scene side.** `CreatorRenderScene` builds a tree of `RenderProxy` objects mirroring the
  document: `ViewportRenderProxy`, `MasterSurfaceRenderProxy`, `ContelRenderProxy`,
  `StructureRenderProxy`, `CornerPinRenderProxy`, `CanvasRenderProxy`.
- **Graph outputs.** The scene exposes per-slide RGB and YUV targets (`getSlideGraphTargetName`,
  `getSlideYUVGraphTargetName`). The YUV target feeds the video encoder at publish time **[I]**.

### 4.3 How a surface is drawn [O unless noted]
1. **Mask.** Rasterize the master surface's mask: `MasterSurfaceRenderProxy::renderSurfaceMask`
   from Path/CompositePath geometry. NanoVG (`glnvg_*`, `AA_STENCIL` renderer type) draws
   anti-aliased stencil fills **[O]**.
   - Feather and edge offset come from geometry properties (`#define feather frag[9].w`,
     `maskDistField`). The mask is a **distance field**, so feathering is a soft threshold **[I]**.
2. **Warp.** The structure, a grid of Bézier patches, is evaluated into an indexed triangle mesh
   (`IndexedTriangleMesh<StandardVertexFormat…>`, `StructureMesh`) with UVs (`PS_StructureUV`).
   It maps surface-local content space onto artboard/projector pixels **[O]/[I]**.
   - Corner pin uses `makeCornerPinMatrix` and `QuadrilateralF` (a 4-point homography) **[O]**.
3. **Contel placement.** Each contel draws with its 3×3 projection matrix inside the surface's
   UV space (`texCoord.xy / texCoord.w` gives a perspective divide). It is clipped by
   `_UVBounds` **[O]**.
4. **Contel shading.** ISF effect, image or video texture (YUV→RGB in shader: `PS_YUVConversion-*`
   for NV12, YUV420P and similar) **[O]**.
   - Then `applyContentEffects`: HSV shift, brightness/contrast, colour fill and stroke **[O]**.
   - Then blend against the destination (`texelFetch(dst, …)` when `FULL`) using **16 Photoshop
     blend modes** (Darken … Luminosity) and Porter-Duff ops (SOURCE_IN/OUT,
     DESTINATION_IN/OUT) **[O]**.
   - The shader handles premultiplied alpha (`PREMULTIPLY_SHADER`) **[O]**.
5. Utility passes: Gaussian blur H/V, dilate/erode morphology, passthrough
   (`AUDIO_REACTIVE_MASK` variant), pixel grid, scan-image display, projection preview
   (SIMPLE/LAMBERT using a normals map), UI overlay with edge AA **[O]**.
6. **Output to projector.**
   - On the desktop, the canvas renders to an artboard-sized target for preview.
   - It is also streamed to the device over NDI (`NDISender`, `ApplicationStreamVideoType`
     LOW/MEDIUM/HIGH) **[O]/[I]**.
   - Or it is rendered per slide and encoded to video for publish **[I]**.
   - The device receives video or content and projects it. Device protocol: worker B.

### 4.4 Effects (ISF dialect) **[O]**
- **Package format.** `effects/<name>/<name>.liteffect` (JSON) + `<name>.fs` (GLSL body) +
  `.liteffect.gif` (thumbnail).
  - The JSON is **ISF metadata split into its own file**, rather than the standard ISF comment
    header.
  - Keys: CREDIT, DESCRIPTION, CATEGORIES (effects/generators), TAGS, INPUTS, PRESETS,
    DATE_ADDED, IMPORTED.
  - No built-in effect uses PASSES. The parser recognises `PASSES` and the template defines
    `PASSINDEX` **[O]**; multipass support itself is **[U]**.
- **INPUT TYPEs** seen: float (533), image (124), bool (109), point2D (92), color (87),
  point3D (66), long (8).
  - The parser also accepts `audioFFT` and `externalImageOES`.
- **Lightform extensions:**
  - `VISIBILITY: internal`.
  - `CONTROL_TYPE: hue_saturation`.
  - `point3D`.
  - `IDENTITY` value.
  - Image inputs auto-bound to scan-derived textures: `LF_scan_rgb`, `LF_scan_rgb_cartoonized`,
    `LF_scan_disparity`, `LF_scan_normals`, `LF_scan_edge_trace`, `LF_scan_edge_distance`,
    `LF_scan_edge_distance_trace` (plus legacy `scanImage`/`disparity` names).
  - `channel0..3` are user-assigned image inputs **[I]**.
- **Shader template** (strings in Renderer):
  - **Uniform block.** `STD140_UNIFORM_BINDING(4) PerEffect { vec2 RENDERSIZE; float TIME;
    float TIMEDELTA; vec4 DATE; float SLIDE_LENGTH; int FRAMEINDEX; int PASSINDEX;
    float _ContelOpacity; vec4 _UVBounds; ivec3 _PorterDuff; … }`.
  - **Source rewrites (regex):**
    - `gl_FragColor` → `isf_FragColor`
    - `gl_FragCoord` → `getFragCoord()`
    - `IMG_NORM_PIXEL(a,b)` → `texture(a,b)`
    - `IMG_THIS_NORM_PIXEL(a)` → `texture(a, isf_FragNormCoord.xy/isf_FragNormCoord.w)`
    - `void main()` → `void isf_main(out vec4 isf_FragColor)`
  - **Includes:** `Common.glsl`, `Common_ISF.glsl`, `Common_Noise.glsl`, `Common_Shadertoy.glsl`,
    `Common_Compositing.glsl`. `#define TWO_PI` is predefined.
  - **Desktop audio.** On desktop, `LF_AUDIO` and `LF_AUDIO_ACCUM` are `const vec4(0.0)`, so audio
    reactivity only runs on the device **[O]/[I]**.
- 77 unique built-in effects across the 4 libraries (list in §9). Libraries 2–4 chain to earlier
  ones (`manifest.json` → `assetLibraryReferences`); library 4 references 1, 2 and 3.

---

## 5. Media pipeline

| Concern | Implementation | Tag |
|---|---|---|
| Video decode | `VideoDecoder` (FFmpeg 4.2.1 static, built with `--enable-videotoolbox`) with plane access (`getPlaneInfo`, YUV formats), `TimelineVideoDecoder` (seek-to-timeline-time), `VideoRescaler` (swscale) | O |
| Video encode | `VideoEncoder` (FFmpeg muxer + libx264/libx265/OpenH264/libvpx; `EnableHardwareAcceleratedVideoEncoder` setting → VideoToolbox) used by `PublishManager` (“Failed to encode published slide %1%”) | O |
| Images | `PNGReader/Writer`, `JPEGReader/Writer`, `ImageCache`, `PixelCache`, stb_image in Renderer, OpenCV imgcodecs (EXR/WebP/JPEG2000 available) | O |
| SVG / text | `SVGDocument` (tinyxml2-based **[I]**); `TextImageRenderSettings` + `QtTextImageRendererAssetProvider` → text contels are rasterized to images by Qt **[I]** | O/I |
| Asset system | `AssetManager` = Cache + Database + Factory + FileSystem + Notifications + RequestQueue (RTTI MI); providers: `FileSystemAssetProvider`, `LocalAssetProvider`, Storyblocks (App); built-in libraries referenced by `*.reference` JSON (`assetLibrary`, `assetPath`) with `*.metadata` (width/height/pixelFormat/fileSize) | O |
| NDI | `libndi.4.dylib` (NDI SDK v4). App imports both `NDIlib_recv_*` (receive camera/peek stream from device, `CameraStreamWindow`) and `NDIlib_send_*` (send canvas to device/stream) | O (direction purposes I) |
| Audio | FFmpeg built with fdk-aac/opus/vorbis; no desktop audio-reactive path observed | O/I |

---

## 6. Scanning and scan-image processing
- **The scan is captured on the device [I].** The desktop stores a `ScanSessionObject` with
  scan image assets, `deviceScanID`, `projectorDimensions` and `disparityValid`. Scan control
  messages are in App (`Cloud…ScanStateRequest`, `CameraFrameRequest`, `ScanWorkflowObject`).
  Protocol: worker B.
- **Desktop post-processing [O]** runs in `AuxiliaryEffectImages` on worker threads, producing:
  - `aux::rgbd_bifiltered`: bilateral-filtered colour and disparity.
  - `aux::normals`: from disparity.
  - `aux::edges_distance`: Canny edges, then a distance transform ("Tracer").
  - `aux::img_effects`: "Cartoonize".
  - `DisparityPostprocessor` (rotation, filtering).
- **OpenCV functions present** in exported symbols: `bilateralFilter`, `Canny`,
  `distanceTransform`, `findContours`, `approxPolyDP`, `medianBlur`, `morphologyEx` **[O]**.
- **No reconstruction on desktop [O]** (negative within exported symbols): no `findHomography`,
  `StereoSGBM`, `structured_light` decoding or calibration symbols.
- These images become the `LF_scan_*` effect inputs and the background display modes **[O]/[I]**.
- `Eigen` is used throughout for geometry types (`Matrix3f` projections, `Transform<float,2>`) **[O]**.

---

## 7. Scripting (Mono / C#) **[O]**
- The Mono JIT runtime is statically linked into the Engine (`mono_*`, "Mono JIT compiler version %s").
- C# sources (`*.cs`) are compiled with `mcs.exe` against `/r:mscorlib.dll /r:System.dll` and the
  SWIG-generated wrappers `LightformCommon_wrap.dll`, `LightformEngine_wrap.dll` and
  `LightformCreatorLibrary_wrap.dll` (`_CSharp_Lightformf…` / `SWIGRegister…` exports in Common).
- Purpose: `MonoScriptTestBench/Runner` (unit tests → `tests.dll`) and
  `MonoScriptUtilityBench/Runner` (utility scripts → `utilities.dll`), surfaced in
  `ScriptingDialog` and `ApplicationStatusDialog`. It is **developer and QA tooling, not a user
  feature [I]**.
- The bundle ships no `mscorlib.dll`, Mono class libraries or wrapper DLLs (looks for
  `%1%/lib/mono/4.5`), so scripting **does not work in the shipped build without an external Mono
  install [I]**.

### Labs and developer features **[O]**
- `labsFeaturesEnabled`, toggled via the `EnableLabsDialog` and Labs icon, adds a Labs indicator
  stripe and device FPS readouts in the camera stream and scan workflow.
- It changes "Pair Device to Wi-Fi Network" wording and part of the publish workflow
  (`PublishWorkflow.qml:237`).
- It shows `LabsIndicator` badges on individual properties and group headers.
- Several pixel-tool rows were labs-gated, but those gates are commented out (now always on).
- `developerFeaturesEnabled` exposes extra contel panel rows (`custom-panels/lightform.object.contel.qml`).

---

## 8. Third-party components

| Library | Version | Where | Tag | Rust replacement |
|---|---|---|---|---|
| Qt (Core, Gui, Qml, Quick, Quick3D-family, Svg, Network, Sql, VirtualKeyboard…) | 6.2.x | Frameworks | O | Slint (UI); drop Qt |
| OpenCV + contrib (aruco, ximgproc, structured_light, rgbd, stereo… built; only a few used) | 4.5.0 | static in Engine | O | `imageproc` (canny, distance transform, contours, median, morphology, bilateral) or `opencv` crate |
| FFmpeg (GPL **+ nonfree**: libx264, libx265, OpenH264, libvpx, fdk-aac, opus, vorbis, webp, VideoToolbox, AVFoundation) | 4.2.1 | static in Engine | O | `ffmpeg-next` (LGPL build) or `objc2-av-foundation`/VideoToolbox; `openh264` crate for encode |
| x264 / x265 / OpenH264 (Wels) / FDK-AAC | x265 copyright 2013-2018 | static in Engine | O | as above; avoid GPL/nonfree |
| libjpeg-turbo / libpng / zlib / OpenEXR / OpenJPEG / libwebp | 2.0.2 / 1.6.37 / 1.2.11 / 2.3.0 / 2.3.1 / ? | static | O | `image`, `zune-jpeg`, `png`, `exr` |
| Eigen | ? | header-only | O | `nalgebra` / `glam` |
| Boost (Log, Asio, Thread, property_tree, multiprecision) | ? | Common/Engine | O | `tracing`, `tokio`, std |
| Mono runtime + SWIG C# bindings | ? | Engine/Common | O | drop (or `rhai`/`mlua` if scripting wanted) |
| NanoVG | ? | Renderer | O | `femtovg` (NanoVG-derived) or `lyon` / `vello` |
| stb_image | ? | Renderer | O | `image` |
| tinyxml2 | ? | Common/App | O | `quick-xml` / `usvg` for SVG |
| NDI SDK | v4 (`libndi.4`) | Frameworks | O | FFI to NDI SDK (proprietary licence) |
| gRPC C++ / protobuf / abseil / re2 | ? | App | O | `tonic` + `prost` (worker B scope) |
| AWS SDK C++ | 1.8.139 | App | O | drop / `aws-sdk-*` if needed |
| libcurl / OpenSSL | 7.75.0 / 1.1.1k | App | O | `reqwest` + `rustls` |
| Crashpad | ? | App + `Resources/crashpad_handler` | O | `sentry` / `minidumper` |
| RenderDoc API | — | Renderer | O | wgpu has RenderDoc capture support |
| Atlas Grotesk font (Commercial Type, commercial EULA) | 1.001 | embedded in App | O | **must be replaced** (licence) |
| Path boolean / tessellation library | — | — | U (no Clipper/libtess/earcut strings; NanoVG stencil fills) | `i_overlay` / `geo` booleans, `lyon` tessellation |

Licensing note **[O]/[I]**: FFmpeg was configured with `--enable-gpl --enable-nonfree` and
fdk-aac. That combination is not redistributable. A Rust port should not copy this media stack.

---

## 9. Rust mapping and difficulty

| Subsystem | Rust approach | Difficulty | Notes |
|---|---|---|---|
| Object model / reflection | Plain Rust structs per object type + arena IDs (`slotmap`), `serde`; keep type/interface UUIDs + property local IDs as constants for `.lit` compatibility; a small derive macro for property reflection (generic properties panel + undo) | **High** (breadth: ~25 object types, ~30 value types) | Don't port the COM-style multiple inheritance; use traits for the few cross-cutting behaviours (Transformable, Geometry, Blendable) |
| Undo | Command pattern with transaction contexts (`Vec<Box<dyn Command>>` per context, merge/coalesce) | Low–Med | Semantics are clear from the API above |
| Geometry (paths, booleans, Bézier patch structure, homography, corner pin) | `kurbo` (Bézier, curve fitting), `i_overlay` (booleans), `nalgebra` (3×3 homography, patches) | Medium | Bicubic Bézier patch grid + Schneider fitting + RDP are textbook |
| Render graph + compositing | `wgpu` with a small hand-rolled pass graph; port the blend-mode / Porter-Duff / content-effect GLSL into WGSL, or keep GLSL via `naga`'s GLSL front end / `shaderc` | **Med–High** | The shader templates are fully recoverable from strings (§4.4) |
| ISF effect runtime | Parse `.liteffect` JSON (`serde_json`), apply the same regex rewrites, wrap in a GLSL 450 template, compile via `shaderc`→SPIR-V→`naga` (naga GLSL front end requires 440/450 syntax) | Medium | 77 effects ship as source, so effect behaviour can be verified pixel-for-pixel against Creator |
| Vector mask rendering | `lyon` tessellation or `femtovg`, distance-field feather in a shader | Medium | |
| Video decode | `ffmpeg-next` (LGPL build with VideoToolbox) or AVFoundation via `objc2` | Medium | |
| Video encode for publish | VideoToolbox H.264 via `objc2`, or `openh264` crate | Medium | Codec and container must match what the device accepts (worker B) |
| Images | `image` crate | Low | |
| Scan aux images | `imageproc` (+ hand-written bilateral, normals from disparity) or `opencv` crate | Medium | Output only needs to be "close enough" visually |
| NDI | FFI bindings to NDI SDK | Medium | Proprietary SDK licence |
| Text contels | `cosmic-text`/`parley` → rasterize to texture | Low–Med | |
| Scripting (Mono) | Drop | — | Dev tooling only |
| Qt Quick viewport embedding | Slint with wgpu texture import (Slint `unstable-wgpu-*` integration) | Medium | 400+ QML files' behaviour maps to Slint components separately |
| Crash/analytics/AWS/Storyblocks | Drop or `sentry` | Low | The cloud services are likely dead (worker B) |

Built-in effects (77): 3d_wiggle audio_bars bass_drop box_flip breath checkerboard chroma chromatic
circles clock color_flood color_lines crystals depth_trace digital_fade directional_light disasphere
dissolve dot_matrix fill fire fireflies firework fold ganzfeld ghost_flash glassy_lens gradient
hue_picker iridescent_edges iso_bands iso_shading light_house light_tunnel looney_tunes meshify
neon_loops night_vision noisy_circles oil_sheen old_movie palette_trip perlin_noise pie_chart
pixel_fade pixellate plasma point_light pulse quilt rainbow_edges rainbow_tiles ripple scan
scan_image scorch sdf_grid sdf_shape simple_wipe slice solid_color sonar split stripify
sunrise_sunset surface_highlight swirl tie_dye tiles traffic_jam transition_fade transition_wipe
tron tv_static voronoi water wood_grain.
(`fire` and `quilt` descriptors contain raw control characters; `json.loads(strict=False)` reads them.)

---

## 10. Open questions (Unknown)
- Whether keyframe animation of properties is reachable in the 2.2.12 UI, or only plain values are used.
- Exact surface draw order and where geometry feather and edge offset are applied (mask
  distance-field pass vs contel pass). Confirm by decompiling `MasterSurfaceRenderProxy::getRenderPasses`
  and `ContelRenderProxy::renderContel`.
- Which library implements path boolean ops and fill tessellation (NanoVG stencil vs custom).
- Whether `PASSES` (multipass ISF) is supported end-to-end.
- Whether the full slide renders to video on publish, or live effects run on the device (the
  `promoteSelectedToLiveEffects` and `LiveEffectMasks` strings suggest both paths exist). Coordinate
  with worker B.
