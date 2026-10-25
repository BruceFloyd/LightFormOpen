# Lightform Creator `.lit` project format

Scope: on-disk project layout, the `.lit` container formats, the object model they serialize, and how assets are referenced. Source: Lightform Creator 2.2.12.1054 (macOS, x86_64).

Evidence tags:
- **[O] Observed:** seen in sample bytes or decoded output.
- **[I] Inferred:** from symbols, strings or disassembly.
- **[U] Unknown.**

Samples used (six user-authored projects, not distributed; copies were analysed, originals untouched):

| Project | Container | Data version | Date |
|---|---|---|---|
| Sample 1 (new, empty) | binary | 23 | 2022-08 |
| Sample 2 | binary | 23 | 2022-08 |
| Sample 3 | JSON | 9 | 2020-02 |
| Sample 4 | JSON | 9 | 2020-03 |
| Sample 5 | JSON | 9 | 2020-02 |
| Sample 6 | JSON | 9 | 2020-03 |

`~/Library/Application Support/Lightform/Lightform Creator/ApplicationSettings.litsettings` was also decoded.

Reader: `tools/re/litdump.py`. It decodes all 6 projects and the settings file with no undecoded values.

```
python3 -I litdump.py "X.lit" --registry lit_class_registry.json [--tree] > X.json
```

## 1. Project directory layout

```
<Project Name>/
  <Project Name>.lit      the document (binary or JSON container, see §2)
  assets/                 imported media; "stock/" subfolder in 2020 projects (empty in samples)
  scans/                  device scan images, referenced as relativefile:///scans/<file>
    camera-color.jpg  camera-disparity.(png|jpg)  camera-mask.png
    projector-color.jpg  projector-color1.jpg  projector-disparity.png
    projector-mask-distance.png
```

- **[O]** Seen in all 5 projects that have folders. The file names match the `scanImageID` enum values on Asset objects: `CAMERA_COLOR`, `CAMERA_DISPARITY`, `CAMERA_MASK`, `PROJECTOR_COLOR`, `PROJECTOR_COLOR_1`, `PROJECTOR_DISPARITY` and `PROJECTOR_MASK_DISTANCE`.
- **[I]** Project packaging exists for archives and publishing: `ProjectPackager`, plus the strings `createZipPackage`, `packageProject`, `validatePackage` and `archived`. The archive layout is **[U]**; no packaged sample was available.
- **[O]** The app keeps per-user state in `~/Library/Application Support/Lightform/Lightform Creator/`:
  - `ApplicationSettings.litsettings`, which uses the same container; recent projects are stored in plain-file-system-path values.
  - `Storyblocks/{data,metadata}/xx/`, a stock-media cache.
  - Logs, crash data, and `AnalyticsEvents.json`.

## 2. Containers

The document layer is generic. `.lit`, `.litsettings` and, **[I]**, `.litpreset` (the string "Lightform Preset (*.litpreset)") all use it. Two serializations exist. Both are implemented in libLightformCommon: `BinaryDataChunkReader/Writer` and `JsonDataChunkReader/Writer` **[I]**.

### 2a. Binary container ("Lightform Binary Document", 2022 projects)

All integers are little-endian **[O]**. The file is a flat stream of chunks:

| Tag (u8) | Layout | Meaning |
|---|---|---|
| `01` | `u32 len=16`, `uuid[16]` (RFC 4122 byte order) | Begin a nested chunk. The UUID names the chunk *type* (§2c), not an instance. |
| `02` | — | End of the innermost nested chunk |
| `03` | — | End of document; the final byte of every sample |
| `04` | `u32 len`, `u32 key`, `value[len-4]` | Single-valued property |
| `05` | `u32 len`, `u32 key`, `u32 n`, `elements…` | Array property ("values", "id-lookup") |

- **[O]** These are the only tags seen. The reader API also has `readBlob`, `readInt8`/`readInt16`, `readSmallCount` and `getByteOrder` **[I]**, so other tags or a big-endian variant may exist **[U]**.
- **[O]** Each `key` is a 32-bit hash of a kebab-case property name: `h = 0; for byte c: h = h*33 + c` (mod 2^32), with no seed. For example `"id"` = `0x00000ded`. All 21 keys in the samples resolve against the engine's chunk-property-name table (§2c).
- Scalar encodings **[O]**:
  - int: `u32`, or `i64` when the payload is 8 bytes. `count` is `i64` inside state chunks.
  - bool: 1 byte.
  - string: `u32 byte_len` followed by UTF-8 bytes. Used for name, format, value-type and dynamic-description values.
  - `magic` is the raw 4 bytes `litb`.

Top-level stream **[O]**:

```
04 magic="litb"  04 format="Lightform Binary Document"  04 format-version=1
04 max-bytes-per-line=i64 0xFFFFFFFF
01 header{ data-version-number=u32 23, original-data-version-number=u32 23 } 02
01 id-lookup{ count=u32 N, values=[N × uuid16] } 02
01 objects{ count=u32 M, (01 object{…} 02) × M } 02
03
```

- **ID interning [O].** Every UUID in the document is written once into `id-lookup`. All later id/class-id/owner-id/subtype-id/reference fields are `u32` indices into that table.
- **[O]** Interning order follows writing order (self, owner, class), so the class and owner UUIDs are mixed in with the object ids.

### 2b. JSON container ("JSON Document", 2020 projects)

**[O]** This is not strict JSON: the writer leaves trailing commas, which the reader strips before parsing. Keys are camelCase. A nested chunk is written as `"<chunk-name>|<chunk-type-uuid>": {…}`. Ids are UUID strings, with no id-lookup table.

```
{"format":"JSON Document","formatVersion":1,"blobTextFormat":"byte-array",
 "header|8e73ed04-…":{"dataVersionNumber":9,"originalDataVersionNumber":9},
 "objects|54fa5fd0-…":{"count":3256,
   "0":{"name":"Path_points_Property_ValueController","id":"<uuid>","classId":"<uuid>","ownerId":"<uuid>",
        "state|0c5a8d0c-…":{"isArray":true,"count":115,"values":[…]},
        "references|95623259-…":{"count":1,"0":{"localId":0,"id":"<uuid>"}}}, … }}
```

### 2c. Chunk types and property names

**[O]/[I]** The chunk-type UUIDs sit next to their names in libLightformEngine's string table.

| Chunk | UUID |
|---|---|
| header | 8e73ed04-344f-45f0-8821-85fa77dcb9b5 |
| id-lookup | 4505e8fb-03b9-45ae-bb0d-94c83bc2944d |
| objects | 54fa5fd0-d3cb-472f-b660-3bb67f89eef4 |
| object | 787ea4ea-18e1-4e76-97d4-2795d6fe8f50 |
| state | 0c5a8d0c-d533-47b7-bc25-4e21443714e1 |
| references | 95623259-f9de-4a79-922b-e4eef38436dd |
| reference | 607eb63c-c47e-4a87-97b3-135429656984 |
| dynamic-description | 6ce5a4ab-d5c1-4d94-ac26-0fe82546a1cf |
| properties *(not in samples)* | cb30171e-c4bc-4682-901d-1e2cbc802498 |
| property *(not in samples)* | 15a78dd1-1e93-44aa-af02-f02085d75711 |

The property-name table, in engine order:

> class-id, class-name, control-type, count, data-version-number, default-value, enum-description-id, flags, group-name, interface-description-id, is-array, local-id, maximum-value, minimum-value, object-description-id, original-data-version-number, owner-id, precision, purpose-enum-description-id, purpose-enum-value, range, subtype-id, value, value-controller-object-description-id, value-count, value-names, value-type, values

The header also uses magic, format, format-version, max-bytes-per-line, id and name.

## 3. Object model

**[O]** Each `object` chunk holds:

- `id`
- `class-id`
- `owner-id`: the parent object's UUID. Absent only on roots.
- optional `name`
- optional `state{…}`: the object's own value
- optional `references{count, reference{local-id, id}…}`: the object's slots

Classes are identified by UUID. `lit_class_registry.json` maps 424 UUIDs, gathered from the (display name, `lightform.<kind>.<name>`, UUID) triples in the binaries, to type names **[O/I]**.

Three layers:

1. **Domain objects** such as Project, Canvas, Slide and Surface. `references[localId]` fills the class's property slots.
2. **Property** (`lightform.object.property`, 7c0b2130-…).
   - **JSON v9 [O]:** every slot goes through a Property object (`<Owner>_<prop>_Property`), whose `references[0]` is the ValueController.
   - **Binary v23 [O]:** static Property objects are not written. The domain object's `references[localId]` points directly at the ValueController, whose `owner-id` is the elided property's UUID. That UUID appears only in id-lookup.
   - **Dynamic Properties [O]**, such as custom shader inputs, are still written. Their `state` holds a `dynamic-description{name, group-name, value-type ("FLOAT"…), default-value, minimum-value, maximum-value, control-type}`, with every value as a string.
3. **ValueControllers** (`lightform.object.value-controller.plain-*`) hold the data in `state`:
   - `values`: the array.
   - `is-array` and `count`: present for array-valued slots.
   - `subtype-id`: the enum-description UUID, for enums.

`property-array` objects (Effect.shaderProperties) have `state{is-array, count}`. Their `references[i]` point to dynamic Property objects.

Object references are themselves ValueControllers (`plain-object-reference`) with no `values`. The targets are in that controller's `references`, with `local-id` as the array index. A non-array reference has an empty `state`. **[O]**

### 3a. Value encodings in the binary `values` array (tag 05)

**[O]** `n` counts *components*, not elements. A vector2 array of 4 points has n=8.

| Controller | Component | Notes |
|---|---|---|
| float, vector2/3/4, color4f (RGBA), matrix3 (9), matrix4 [I], quaternion [I] | f32 | |
| double, angle, distance [I] | f64 | angle unit assumed radians [U] |
| int32 [I], vector2i, vector3i [I], vector4i [I] | i32 | |
| int64 | i64 | |
| boolean | u8 | |
| color4u8 [I] | u8 | |
| timeline-time, timeline-duration | i64 ticks | **705,600,000 ticks/s (flicks)**, from the constant in `TimelineDuration::toSecondsDouble` [O] |
| date-time | i64 Windows FILETIME | 100 ns since 1601. Decodes to 2022-08-12T14:41:41Z, matching the file mtime [O] |
| string, url, enum, file-system-path | `u32 len` + UTF-8 per element | enums store the value **name**, e.g. `"UNION"` |
| uuid | 16 bytes | |
| transform2/3, angle-axis | — | [U] not in samples |

The JSON container uses numbers, bools, ISO-8601 date strings (`"2020-02-28T14:54:08Z"`), tick strings with an `flk` suffix (`"7056000000flk"` = 10 s), and UUID strings. **[O]**

### 3b. Class slot tables (localId → property : controller)

- Slots without a source tag are **[O]**: names from v9 JSON Property names, types confirmed in v23 binary with **0 type mismatches** across 537 shared slots.
- Slots marked **[I]** exist only in v23. Their names are inferred from the per-class property-name string run in libLightformEngine, which is in local-id order with shared names deduplicated.
- Gaps are local-ids never seen in samples.

- **project** (3690a615):
  - 0 canvas:objref
  - 1 deviceName:string. The engine name order suggests `deviceID` [I].
  - 5 referencedAssets:objref (engine: importedAssets)
  - 6 scanSessions:objref
  - 7 activeScanSession:objref
  - 8 isDeviceRemote:bool [I]
  - 9 projectType:enum [I]
  - 10 projectDescription:string [I]
  - 2–4 are likely projectFilePath, isProjectTemporary and projectName [I]; not serialized.
- **canvas** (db3a2dff):
  - 0 masterSlides, 1 slides, 2 activeSlide, 3 activeSurface: objref
  - 4 artboardSize:vector2i
  - 5 backgroundDisplayMode:enum
  - 6 backgroundOpacity, 7 backgroundFill, 8 backgroundBrightness: float
  - 11 artboardRotation:enum (ROTATE_0…)
  - 12 artboardFlipHorizontal, 13 artboardFlipVertical: bool
  - 14 globalEffectEdgeThreshold:float
- **master-slide** (317a8cf7): 0 surfaces:objref
- **slide** (dbdecc47):
  - 0 masterSlide:objref
  - 1 duration:timeline-duration
  - 2 outOfBoundsBehavior:enum
  - 3 surfaces:objref
  - 4 effect:objref
  - 6 publishingEnabled, 7 fadeIn, 8 fadeOut: bool
  - 9 fadeInDuration, 10 fadeOutDuration: duration [I]
  - 11–13: string ×3 [I]. Candidates are thumbnailUrl and audioReactivitySettingsString; the third is [U].
  - 14 skipTranscodeOnPublish:bool [I]
  - 15 category:string [I]
- **master-surface** (8a71f1e2):
  - 0 geometries:objref
  - 1 structure:objref
  - 2 locked, 3 visibleInCanvas: bool
  - 4 geometryFeatherWidth, 5 geometryEdgeOffset: float
  - 6 position:vector2, 7 rotation:angle, 8 scale:vector2
  - 9–10: bool [I]. Candidates are autoPerspectiveFit and autoPathNamingEnabled.
- **surface** (e2ac8a98, the per-slide instance of a master-surface):
  - 0 masterSurface, 1 contels, 2 effect: objref
  - 4 opacity:float
  - 5 blendMode:enum (NORMAL, OVERLAY…)
  - 6 geometryFillColor, 7 geometryBorderColor: color4f
  - 8 geometryBorderWidth:float
  - 11 contentEffects:objref
  - 12 audioReactive:bool [I]
  - 13 bool [U]
- **structure** (a799e093, the warp grid):
  - 0 gridSize:vector2i
  - 1 points, 2 controlPoints, 3 envelope, 4 uneditedEnvelope: vector2
  - 11 size:vector2
  - 12 hasBeenCornerPinned, 13 hasLinearGrid: bool
- **path** (b22f7da8): 0 points, 1 controlPoints: vector2; 2 isHole, 3 isClosed: bool; 8 enum [I], probably booleanOp (UNION/SUBTRACTION).
- **composite-path** (77fb4305): 0 paths:objref; 1 invertComposite:bool
- **contel** (d2644d83, a content element: media, text or effect layer):
  - 0 mediaAsset, 1 effect: objref
  - 2 text, 3 fontFamily: string
  - 4 bold, 5 italic: bool
  - 6 fontSizePixels:int64
  - 7 textColor:color4f
  - 8 horizontalAlignment, 9 verticalAlignment: enum
  - 10 size:vector2
  - 11 transformSpace:enum
  - 12 position:vector2, 13 rotation:angle, 14 scale:vector2
  - 15 fillColor, 16 borderColor: color4f
  - 17 borderWidth, 18 borderFeather: float
  - 19 textImageAsset:objref
  - 20 contelType:enum (VIDEO, EFFECT…)
  - 21 videoSpeed:float
  - 22 videoTrimStart, 23 videoTrimEnd: timeline-time
  - 24 videoDuration:duration
  - 25 visibleInCanvas, 26 locked: bool
  - 28 opacity:float
  - 29 blendMode:enum
  - 30 inheritAlpha:bool
  - 31 contentEffects:objref
  - 33 projection:matrix3 [I]
- **effect** (00076275): 0 effectAsset:objref; 1 shaderProperties:property-array
- **asset** (2c78a4b8):
  - 0 externalAssetReferenceUrl:url
  - 1 displayName:string
  - 2 scanImageID:enum
  - 3 publishedPresence:enum (LIVE / PRERENDERED)
- **scan-session** (f410ae56):
  - 0 scanImageAssets:objref
  - 1 timestamp:date-time
  - 2 deviceScanID:uuid
  - 3 projectorDimensions:vector2i (1280×720 observed)
  - 4 disparityValid:bool
  - 5 deviceName:string
  - 6 deviceRemote:bool [I]

Ownership and containment **[O]**:

- Project → Canvas → (MasterSlides, Slides).
- A Slide's surfaces are Surface instances, each pointing to a MasterSurface.
- A MasterSurface owns its geometries (Path / CompositePath) and Structure.
- A Surface owns its Contels, and each Contel has an Effect → Asset.
- Assets are owned by the Project (referencedAssets) or are roots.

## 4. Assets and asset libraries

`Asset.externalAssetReferenceUrl` uses three schemes **[O]**:

- `localassetlibrary:///<Vendor>/<Library>/<version>/<path>`, for example `localassetlibrary:///Lightform/Built_In/3.0.0/effects/gradient/gradient.liteffect`. This resolves to the bundled `Contents/Resources/Lightform-Built_In-3_0_0.zip`: `<Vendor>-<Library>-<ver with _>.zip`, then `<path>` inside the zip **[I]** (name mapping inferred; the paths exist in the zips **[O]**).
- `relativefile:///scans/camera-color.jpg`: relative to the project folder.
- `file:///abs/path.mp4`: absolute user media, not copied in the samples. These are machine-specific and break on other hosts **[O]**.
- An empty string means no asset.

Library zips **[O]**:

- Each zip has a root `manifest.json`: `{"versionNumber":0, "name":"Lightform-Built_In-3_0_0", "assetLibraryReferences":["Lightform-Built_In-1_0_0", …]}`. Library 1 has no manifest.
- Contents are `effects/<id>/<id>.liteffect` (ISF-style JSON: CREDIT, DESCRIPTION, CATEGORIES, INPUTS[NAME, TYPE, DEFAULT, MIN, MAX]), `<id>.fs` (GLSL), and `<id>.liteffect.gif`, plus `images/` and `videos/`.
- Library 4.0.0 holds only `.reference` stubs: `{"versionNumber":0, "assetLibrary":"Lightform-Built_In-1_0_0", "assetPath":"images/grid.png"}`. These are redirects into older libraries.
- Effect parameters chosen in a project are stored as dynamic Properties under Effect.shaderProperties. Each one copies the `.liteffect` INPUT metadata into its dynamic-description, and its ValueController holds the value **[O]**.

## 5. Versioning and migration

- **[O]** Two independent version axes:
  - container `format-version` / `formatVersion`: 1 in both containers.
  - schema `data-version-number`: 9 in the 2020 JSON projects, 23 in the 2022 binary projects. `original-data-version-number` records the version a document was created at.
- **[I]** The app migrates on load. Evidence:
  - The string "Project structure data migration" (help URL in urls.json).
  - Class names with obsolete/retired fields: `obsoleteHighlighted`, `obsoleteColor`, `retiredShaderProperties`, `obsoleteGeometryFeatherWidth`, `obsoleteGeometryEdgeOffset`.
  - The `AnyObjectObsoleteStatus` enum.
- **[O]** Known changes from v9 to v23:
  - The writer switched to the binary container.
  - Static Property objects stopped being serialized.
  - New slots were added (listed in §3b).
- **[O]** Local-ids are stable across versions: 0 type conflicts.
- **[U]** Exact per-version migration steps, whether v23 can still write JSON (`DataChunkWriterStyle` enum suggests a choice [I]), and what `max-bytes-per-line` controls. The last is probably JSON line wrapping; the binary file stores 0xFFFFFFFF.

## 6. Implications for a Rust port

- Write one generic chunk codec with two backends (binary and lenient JSON), a UUID-interning table, and the `h*33+c` key hash.
- Model documents as an `AnyObject` graph: id, class, owner, state, references. Map class UUIDs to typed structs through the slot tables in §3b.
- Writing v23 binary requires the class-UUID → (localId, controller type) schema. §3b plus `lit_class_registry.json` covers every class in the samples.
- Unknown slots should round-trip verbatim: keep the raw state and references for unrecognised localIds.
- Asset resolution needs a library resolver for `localassetlibrary` URLs that follows `.reference` redirects, plus a project-relative resolver for `relativefile`.
