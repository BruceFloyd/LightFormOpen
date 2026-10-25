# Lightform Creator 2.2.12 — UI structure, feature set, backend API contract, Slint port assessment

Worker D. Static reading only. Paths are relative to `re/qrc/resources/src/qml/` unless prefixed.
Strings marked *(bin)* come from `strings` on `Lightform.app/Contents/MacOS/Lightform` or
`Frameworks/libLightformEngine.1.0.0.dylib`, not from QML.

## 1. Shape of the app

- **Host is a native Qt Widgets `MainWindow`, not a QML `ApplicationWindow`.** *(bin)* shows `MainWindow`, `centralWidget`, `menuBar`, `statusBar`, `QtNavigatedPage` and `pageChanged(QWidget*, QtNavigatedPage*)`. Each top-level QML file is loaded by its own C++ class, for example `src/qml/DesignPage.qml` next to `DesignPage`, or `src/qml/EffectEditor.qml` next to `EffectEditor::onSaveFiles`. That means the C++ side owns QQuickWidget hosting, dialogs and windows.
- **Native menu bar** *(bin, MainWindow strings)*:
  - **File:** New / Open / Open Recent / Save / Increment & Save / Archive / Close Project, Export Scan, Export Selected Surface Mask, Export Effect Images, Publish From Local Folder, Show Project in File Explorer, Quit.
  - **Edit:** Undo/Redo "%1", Preferences, Check Project.
  - **View:** Show Design UI, Show Pixel Grid, Show Projection Preview, Enable Design Snapping, Notifications, Effect Editor Window, Scripting Window, Scan Properties, New Scan.
  - **Labs:** Enable Labs.
  - **Developer:** RenderDoc capture, Dump Render Graph, GPU Memory Window, Show/Create/Remove Orphaned Objects, Show Dirty Count, Generate Scan Images, Set Scan/Disparity Image, Render Pick Buffer, Clear Undo Stack, Force App Crash (two variants), Reset Analytics Opt In/Out.
  - **Help:** About, Support Articles, Keyboard Shortcuts, Copy Shortcuts to Clipboard, Submit Bug Report, Submit Feedback, Update Application, Log In / Log Out / My Account.
- **Every page receives one `application` object.** `components/ApplicationCommonsItem.qml` (the base of `ApplicationPage.qml`) derives the whole backend graph from it (§4.1).
- **QML size.** 353 qrc files and about 37k QML lines. About 34k are real UI, excluding `*DynamicSizes.qml` (2,074 lines), designer mocks (`designer/`) and the samples (`controls/custom-panels/Sample*`, `*.ui.qml`). There are 374 JS functions plus `functions/Utilities.js` (300 lines).
- **Qt3D isn't used from QML.** No `Scene3D` or `Scene2D` appears anywhere. The bundled Qt3D frameworks are either used from C++ or are only deployment baggage; that's for another worker to resolve.

## 2. Navigation map

```
launch
 ├─ StartupErrorPage.qml            (fatal init error; ApplicationEventType.AppStartupErrorPageRequested)
 ├─ LoginPage.qml                   (AppLoginPageRequested; states stateLogin→EnterCredentials→LoggingIn→LoginFailed→stateWelcome;
 │                                   "Skip" → qml_loginSkipped; Sign Up / Forgot Password open web pages)
 └─ WelcomePage.qml                 (cards: New Project / Open Project / Control Device; "What's New?";
      │                              "Register Device on Lightform.com" if lightformCloudEnabled)
      ├─ NEW_PROJECT → SelectProjectDirectoryPage.qml (name, folder, type Standard | Auto-Align Object [+reference image],
      │                                                "Perform New Scan to Initialize Project") → DesignPage
      ├─ OPEN_PROJECT → file dialog (.lit) → DesignPage
      └─ CONTROL_DEVICE → ControlPage
DesignPage.qml  ⇄  ControlPage.qml   (PageTabBar.qml "Design" / "Control"; GO_TO_DESIGN_PAGE / GO_TO_CONTROL_PAGE)
```

**Pages and dialogs owned by MainWindow:**
- `AboutPage.qml`
- `SubmitBugReportPage.qml` (bug report or feedback, with screenshot upload)
- `SelectArchiveProjectDirectoryPage.qml` (archive to a directory or a ZIP)
- `ApplicationSettingsDialog.qml`
- `ApplicationStatusDialog.qml` (App and Project message logs)
- `CheckProjectDialog.qml` ("Repair Project")
- `UnsavedChangesDialog.qml` (Save & Close / Don't Save / Cancel)
- `YesNoDialog.qml`, `ProgressDialog.qml`, `BigTextDialog.qml` (copy to clipboard)
- `EnableLabsDialog.qml`
- `ActionManagerKeyCombinationsDialog.qml` (shortcut list)
- `SelectDeviceDialog.qml`, which wraps `SelectDeviceWorkflow.qml`
- `ScanDialog.qml`, which wraps `ScanWorkflow.qml`

**Separate windows:**
- `EffectEditor.qml`
- `ScriptingDialog.qml`
- `ServiceAssetsCatalogWindow.qml`
- `CameraStreamWindow.qml`
- `GPUMemoryProfilerWindow.qml`
- `PixelDensityWindow.qml`

**Drawers inside pages:**
- `PublishWorkflow.qml` (DesignPage, through `WorkflowDimmingDrawer`)
- `FirmwareUpdateWorkflow.qml`
- `PairingWorkflow.qml` (from `SelectDeviceWorkflow`)
- `AssetBrowserPanel.qml`
- `NotificationsPanel.qml`

**Workflow state machines (QtQml.StateMachine).** All of this logic should move into Rust.

| Workflow | States |
|---|---|
| `SelectDeviceWorkflow.qml` | selectDevice → pair → accepted / cancel |
| `PairingWorkflow.qml` | waitingForDevice → waitingForInput → attemptingToPair → complete / cancel |
| `ScanWorkflow.qml` | selectDevice → prepareScan → scanRunning → error / cancel |
| `ScanDialog.qml` | start → scanning → manageDevices → scanFinished |
| `FirmwareUpdateWorkflow.qml` | checkForUpdates → approveUpdate → applyUpdate → succeeded / failed |
| `PublishWorkflow.qml` | prePublish → createPackage → uploading → failed / canceled / success |
| `LoginPage.qml` | see the navigation map above |
| `components/DesignPageToolBar.qml` | default / pixelTool / magicWand / magicBrush / brush / vectorize toolbar modes |

## 3. Feature inventory by area

### 3.1 Project management
- **New project.** Name, directory and project type (`SelectProjectDirectoryPage.qml`):
  - **Standard:** "Take full control over the scanning and content creation process."
  - **Auto-Align Object:** "Automatically detect a flat object based on a picture of the object." Requires an object image. Behind it is `activeProject.isAutoAlignObjectProject`. It can't be published to LF1 or to remote devices (`DesignPage.qml` `validateDeviceForPublish`).
- **Lifecycle.** Open, recent projects, save, increment & save, close, autosave (`onAutosaveTimeout` *(bin)*), and archive to a folder or ZIP (`SelectArchiveProjectDirectoryPage.qml`).
- **Maintenance.** Check/Repair Project (`CheckProjectDialog.qml`) and the orphaned-object tools (Developer menu).
- **Project properties panel** (`custom-panels/lightform.object.project.qml`):
  - Description and project type
  - Scan properties: "Last scan taken with %1 at %2x%3 pixels on %4", Export Scans, Generate scan images
  - Background display: Color / Disparity / Effect Edges, with Brightness and Opacity
  - Hole filling
  - Rotate & Flip artboard: `canvas.qml_artboardRotateLeft/Right` plus the undo labels "Set Artboard Horizontal/Vertical Flip" *(bin)*
- **Undo/redo everywhere.** Undo labels *(bin)* include Set Background Display Mode/Opacity/Fill/Brightness, Set Edge Threshold, Add/Move/Delete/Duplicate Slide, Set Slide Name, Enable/Disable Slide Publishing/Looping, Reorder Paths, Split Path To New Surface, and more.

### 3.2 Surfaces and mapping tools (DesignPage canvas)
- **Canvas.** A C++ `GraphicsQuickControl` (`DesignPage.qml:547`, `objectName: "canvasGraphics"`) renders the artboard, the scan background and all overlays. Zoom comes from `DesignPagePlaybackBar.qml` (`zoomChangeRequested`). Optional overlays: pixel grid, projection preview and snapping (View menu).
- **Tools.** `ToolManager` enum, from `components/DesignPageDefaultToolRow.qml` and *(bin)* "Design Page Action":

  | Tool | Shortcut action |
  |---|---|
  | Select | SET_SELECT_TOOL |
  | Move | SET_MOVE_TOOL |
  | Pen (polygon/path) | SET_PEN_TOOL |
  | Rectangle | SET_RECTANGLE_TOOL |
  | Ellipse | SET_ELLIPSE_TOOL |
  | Text | SET_TEXT_TOOL |
  | Perspective Warp | SET_PERSPECTIVE_WARP_TOOL |
  | Brush | SET_BRUSH_TOOL |
  | Magic Wand | SET_MAGIC_WAND_TOOL |
  | Magic Brush | SET_MAGIC_BRUSH_TOOL |

  Cursor modes *(bin)*: Move, Resize, Rotate, PenTool(-ClosePath/-Move), BendTool, StructureTool(-Move/-Rotate), MagicWand-Replace/Add/Subtract.
- **Pixel tools** (Brush, Magic Wand, Magic Brush). They work on the scan image:
  - Color or Disparity mode (`ToolManager.COLOR` / `DEPTH`), and Replace/Add/Subtract.
  - Brush size and wand tolerance; "Connected" flood fill vs ALL_PIXELS.
  - Vectorize the mask into a path, with Accuracy and "Modify Edge" grow/shrink.
  - Create Surface, Export Mask.
  - Sources: `components/DesignPageMagicWandToolRow.qml`, `DesignPageBrushToolRow.qml`, `DesignPagePixelToolVectorizationRow.qml`, `DesignPagePixelToolButtons.qml`.
  - Backend members: `toolManager.qml_pixelModeVectorize`, `qml_pixelModeGrowShrink`, `qml_pixelModeAcceptObject`, `qml_saveCurrentMask`, `pixelToolBrushSizePixels`, `pixelToolMagicWandThreshold`, `pixelToolCurveAccuracy`.
- **Surfaces** (`custom-panels/lightform.object.surface.qml`):
  - X/Y/W/H, opacity and blend mode
  - Paths list with boolean ops Union / Subtract / Intersect / Exclude, evaluated bottom to top (`components/GeometryList.qml`, `PathBooleanOperation`)
  - Per path: Duplicate / Split To New Surface, Delete
  - Feathering, Mask, Stroke, Offset
  - Structure: make planar / reset / resize (`uiMakeStructurePlanarRequested` etc. in `DesignPage.qml`)
  - "Auto-Fit New Content", "Auto-Name Paths"
  - Content Styles: Color Fill, Color Stroke, Hue/Saturation, Brightness/Contrast
- **Layers panel** (`components/DesignPageLayersPanel.qml` + `controls/core/ReorderableTreeView.qml`):
  - Tree of surfaces and the content inside them
  - Drag reorder via `treeModel.qml_moveNode`
  - Per row: visible/hidden, locked/unlocked, audio-reactive toggle (`components/LayersPanelRowDelegate.qml`)
  - Arrange: Move to front/back/forward/backward
  - Merge or split selected surfaces
  - Duplicate content forward / back / to all slides
- **Context menu** (`components/DesignPageContextMenu.qml`): Add Surface, Pen Tool, Brush, Magic Wand, plus the actions supplied by the C++ side.

### 3.3 Content ("contels") on surfaces
`custom-panels/lightform.object.contel.qml`, 1,134 lines:
- **Content types:** media (image/video), effect, text.
- **Transform:** position, size, scale, rotation and a transform space ("Contel Space").
- **Perspective:** Fit to Surface, Fit Perspective to Surface (4-point surfaces only), Reset Perspective.
- **Compositing:** opacity, blend mode, alpha inherited or not.
- **Text:** font family, pixel size, colour, horizontal alignment (`components/FontSelector.qml`, `PropertyFont.qml`).
- **Effect parameters:** edited through the dynamic properties panel.
- **Effect presets:** built-in presets, Apply Built-in Preset, Save Current as Preset / Load Preset from File (`*.litpreset` *(bin)*), and Promote to Live Effect.
- **Content Styles stack:** same list as surfaces.
- **"Match slide duration to selected contel duration"** (`DesignPage.qml` signal).

### 3.4 Slides and timeline
- **Slides bar** (`components/DesignPageSlidesBar.qml`). Each slide thumbnail is a live renderer: `GraphicsQuickControl` with `rendererFactory: slideViewFactoryProvider.newSlideViewFactory(...)`. Per slide: add, duplicate, delete, rename, drag reorder, publishing on/off, loop when published, and Publish this slide.
- **Playback bar** (`components/DesignPagePlaybackBar.qml`). `canvas.playbackMode` takes Play / Pause / Stop. Also playback time and percent of slide duration, `resetPlaybackTime`, and zoom.
- **Slide properties** (`custom-panels/lightform.object.slide.qml`):
  - Duration and loop slide
  - Fade In/Out with durations
  - Out-of-bounds behaviour
  - Category, description, thumbnail
  - Skip Transcoding
  - Audio reactivity settings (stored as a JSON string, `audioReactivitySettingsString`)

### 3.5 Effects and the Effect Editor
- **Effects** are `.liteffect` JSON (ISF-like: INPUTS / PRESETS / CATEGORIES) plus `.fs` GLSL. The built-in spec text *(bin, engine)* covers:
  - Input types: boolean, long, float, color, point2D, point3D, image
  - Built-in constants and functions: `RENDERSIZE`, `TIME`, `SLIDE_LENGTH`, `IMG_NORM_PIXEL`, colour-space helpers, `perlin_noise*`, …
  - Special image sources such as `lightform:scan_edge_distance_trace`
- **Effect Editor window** (`EffectEditor.qml`):
  - Two code panes, `.liteffect` and `.fs`, with syntax highlighting (`LitEffectSyntaxHighlighterHelper` and `ShaderSyntaxHighlighterHelper` *(bin)*)
  - New / Open / Save / Save All / Reload / Prettify, with file watching
  - Save methods: Direct, Temp + Copy, Delete + Direct, Delete + Temp + Copy, Unwatch + Watch
  - Tabs: Specification, Shader, Documentation, Help
- **Developer:** "Export Effect Images".

### 3.6 Assets and libraries
- **Asset browser drawer** (`components/AssetBrowserPanel.qml`, `AssetBrowserPage.qml`, `AssetBrowserAssetElementDelegate.qml`):
  - Tabs: Project Assets, Built-In Assets, Stock Assets
  - Groups and alphabetical sort
  - Drag files onto the window to import (`DesignPage.qml` `DropArea` → `assetBrowser.importFiles`)
  - Animated thumbnails via `SequencedImage`
  - Per asset: Insert Content, Insert as New Surface, Remove From Project
  - Shows `%1fps` and `%1x%2` metadata
- **Stock assets (Storyblocks service)** (`ServiceAssetsCatalogWindow.qml`, `components/ServiceAssets*GridView.qml`):
  - Search catalog, Download, Preview In External Browser
  - Downloads list
  - "Check Project For Missing Stock Assets" / "Download Missing Stock Assets"
  - *(bin)* `onStoryblocksErrorMessage`, `StoryblocksAssetLibraryListModel` (comment in `DesignPage.qml:43`)
  - **The service is almost certainly dead.**
- **Built-in libraries.** `Resources/Lightform-Built_In-{1..4}_0_0.zip` hold effects, images and videos plus `.reference` indirections (`components/AssetBrowserPanel.qml` "Built-In Assets").

### 3.7 Device control (ControlPage + DesignPage device panel)
- **Device selection** (`SelectDeviceWorkflow.qml`, `components/DeviceListView.qml`):
  - Two tabs: Local Devices (mDNS/LAN discovery) and Cloud Devices (account-linked, "remote")
  - Add Static IP, troubleshooting text, "Register Device on Lightform.com"
- **Pairing** (`PairingWorkflow.qml`, 1,324 lines). Guided onboarding to Wi-Fi:
  - Device type picker: LF1 / LF2 / LF2+ / LFC
  - Connect the computer to the device's own Wi-Fi network
  - Choose an SSID or enter one manually, with security type None/… and a password
  - Instructions for entering pairing mode per device type
- **ControlPage** (`ControlPage.qml`, `ControlPageDevicePanel.qml`):
  - **Device info:** hostname, serial, part number, manufacture date, lens type, firmware type and version, storage, location (editable).
  - **Display:** resolution list (Set Resolution), HDMI-detected warning, projection mode Front / Rear / Front Ceiling / Rear Ceiling, brightness.
  - **Projector controls:** focus + / −, test card on/off, standby on/off, restart.
  - **Pairing mode:** "Enter Pairing Mode" forces it.
  - **Wi-Fi:** saved and visible networks, add network, forget network.
  - **Updates:** Check for Firmware Updates → `FirmwareUpdateWorkflow.qml`.
  - **Camera stream:** "Show Camera Stream" (`CameraStreamWindow.qml`, peek stream URL).
  - **Live input:** "Streaming Source (NDI)" (`livelight*`).
- **Playback control of the published project on the device:**
  - Slides list with the current slide, next/previous/set index, play/pause/stop, global brightness
  - Per-slide Live Effect properties and loop (`components/ControlPageSlidesListElementDelegate.qml`)
  - "Control on Lightform.com" for cloud devices
- **ControlPage sub-tabs** (`ControlPage.PageIndex`: Slides / Osc / AudioReactivity / ManualAlignment):
  - **OSC** (`ControlPageOSCSettingsWindow.qml`): per-parameter OSC address with "Learn" from the next incoming OSC message.
  - **Audio reactivity** (`ControlPageAudioReactivitySettingsWindow.qml`, 909 lines): enabled flag, gain/speed filters per band (Low, Mid-Low, Mid-High, High), attack/release, intensity, calibrate sound levels, test sound bars, copy settings to/from a slide.
  - **Manual alignment** (`ControlPagePerspectiveWarpWindow.qml`): 4-corner keystone drawn on a `Canvas`, with point nudge (`canvas.nudgeSelectedPoint`, `findPointAt`). For auto-align projects it drives `oaklightSetCustomObject` / `oaklightOnObjectAlignment` / `oaklightOff`. `ControlPageContentRealignmentWindow.qml` adds "Subdivisions".
- **Publish** (`PublishWorkflow.qml`, launched from `components/DesignPageToolBar.qml`):
  - Targets: Publish to device, Publish To Local Folder, Publish From Local Folder
  - Steps: Rendering project → (Publishing to local folder | Uploading to device) → success / error / retry
  - Only slides with publishing enabled are sent (`canvas.qml_getSlideIndexesWithPublishingEnabled`)
- **Streaming preview** (`components/StreamWidget.qml`, "Preview" toggle `streamManager.isStreaming`, TOGGLE_STREAMING). It streams the live canvas to the device and is turned off automatically when publishing (`DesignPage.qml:642`).

### 3.8 Scanning
- **Scan workflow** (`ScanWorkflow.qml`, 1,260 lines; `components/DesignPageScanWidget.qml`; `DesignPageDevicePanel.qml` "Perform a scan to initialize the project"):
  - Scan type: Full or Planar
  - Resolution; HDR off / 2 / 3 exposures; smoothing; noise reduction; Safe Mode for high-latency projectors
  - Camera mask (region of interest, `controls/ImageRegionOfInterestSelector.qml`); focus + / −
  - Object UUID (optional)
  - Labs: Analyze Environment / Analyze Objects
  - Download Previous Scan; Save Debug Images; error and retry
- **Scan results feed into the design page:** the scan colour image and the disparity image become the canvas background and the pixel-tool sources.

### 3.9 Scripting
- **`ScriptingDialog.qml`:**
  - Pick a directory of **C# `*.cs` unit-test or utility scripts**
  - Recompile / Run / Recompile & Run / Stop
  - Repeat N times or forever every X ms
  - Pass/fail/ignored totals
- This implies an embedded Mono/.NET runtime with a C# API over the object model. *(bin)* also contains Mono strings. **Out of scope here, but significant for the port:** it's either dropped, or replaced with Rhai or Lua scripting.

### 3.10 Settings, notifications, account, labs
- **Preferences** (`ApplicationSettingsDialog.qml`):
  - Pages: Application, User Interface, Artboard, Stream Preview, Support (`PageIndex.*`)
  - User Interface Sizing (`PixelDensityWindow.qml`: Low/Default/High)
  - Show Cursor, Show Masks
  - Playback Renderer Target Frame Rate
  - Video frame memory/file cache sizes, max frame skip, "Load and Decompress Imported Video Files From Memory"
  - Enable NDI Output
  - Data and Temporary directories, file size display
  - Send Analytics
- **Notifications** (`components/NotificationsPanel.qml`): Update Application, Low Memory / Storage / Graphics Hardware warnings, Analytics opt-in, Project messages. Backend: `application.hasInsufficientPhysicalMemory`, etc.
- **Account** (`LoginPage.qml`): email/password login to Lightform services, sessions stored encrypted on disk *(bin: LoginSession.cpp)*, `loginManager.isInternalUser` gates internal features. **Dead service; replace with no-account local mode.**
- **Labs** (`EnableLabsDialog.qml`): `applicationSettings.labsFeaturesEnabled` shows a yellow indicator line and unlocks a few features (`DesignPage.qml:275`, `ControlPage.qml:241`, `components/PageTabBar.qml:76`, `ScanWorkflow.qml`). Most labs gating in the QML is commented out. `developerFeaturesEnabled` gates internal contel options (`custom-panels/lightform.object.contel.qml:443,756-789`).
- **Bug report / feedback** (`SubmitBugReportPage.qml`): posts to a Lightform endpoint. Dead service.

### 3.11 Keyboard shortcuts / ActionManager
- **Action managers.** Actions are typed enums with UUIDs, defined in the engine *(bin, engine)*. Each manager is exposed as `applicationSettings.{global,welcomePage,loginPage,designPage,controlPage}ActionManager`. QML triggers actions only by name: `actionForEnumValueName("X").triggerAction()` / `triggerActionWithParameters({...})`. `globalActionManager.handleKeyDownAndModifiers` routes keys.
- **Enumerations:**

  | Enum | Values |
  |---|---|
  | Global Action | NEW/OPEN/SAVE/INCREMENT_AND_SAVE/ARCHIVE/CLOSE_PROJECT, QUIT, GO_TO_DESIGN/CONTROL_PAGE, PIXEL_GRID_VISIBLE, PROJECTION_PREVIEW_ENABLED, DESIGN_SNAPPING_ENABLED, EXPORT_*, PUBLISH_FROM_DIRECTORY, SHOW_KEYBOARD_SHORTCUTS, SHOW_SCRIPTING_DIALOG, SHOW_EFFECT_EDITOR, CHANGE_PROJECT_DEVICE, EXPLORE_FILE_SYSTEM_PATH, NOTIFY, … (~70) |
  | Design Page Action | ESCAPE_ACTIVE_ACTION, TOGGLE_STREAMING, BROWSE_ASSETS, SET_*_TOOL, MOVE_TO_FRONT/BACK/FORWARD/BACKWARD, SET_SELECTED_EFFECTS_TO_LIVE, SET_PIXEL_TOOL_MODE_COLOR/DISPARITY_GRAY, TOGGLE_LOCK/VISIBILITY/AUDIO_REACTIVITY, RENAME, NEXT/PREVIOUS_SLIDE, SELECT_NEXT/PREVIOUS_LAYER, NEW_SCAN, ASSET_SERVICES_*, DUPLICATE_CONTEL_*, DUPLICATE, MERGE/SPLIT_SELECTED_SURFACES |
  | Creator Edit Tool Action | CLEAR_SELECTION, MOVE_{LEFT,RIGHT,UP,DOWN}_{NUDGE,STEP}, REMOVE_SELECTED_OBJECTS |
  | Pixel Tool Action | IN/DECREASE_BRUSH_SIZE, _WAND_THRESHOLD, _FIT_TOLERANCE |
  | Create Project Geometry Action | CANCEL/COMMIT_CREATION |
  | Control Page Action | SELECT_DEVICE, SHOW_CAMERA_STREAM |
  | Welcome Page Action | CONTROL_DEVICE, PAIR_DEVICE (+ NEW/OPEN_PROJECT) |
  | Login Page Action | ACCOUNT_SIGNUP, RELEASE_NOTES_NEW_FEATURES, SHOW_FORGOT_PASSWORD_SUPPORT_WEB_PAGE |
  | Remote project actions (OSC/remote) | PROJECT_NEXT_SLIDE / PREV / SET_SLIDE / PLAY / STOP / PAUSE / SET_GLOBAL_BRIGHTNESS / SET_SLIDE_NORMALIZED |

- **Default key bindings are not in any string table.** Engine strings show `KeyCombinationObject`, `keyCombinations`, `retiredKeyCombinations` and `ReadOnlyKeyCombinations`, but no "Ctrl+…" literals. The defaults are built in code. **Unknown:** recover them from the shortcuts dialog at runtime ("Copy Keyboard Shortcuts to Clipboard") or by disassembly.

## 4. Backend API contract (what QML calls on C++)

Registered QML modules *(bin)*: `com.lightform.controls`, `com.lightform.data`, `com.lightform.application`, `com.lightform.utility`, `com.lightform.device.client`. The counts below are QML use sites (static grep). This is the minimum surface a Rust backend must provide if the UI is ported faithfully.

### 4.1 Root graph (`components/ApplicationCommonsItem.qml`)

**`application`:**
- **Roots:** `qml_applicationModel`, `qml_applicationEventFactory`, `qml_typeDescriptionDatabase`, `qml_loginManager`, `qml_loginSkipped`, `qml_firmwareUpdateManager`, `qml_publishManager`, `qml_deviceManager`, `qml_isAnalyticsReportingAvailable`, `qml_statusItemsModel`, `globalSequencedImageProvider`
- **Health warnings:** `hasInsufficientPhysicalMemory` / `physicalMemoryWarningMessage` / `checkPhysicalMemory()`, the same trio for storage capacity, `hasGraphicsHardwareWarning` / `graphicsHardwareWarningMessage`
- **Other:** `layoutDirection`

**`applicationModel`:** `activeProject`, `applicationSettings`, `url(key)` (URL indirection via `resources/urls/urls.json`)

**`applicationSettings`:**
- **Flags:** `labsFeaturesEnabled`, `developerFeaturesEnabled`, `lightformCloudEnabled`, `stockAssetsEnabled`
- **Analytics:** `analyticsReportingOptedInOrOut`, `isAnalyticsReportingEnabled`, `isUserAllowedToToggleAnalytics`
- **Updates:** `applicationUpdateVersionAvailable`, `applicationUpdateButtonText`
- **Version info:** `versionString`, `versionAndBuildNameDisplayString`, `releaseDate`, `releaseChannel`
- **Remembered paths:** `lastPublishedProjectDirectoryPath`, `lastAssetDirectory`
- **Other:** `simulatedDeviceName`, `editorSettings`, `{global,welcomePage,loginPage,designPage,controlPage}ActionManager`

**Action manager:** `actionForEnumValueName(name)` → action with `triggerAction()` / `triggerActionWithParameters(map)`; `handleKeyDownAndModifiers`.

**`applicationEventFactory`:**
- **Methods:** `createEvent(type)`, `createVariantEvent(type, v)`, `createToastEvent(...)`. *(bin)* adds createEnableEvent, Index/Id/StringId selection events and createBugReportSubmittedEvent.
- **Event types** (`ApplicationEventType`): UIMainDeviceChanged, UIControlPageDeviceChanged, App{Login,Welcome,StartupError}PageRequested, AppInitialized, AppUpdateAvailable, AppOpen/Close/ClosingProject, AppPrepareFor/PrepareNewProject, AppProjectReady, AppNewProject, AppPublishStarting/Ending, AppCloseAndUpdate, AppClosingMainWindow, AppAddAnalyticsEvent, AppToastMessage, UIStart/FinishWiFiPairingDeviceDiscovery.

**`typeDescriptionDatabase`:** `enumDescriptionDatabase` (enum value lists for combo boxes).

**Account, firmware and publish managers:**
- **`loginManager`:** `isLoggedIn`, `isLoginEnabled`, `isInternalUser`, `currentSession`, `userEmail`, `login()`, `lastLoginErrorMessage`
- **`firmwareUpdateManager`:** `setDeviceClient`, `createFirmwareUpdateObject` → object with `needsUpdate`, `availableVersion`, `progress`, `doUpdate()`, `cancel()`, `getUiMessageForStatus()`, `deviceClient`
- **`publishManager`:** `start`, `cancel`, `progress`, signals `uploadStarting`, `uploadFinished`

### 4.2 Project / canvas / editing
- **`activeProject`:** `id`, `projectID`, `projectName`, `canvas`, `isAutoAlignObjectProject`, `activeScanSession`, `qml_createOrGetActiveScanSession`, `scansDirectory`, `projectorResolutionMatchesProjectCanvasDimensions`
- **`canvas`:**
  - **Playback:** `playbackMode` (CanvasPlaybackMode Play/Pause/Stop), `playbackTime`, `playbackTimeAsPercentageOfSlideDuration`, `resetPlaybackTime`
  - **Slides:** `activeSlide`, `activeSlideIndex`, `slidesModel` (`qml_get(i)`), `slideCount`, `hasPublishableSlide`, `qml_addNewSlide`, `qml_duplicateSlide`, `qml_removeSlide`, `qml_moveSlide`, `qml_setSlideName`, `qml_setSlideLoopingEnabled`, `qml_setSlidePublishingEnabled`, `qml_getSlideIndexesWithPublishingEnabled`, `qml_get/setSlideAudioReactivitySettings`
  - **Artboard:** `width`, `height`, `backgroundDisplayMode`, `qml_artboardRotateLeft/Right`
  - **Property access:** `subProperties`, `propertyBag`
  - **Warp points:** `normalizedPoints`, `setNormalizedPointsCopy`, `setDefaultPoints`, `findPointAt`, `selectedPointIndex`, `hoveredPointIndex`, `moveSelectedPoint`, `nudgeSelectedPoint`
- **`toolManager`:**
  - **Active tool:** `activeTool` (ToolManager enum), `qml_requestActiveTool`, `qml_requestSimpleToolIfNecessary`
  - **Pixel-tool settings:** `pixelToolMode`, `pixelToolColorMode`, `pixelToolWandMode`, `pixelToolBrushSizePixels`, `pixelToolMagicWandThreshold`, `pixelToolCurveAccuracy`
  - **Pixel-tool operations:** `qml_pixelModeVectorize/GrowShrink/CancelVectorization/AcceptObject`, `qml_saveCurrentMask`
  - **Status:** `hasScanImage`, `isVectorizationValid`, `vectorizationMessage`, `singlePerspectiveSelected`, signals `selectionDataReady`, `pathAccepted`, `maskVectorized`
- **`selectionSet`:** `selectedObjects`, `qml_setToSingleItem`, `qml_add`, `qml_remove`, `qml_contains`, `qml_clear`, `qml_forceAnyObjectSelectionSetChangedSignal`
- **`treeModel`** (layers): `qml_rootNode`, `qml_lastVisibleNode`, `qml_moveNode`. Node interface *(bin)*: Set Content Element Visibility, Lock, Move Item, Delete, Insert, Set Surface Visibility, Lock Surface.
- **Properties system.** This is what drives the whole right-hand panel:
  - `propertiesPanelConnector.propertyBags` → each bag has `propertiesByName`, `propertyGroups` (`name`, `propertySets`), `uiSourceFile`, `addedObjects`, `executeUndoableAction`, `begin/endPropertyChangeAction`
  - **Property adapter:** `value`, `type` / `propertyType`, `controlType` (e.g. HUE_SATURATION), `readOnly`, `isPublic`, `isArray`, `hasProperty`, `bestDisplayName`, `displayPrecision`, `valueRange`, `isValueRangeValid`, `minimum/maximumValueReal/Vector`, `hasNamedValueList`, `valueList`, `namedValuesEnumDescription`, `propertyEnumUuidString`, `openColorDialog`, `begin/endPropertyChangeAction`
  - **Property types** (`controls/PropertyControl.qml:75-137`): OBJECT_REFERENCE, VECTOR2/3/4(I), MATRIX3/4, TRANSFORM2/3, DISTANCE, ANGLE, ANGLE_AXIS, COLOR4F, COLOR4U8, BOOLEAN, UUID, STRING, FILE_SYSTEM_PATH, URL, DATE_TIME, TIMELINE_TIME/DURATION, FLOAT, DOUBLE, INT32, INT64, ENUM
  - **Panel choice.** `uiSourceFile` selects one of `custom-panels/lightform.object.{project,slide,surface,contel}.qml`, or the generic `controls/DynamicPropertiesPanel.qml`.
  - **Named properties the custom panels read:**
    - contel: blendMode, contelType, contentEffects, effect, fontFamily, fontSizePixels, horizontalAlignment, inheritAlpha, masterSurface, opacity, position, rotation, scale, size, text, textColor, transformSpace
    - project: activeScanSession, canvas, projectDescription, projectType
    - slide: audioReactivitySettingsString, category, description, duration, fadeIn/Duration, fadeOut/Duration, outOfBoundsBehavior, skipTranscodeOnPublish, thumbnailUrl
    - surface: blendMode, contentEffects, masterSurface, opacity
- **Asset tabs:**
  - `projectAssetsTabInterface`: `selectItem`, `qml_referenceAssets`, `qml_getIndexObjectForUrl`
  - `libraryAssetsTabInterface`
  - `serviceAssetsLibraryItems`: `totalAssetCount`, `assetCount`
  - `AssetType`: Video / Image / Effect
- **`activeScanSession` / `scanSession`:** `subProperties`, `timestamp`, `updateTimestamp`, `projectorDimensions(String)`, `qml_getScanProjectorColorImageFilePath`, `qml_setScanImageFilePath`, `disparityValid`, `deviceScanId`, `deviceId`, `isDeviceRemote`
- **Utility singletons:**
  - `QmlUtilities`: path/URL helpers (`directoryExists[Url]`, `joinFilePaths[ToUrl]`, `getPlatformLocalFilePathFromUrl`, …), cursor control (`setCursor`, `restoreCursor`, `setCursorScreenCoordinates`, `canSetCursorPosition`), `qml_getNativeShortcutString`, `formatOSCParameterName`
  - `Int64Factory`
- **Registered helper types** *(bin)*: `GraphicsQuickControl` (canvas and slide renderers; `renderingEnabled`, `rendererFactory`), `SequencedImage` + `SequencedImageProvider` (animated thumbnails), `ColorizedSVG`, `Int64Validator`, `EnumValueListModel`, `*DescriptionObject`, the syntax highlighter helpers, `StandardLogChannel`, and enum containers: `SurfaceInterfaceObject` (SceneObjectType SURFACE/CONTEL), `ProjectAssetGroupingOptions`, `ContentEffectType` (Color / Hsv / BrightnessContrast), `PathBooleanOperation`, `CanvasPlaybackMode`.

### 4.3 Devices (`com.lightform.device.client`)
- **`deviceManager`:** `getDeviceClientListModel()`, `getLocal-` / `getRemoteDeviceClientSortFilterListModel()`, `getPairingDeviceListModel()`, `qml_getDeviceClientByIdAndIsRemote`, `addUnknownDeviceByIp`, `pairingWorkflowObject`
- **`pairingWorkflowObject`:** `deviceType`, `deviceName`, `hasDevice`, `availableNetworksList`, `connect`, `tryConnect`
- **Device client fields:**
  - `isConnected`, `isDeviceRemote` / `isRemoteDevice`, `deviceIdentityMap`, `checkInterfaceVersion`, `storage`
  - `config`: `deviceType` (DeviceConfigData LF1/LF2/LF2_PLUS/LFC/SIMULATED), `name`, `hostname`, `serialNumber`, `partNumber`, `manufactureDateDisplayString`, `lensTypeDisplayString`, `location`, `peekStreamUrlDisplayString`
  - `deviceState`: `status` (… OFFLINE), `statusString`, `ledColor`, `projectionMode` (PROJECTIONMODE_FRONT/REAR/FRONT_CEILING/REAR_CEILING)
  - `firmwareState`: `currentVersion`, `availableVersion`, `updateType`, `isFirmwareCompatible`
  - `otaState.overallPercentComplete`
  - `networkState`: `connectionType` (CONNECTIONTYPE_WIFI…), `visibleNetworksModel`, `savedNetworksModel`
  - `displayState`: `resolution`, `availableResolutionsModel`, `hdmiCableConnected`
  - `playbackState`: `status`, `projectName`, `projectTimestamp`, `projectPropertyBag`, `slidePropertyBags`, `slidesModel`, `slideCount`, `currentSlideIndex`, `currentSlideName`, `globalBrightness`, `hasProjectThumbnail`, `objectCoordinatePoints`, audio-reactivity maps, effect-name lists and overlay flag
  - `peekState` (camera stream)
  - `livelightState`: `ndiSourceList`, `currentNdiSource`
- **Device client commands:**
  - **Playback:** `play`, `pause`, `stop`, `nextSlide`, `previousSlide`, `setSlideIndex`, `setGlobalBrightness`
  - **Projector:** `turnProjectorOn` / `Off`, `adjustProjectorFocus`, `showTestcard` / `hideTestcard`, `setResolution`, `setProjectionMode`, `setLocation`, `restartDevice`, `renewPeekStream`
  - **Network and pairing:** `removeNetwork`, `forceIntoPairingMode`
  - **Firmware:** `checkForFirmwareUpdates`, `updateFirmware`, `setUpdateType`
  - **Audio reactivity:** `setAudioReactivityEnabled` / `Settings` / `OverlayEnabled` / `CalibrationEnabled`
  - **OSC learn:** `learnParameterOscAddress` / `doneLearningParameterOscAddress` / `currentLearningParameter(Group)Name`
  - **Publish and project:** `publish`, `cancelPublish`, `updateProjectThumbnail`, `setObjectCoordinates`
  - **Auto-align:** `oaklightSetCustomObject` / `OnObjectAlignment` / `Off`
  - **Live input:** `livelightOn` / `Off` / `SetNdiSource` / `ClearNdiSource` / `ClearVectorScene` / `ClearCursorPosition`
  - **Scan:** `scanWorkflowObject` (`prepareNewWorkflow`, `doScanWorkflow`, `cancelWorkflow`, `progress`, `statusMessage`, `errorMessage`, `projectorDimensions`, `downloadedScanImageList`)
- **Device state structs** *(bin)*: DeviceConfigData, Firmware/Network/OTA/Display/Renderer/Playback/Peek/Scan StateData, DeviceScanSettings, NetworkConfiguration. Strings near them also show mDNS `224.0.0.251` and gRPC/protobuf. The protocol itself is out of scope for this worker.

**`streamManager`:** `isStreaming`.

## 5. Style system
- **`controls/core/core-style/StaticStyle.qml`** (singleton). It holds the palette, which is a small dark theme:
  - **Neutrals:** `#FFFFFF`, `#BABFD1`, `#575C6D`, `#3C3E47`, `#2B2B30`, `#232326`, `#1A1A1C`, `#161618`, `#0E0E0E`, `#000000`
  - **Blue:** `#5885FF` / `#2C65FF` / `#2352D0`
  - **Purple:** `#7C2CFF` / `#690EFD`
  - **Yellow** (labs): `#F2C94C` / `#DCB745`
  - **Green:** `#17E8B0` / `#15C595` / `#12A37B`
  - **Red:** `#D43333` / `#C23030`, and mask red `#FF0000`
  - Also default button/column/panel sizes, plus FontLoaders.
- **`styles/StyleSheet.qml`** (singleton, 467 lines). Semantic colour aliases and 111 `qrc:/resources/icons/*.svg|png` image references. `styles/StyleSheet.qss` / `StyleSheet-QPushButton.qss` style the native widgets.
- **`*DynamicSizes.qml`** (≈100 files, 2,074 lines). Per-component metric tables passed through `BaseDynamicSizes.adjustValue()`, which applies DPI fudge factors (×1.3 / ×1.0 / ×0.8, `controls/core/core-style/BaseDynamicSizes.qml`). In Slint these collapse into a single `globals.slint` token file, because Slint uses logical pixels with automatic scale factor.
- **Fonts** (`controls/core/core-style/fonts/`): Atlas Grotesk (Thin/Light/Regular/Medium + italics) and Atlas Typewriter Medium, both by **Commercial Type**. They're commercial fonts with an embedding licence that won't carry over to a new product. Substitute with Inter / IBM Plex or license them.
- **Images:** `controls/images/*.svg|png` (10), `resources/logo/logotype-{black,white}.svg`, app icons (`resources/app/{app,early-access-app,internal-app}.{icns,ico,png}`).
- **Extraction gap (resolved 2026-10-18):** all 116 icons are now in `re/qrc/resources/resources/icons/` (one entry unnamed) and the 34 cursors in `.../cursors/`. Original note: **109 of 127 `qrc:/` image refs were missing from `re/qrc`.** All of them sit under `resources/icons/`. Their names are present as UTF-16 in `MacOS/Lightform` at about file offset `0xE530C6`, with SVG data near `0xDC6578`. That's a second rcc tree the extractor didn't align, so re-run extraction targeting that region.

## 6. Hard-to-port QML features

| Feature | Where | Port note |
|---|---|---|
| `GraphicsQuickControl` (C++ QQuickItem, FBO renderer) | `DesignPage.qml:547` main canvas; `components/DesignPageSlidesBar.qml:235` one live renderer per slide thumbnail | The engine's real-time renderer. Needs a **wgpu surface**: render offscreen and hand a texture to Slint (Slint wgpu interop) or use a rendering underlay. Per-slide live thumbnails become N small render targets or cached snapshots. |
| Canvas tools and interaction (pen/bend/structure/move/rotate, pixel brush/wand) | C++ `InteractiveViewport` / ToolManager behind `GraphicsQuickControl` | All input handling and overlay drawing lives in C++, not QML, so it must be written in Rust against the wgpu canvas. This is the largest single UI item. |
| `Canvas` (JS 2D) | `ControlPagePerspectiveWarpWindow.qml:174` (4-corner warp), `components/AssetBrowserPage.qml:205` (drop-zone border) | Slint `Path` elements handle both. |
| `ReorderableTreeView` with drag reorder and autoscroll | `controls/core/ReorderableTreeView.qml`, `components/DesignPageLayersPanel.qml` | Slint has no TreeView. Build a flattened ListView with indent and a custom drag via TouchArea plus pointer tracking. Same approach for slide reorder (`ReorderableListView`, `ControlPageSlidesList*Delegate.qml`). |
| `Loader` by URL (25 uses in 14 files), especially `uiSourceFile` | `controls/PropertiesPanel.qml` | Slint is compile-time only. Replace with an enum-switched `if` of 5 known panels (project/slide/surface/contel/generic). |
| Property-type dispatch for 27 types | `controls/PropertyControl.qml` (564 lines) | Port as one Slint component with conditional sub-widgets. Needs custom vector/matrix/colour editors and a 2D slider (`PropertySlider2D`, `BaseSlider2D`). |
| `Repeater` over models (38) | property groups, settings pages | Direct: Slint `for` over `ModelRc`. |
| QtQml.StateMachine (8 workflows) | §2 | Move to Rust state enums. Slint only renders the current state. |
| External file drag-and-drop | `DesignPage.qml:607` `DropArea` (file import) | Slint's support for OS file drops is limited and must be verified; may need a winit-level hook. Internal DnD uses custom code. |
| Rich text / HTML links | `SelectDeviceWorkflow.qml`, `PairingWorkflow.qml` (`<b>`, `<font>`, `Utilities.makeLink`) | Slint has limited styled-text support. Restructure as separate Text and link elements. |
| Syntax-highlighted code editors | `EffectEditor.qml` (C++ QSyntaxHighlighter helpers) | No Slint equivalent. Use plain TextEdit, a custom highlighted view, or open in an external editor with file watching (the app already watches files). |
| `SequencedImage` animated thumbnails | `components/AssetBrowserAssetElementDelegate.qml:127` | Rust decodes frames, swaps `Image` on a Timer. |
| Video/camera streams | `CameraStreamWindow.qml`, `controls/ImageRegionOfInterestSelector.qml`, NDI | Rust decoder → `SharedPixelBuffer` → `Image`, updated per frame. |
| Graphical effects (`Qt5Compat.GraphicalEffects`, 3 uses), `ColorizedSVG` (5) | various | Slint `colorize` on images and drop shadows on Rectangle cover these. |
| Native menu bar and multiple windows | MainWindow (C++) | Slint `MenuBar` and separate window components. Recent Slint supports both; check the version you use. |
| C# scripting | `ScriptingDialog.qml` | UI is simple; the backend (embedded Mono) has no Rust analogue. Drop it or replace with Rhai or Lua. |

## 7. Slint porting assessment by area

| Area | QML source (approx. lines) | Slint verdict | Notes |
|---|---|---|---|
| Shell: menus, page switching, welcome, about, dialogs | ~1.5k | Direct | Rebuild MainWindow menus in Slint. Pages become a page enum. |
| Style tokens, fonts, icons | ~2.7k (incl. DynamicSizes) | Direct (shrinks to ~300) | Font licence issue; recover the missing icons first. |
| Generic controls (`controls/core/*`, Property* editors) | ~5k | Mostly direct; custom widgets for 2D slider, colour picker (HSV), draggable numeric field, vector/matrix | Slint std-widgets cover buttons, combos, sliders, text fields. |
| Properties panels (generic + 4 custom) | ~3k | Direct UI, heavy Rust model | Requires a Rust property-bag/undo system mirroring §4.2. |
| Design page layout, toolbars, tool rows, context menu | ~3k | Direct | Tool state comes from a Rust ToolManager. |
| Design canvas and slide thumbnails | (C++) | **Needs wgpu rendering surface + custom input** | The core of the product. Depends on the engine/renderer port. |
| Layers tree and slide reorder | ~1.5k | Needs custom Rust/Slint widget | Flattened tree list with DnD. |
| Asset browser and stock catalog | ~2k | Direct UI + custom animated thumbnails | Stock catalog (Storyblocks) is probably dead; drop it. |
| Control page: device panel, slides, OSC, audio reactivity, warp | ~3.7k | Direct; warp uses `Path`; camera stream needs a frame→Image pump | Behaviour depends on the device protocol (other worker). |
| Device select, pairing, firmware, publish, scan workflows | ~4.7k | Direct UI; Rust state machines | Cloud-device parts are dead; local LAN parts are the value. |
| Effect editor | 650 | Custom (code editor) | Or delegate to an external editor. |
| Scripting, GPU profiler, developer tools | ~1.1k | Drop or defer | Developer-only. |
| Login / account / bug report / analytics / updater | ~1.3k | Drop | Lightform services are defunct. |

**Rough size for a faithful port of the UI layer, excluding the engine, renderer and device protocol:**
- About **15–20k lines of `.slint`**, starting from ~34k QML; the DynamicSizes and JS glue shrink.
- **8–12k lines of Rust** for view-models and state machines.
- Roughly **3–5 developer-months** for one experienced Rust/Slint developer.
- Dropping the dead cloud features, scripting and developer tools cuts this by about 25%.

**Not included in that estimate**, and on the critical path:
- The wgpu canvas: rendering, tools, hit-testing, pixel tools and vectorisation
- The `.lit` object model with undo
- Video decoding
- The device protocol

## 8. Open questions / unknowns
- **Default keyboard shortcuts:** not recoverable from strings (§3.11).
- **Missing icons:** resolved; see §5.
- **Unseen C++ panels:** whether any `uiSourceFile` points outside the four custom panels, at `:/external/lit-engine/qtengine/qml/controls/custom-panels/` *(bin)*. `controls/custom-panels/lightform.object.polygon.qml` exists and may be live.
- **ApplicationSettingsDialog page set:** inferred from `PageIndex` names; not walked line by line.
