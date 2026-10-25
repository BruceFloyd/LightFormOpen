# Lightform Creator 2.2.12 — device and cloud protocol

Static analysis of `Lightform.app/Contents/MacOS/Lightform` (x86_64, Qt 6.2). No device was contacted and the app was not run.
Labels: **[O]** observed directly in the binary or QML, **[I]** inferred from code shape or naming, **[U]** unknown or unconfirmed.

Evidence came from `strings`, `otool -tV` disassembly (addresses below are load addresses in `MacOS/Lightform`), `nm | c++filt`, embedded protobuf descriptors and the extracted QML in `re/qrc`. Hopper decompilation was attempted, but Hopper is blocked on its first-run demo/license dialog, so none of this rests on decompiler output.

Source layout recovered from build paths [O]. All under `/Users/server/.jenkins/workspace/lightform-creator-release-macos/`:

- `external/device-client/src/private/{LocalDeviceClient,DeviceHTTPRequests,ScanWorkflowObject}.cpp`, `src/gen-cpp/MetaRPC.pb.cc`: LAN device client.
- `external/lf-rpc/Source/public/lightform/rpc/{ServiceConnection,ServiceImplThread,ClientAPI.h}`: gRPC stream wrapper.
- `src/app/{RemoteDeviceControl,PublishManager,StreamManager,LoginManager,LoginSession}.cpp`, `src/app/firmware/FirmwareAppcast*.cpp`: app side.
- `external/cloud-api/.../{lightform/Client,storyblocks/*,updater/AppUpdater}.cpp`, `external/webservices/.../HttpClient.cpp`: cloud.

There are two device-client classes, `Lightform::LocalDeviceClient` (LAN) and `Lightform::RemoteDeviceClient` (via the Lightform cloud) [O]. Only the local path is still viable.

---

## 1. Port map (LAN, device side)

| Port | Proto | Use | Label |
|---|---|---|---|
| 7350/udp | JSON | Discovery queries sent from Creator, multicast and broadcast | O |
| 7351/udp | JSON | Creator listens here for discovery replies | O |
| 50050/tcp | gRPC (HTTP/2, insecure) | MetaRPC `Talk` bidi stream (primary) | O (port), I (role) |
| 50051/tcp | gRPC | MetaRPC `Talk` (fallback, tried if 50050 fails) | O (port), I (role) |
| 7341/tcp | HTTP/JSON | "Legacy stagemanager / MetaHTTP": state and control GET/POST, also used in pairing mode | O |
| 7343/tcp | HTTP | Peek camera: `GET /peek/frame` (single frame), `/peek/stream` | O |
| 7344/tcp | HTTP | Livelight: `/livelight/frame` (Creator pushes frames) | O |
| 7345/tcp | HTTP | Scan and thumbnail image download: `GET /<path>` | O |
| 7346/tcp | HTTP | Project upload: `POST /playback/upload` | O |
| 7500/tcp | HTTP | Device bug-report and log bundle: `http://<host>:7500/bug-report` | O (string), I (purpose) |
| NDI | NDI 4 (`libndi.4.dylib`) | Peek camera as an NDI source ("Peek NDI Sender"); device can also *receive* NDI for Livelight | O |

The pairing-mode device runs its own Wi‑Fi AP at **192.168.0.1** with the password **`lightform`**. The SSID is the device name. [O: QML `PairingWorkflow.qml:790-796`, string `192.168.0.1` @ `0x1005a7a69`]

---

## 2. Discovery [O unless marked]

The code is the class around `0x10055a8b0` to `0x10055c900` (`ApplicationPairingDeviceDiscoveryClient` and related).

- Creator creates a `QUdpSocket` and binds `AnyIPv4:7351` with `ShareAddress` (`0x10055a960`). It joins multicast group **224.0.0.251**, the mDNS group, but the payload is not DNS (`0x10055a9e6`).
- A `QTimer` fires every **15 s** (`TimeDuration::seconds(0xf)`, `0x10055a8f2`).
- On each tick Creator builds this JSON (`0x10055abe0`):
  ```json
  {"api": "0.0", "hostname": "<QHostInfo::localHostName()>", "timestamp": "<string>"}
  ```
  It sends the JSON with `writeDatagram(..., port 7350)` to 224.0.0.251, and also to the broadcast address of every up interface from `QNetworkInterface::allInterfaces()` (`0x10055b2a7`). It also sends to any "fixed discovery IPs": `addFixedDiscoveryIp` and `addUnknownDeviceByIp` for manual IP entry, and 192.168.0.1 in pairing mode [I].
- The reply is a datagram parsed with `QJsonDocument::fromJson` (`0x10055bb37`). Shape [O keys, I nesting]:
  ```json
  {"data": {"serialNumber": "...", "name": "...", "partNumber": "...",
            "manufactureDate": "...", "version": "...",
            "services": {"<service>": "<string>", ...}}}
  ```
  The service type names are mapped from strings at `0x100558f50`: `meta`, `none`, `data`, `downstream`, `upstream`, `download`, `upload`. Each value is a trimmed string, and an `http://` URL is built from it (`0x10055c364`). [I] The values are ports or URLs for the services in §1. **[U]** Exact value format.
- `partNumber` selects the device family (LF1, LF2/LF2+, LFC). An unknown part number disables the firmware check and the max-upload-size check [O strings].

## 3. MetaRPC: gRPC control channel [O]

- Recovered schema: `proto/MetaRPC.proto` (proto3, package `metarpc`, 191 messages, 1162 lines, extracted from the embedded `FileDescriptorProto` at `0xfc6f70`).
- Service: `service MetaRPC { rpc Talk(stream MetaRPCMessage) returns (stream MetaRPCMessage); }`. That's one bidirectional stream.
- Connection (`0x10056e900`–`0x10056f1a1`):
  - Target `"<host>:50050"`. If that fails, it retries `"<host>:50051"`, with both stored in the same slot [I fallback order].
  - `grpc::InsecureChannelCredentials()`. No TLS and no auth token.
  - Channel args: `keepalive_time_ms=10000`, `keepalive_timeout_ms=5000`, `keepalive_permit_without_calls=1`, `http2.max_pings_without_data=0` [I value], `http2.min_time_between_pings_ms=10000`.
  - Client metadata: **`api-version: 1.3.2`** (`0x10056ef67`). The server returns its own `api-version` in initial metadata (`0x1005878b6`). Creator logs `"starting metaprc connection. Local version: %3%, remote version: %4%"`, and the QML shows "Firmware is incompatible with this software version" on a mismatch.
- Envelope:
  ```proto
  message MetaRPCMessage {
    uint32 messageId = 1;
    oneof data {
      MetaRPCServerRequestMessage serverRequest = 2;  // Creator -> device command
      MetaRPCServerReplyMessage   serverReply   = 3;  // device -> Creator reply (same oneof tag as request)
      MetaRPCClientRequestMessage clientRequest = 4;  // device -> Creator state push
      MetaRPCClientReplyMessage   clientReply   = 5;  // Creator ack (empty)
    }
  }
  ```
  Field numbers inside the sub-oneofs are large hashes, for example `PlayRequestMessage = 424021114`. They look like a hash of the method name, carried over from an older JSON‑RPC protocol: `ErrorType` contains JSON‑RPC codes −32700…−32603, and the cloud relay posts `jsonrpc`/`method`/`params` [I]. Replies are matched by `messageId` [I].
- **Commands (Creator → device), 71 total.** The full field list is in the .proto. Grouped:
  - Session: `TestConnection`, `Subscribe{dataFlags}`, `Unsubscribe{dataFlags}` (`DataFlag` bitmask, `ALL=8191`).
  - Playback: `Play`, `Pause`, `Stop`, `SetSlideIndex{index}`, `SetSlideByName{name}`, `NextSlide`, `PrevSlide`, `SetGlobalBrightness{float}`, `SetGlobalVolume{float}`, `SetParameterValue{name,slideName,value,type}`, `SetParameterValueByOSCAddress`, `ModifyParameterOSCMapping`, `LearnOSCParameter`.
  - Audio reactivity: `SetAudioReactivity{Enabled,Settings,CalibrationEnabled,OverlayEnabled}`.
  - Display/projector: `ShowTestcard{mode}`, `HideTestcard`, `SetResolution{id}`, `TurnProjectorOn/Off`, `AdjustProjectorFocus{increase}`, `AdjustProjectorFocusFine{delta}`, `SetProjectionMode{FRONT|FRONT_CEILING|REAR|REAR_CEILING}`, `ResetKeystone`, `SetKeystoneAngles`, `SetKeystoneCorners`, `HdmiStart{id}`, `HdmiStop`, `Get/SetHdmiGeometryCorrection`.
  - Warp/structure (on-device mesh fine-tune): `Reset/Enable/DisableStructure`, `Enable/DisableStructureGridUI`, `Enable/DisableStructureTessellationUI`, `SetRendererStructure{Structure}`, `UpdateRendererStructure{updates}`, `SetObjectCoordinates{vertices[4],texCoords}`.
  - Device: `Reboot`, `ResetUserData`, `SetLocation{string}`, `ForceIntoPairingMode`.
  - Network: `ConnectToNetwork{ssid,security,psk}`, `RemoveNetwork{ssid}`.
  - Firmware: `CheckForFirmwareUpdates`, `UpdateFirmware`, `SetUpdateType{type}`.
  - Camera: `PeekOn`, `PeekOff`, `SetPeekExposure{int}`, `SetPeekWhiteBalance{int}`.
  - Scan: `StartScan{StartScanSettings}`, `CancelScan`.
  - Livelight (live drawing, NDI): `LivelightOn{fbW,fbH}`, `LivelightOff`, `LivelightSetCursorProperties`, `LivelightSetCursorPosition`, `LivelightVectorSceneFullUpdate{VectorScene}`, `LivelightSetNdiSource{sourceName}`, `LivelightClearNdiSource`.
  - Oaklight (object mode): `OaklightOn{mode}`, `OaklightOff`, `OaklightSetObject{projectId}`, `OaklightSetCustomObject`, `RemoveAllCustomObjectFiles`.
  - Replies are mostly `{Response response}`, where `Response{Result{string result} | Error{ErrorType,message}}`. Exceptions are `GetHdmiGeometryCorrectionReply{GeometryCorrection}` and a few that use field 1.
- **State pushes (device → Creator), 13 total.** Each one corresponds to a `DataFlag` bit, delivered after `Subscribe`:
  - `DisplayStateChanged`: HDMI cable, resolution and available list, `CameraModel`, camera firmware, light engine on.
  - `NetworkStateChanged`: hostname, ethernet or wifi, client or AP mode, active SSID, known and available networks.
  - `DeviceStateChanged`: testcard, `LEDColor`, projection mode.
  - `FirmwareStateChanged`: current and available version, update type.
  - `OtaStateChanged`: status, percentages, bytes, rate, error.
  - `DeviceConfigChanged`: serial, name, part number, manufactured date, location.
  - `PlaybackStateChanged`: status, slide, `ProjectMetadata` (current) plus a list, brightness, volume, object coordinates.
  - `PlaybackParametersChanged`: global and per-slide exposed parameters with min/max/default/controlType/OSC addresses.
  - `AudioReactivitySettingsChanged`, `RendererSettingsChanged`, `RendererStructureFullUpdate`, `ScanStateChanged` (state, error, %), and `LivelightStateChanged` (NDI source list).
- `analytics.proto` (package `Lightform.Analytics.Protobuf`, 36 messages) was also recovered but is not distributed here. It covers only cloud analytics events and is irrelevant to the device; `tools/re/proto_extract.py` regenerates it from your own install.

## 4. Legacy HTTP ("stagemanager / MetaHTTP"), port 7341 [O]

The URL form is `http://%1:7341/%2` (`0x10056c0d9` GET, `0x10056c729` POST; functions `legacyStagemanagerGET/POST`). It's used for devices that haven't come online over MetaRPC, including pairing mode (`legacyDeviceInfo`, `setLegacyOnline`), and for a few dedicated operations:

- `GET  /state/network/wifi/ssids_available` returns `{"wifis":[{"ssid","security","signal"}...]}` [O keys, I shape].
- `POST /control/network/connect_new_wifi` sends `{"ssid","security","psk"}`. Security is one of `WPA2|WPA|WEP|UNSECURED`. Used by the pairing workflow.
- `GET  /state/ota/current/version`, `/state/ota/available/version`, `/state/ota/appcastid`.
- `POST /control/ota/appcastid`, `/control/ota/trigger`, `/control/ota/applyupdate`, `/control/ota/forceupdate`.
- `POST /control/oak/custom/uploadImage` uploads an image with headers `Content-Type` and `Content-Name`. Used for auto-align custom objects.

## 5. Project publish/upload [O]

- `PublishManager::start(DeviceClientBase*, QList<int> slideIndexes, QString)` renders the selected slides offline. The publish thread is `LightformPublishThread`. The output is a directory per slide, written to a temp or user folder, then zipped (`.zip`):
  - `content.mp4`: the slide rendered to video. If the slide is exactly one video contel, the source is copied without transcoding.
  - or `content.png` for a still.
  - `thumb.jpg`.
  - `audioReactiveMask.png`, `mask.png` for audio-reactive or live effects.
  - `shader.fs` + `shader.json` for an effect marked LIVE, which must be top of scene. This is an ISF shader generated by wrapping the effect in a template: `effect1Func`, `getEffectMaskValue`, `IMG_THIS_NORM_PIXEL(content|mask)`, `RENDERSIZE`, `PREMULTIPLY`.
  - `manifest.json` [I as the package manifest, mirroring `metarpc.ProjectMetadata{name,id,timestamp,description,slides[{name,duration,thumbnail,description,category}],type}`]. **[U]** Exact manifest keys. Compare with a real publish output folder ("Publish From Local Folder…" / "Publish to local folder" in the UI).
  - A per-`DeviceType` maximum package size is enforced ("Published package size … exceeds device maximum").
- Upload is `DeviceHTTPRequests::threadRun`, `0x10055d3ef`:
  - `POST http://<host>:7346/playback/upload`, where the method is the literal `"POST"` at `0x10055d534`.
  - Body is the zip file. `Content-Type: application/lightform+zip` (overrides the guessed `application/octet-stream`), `Content-MD5`, `Content-Length`. **[U]** Whether MD5 is base64 (RFC 1864) or hex, whether the body is multipart or raw, and any extra headers (likely `Content-Name`, as in oak upload [I]).
  - Progress drives `PlaybackStatus=UPLOADING`. The device then announces the new `ProjectMetadata` via `PlaybackStateChanged`, and Creator sends `SetSlideIndex` and `Play` [I].
- After publish: `GET http://<host>:7345/playback/thumbnail` fetches the project thumbnail [O path].
- The UI can also publish a pre-built external zip (`publishSettings.externalFile`) or only write the local folder (`outputPublishDirectory`) [O `PublishWorkflow.qml:40-45`].

## 6. Scan (structured light) [O unless marked]

1. `StartScan{settings}`. Settings fields: `hdrExposures`, `smoothCorrespondences`, `noiseReduction`, `safeMode`, `planarScan`, `cameraMask{Rectangle}`, `useExisting`, **`remoteUpload`** (upload to cloud, so set false), `isCreatorScan`, `analyzeObjects`, `analyzeEnvironment`, `objectId`, `cancelInProgressScan`.
2. `ScanStateChanged` reports progress: READY→INITIALIZING→CALIBRATING→SCANNING→PROCESSING→(UPLOADING)→READY, with errors such as `NO_CAMERA_DETECTED`, `FAILED_DETECTING_PATTERNS`, `CAMERA_MODEL_NOT_SUPPORTED`, `DISPLAY_DISCONNECTED`.
3. The images are downloaded by `GET http://<host>:7345/scan/0/{camera.jpg, disparity.png, mask.png, projector.jpg, projector1.jpg, invDisparity.png, invMask.png, invMaskFilled.png, invMaskDistance.png}`. Locally they are renamed to `camera-color.jpg`, `camera-disparity.jpg`, `camera-mask.png`, `projector-color.jpg` and `projector-color1.jpg`. Creator computes the "percentage holes" with OpenCV.
4. Optional: auto-align custom-object image via `POST :7341/control/oak/custom/uploadImage`.

## 7. Peek (live camera) and Livelight

- Peek: `PeekOn` and `PeekOff`, with exposure and white balance, over MetaRPC. Frames come from `GET http://<host>:7343/peek/frame` (`getCameraImageHttp`), `:7343/peek/stream`, or the NDI source "Peek NDI Sender" (`getCameraImageNdi`). QML shows it as `image://deviceClientImageProvider/peek` with an fps and time-left countdown, renewed with `renewPeekStream()` [O].
- Livelight: `LivelightOn{framebufferWidth,framebufferHeight}`, then either Creator pushes frames to `http://<host>:7344/livelight/frame` (`sendFrame`), or Creator tells the device to consume an NDI source (`LivelightSetNdiSource`). The vector-mask drawing is sent with `LivelightVectorSceneFullUpdate` [O].

## 8. Pairing / Wi‑Fi setup [O QML, I sequence]

From `PairingWorkflow.qml`, `ApplicationPairingDeviceDiscoveryClient` and `PairingWorkflowObject::tryConnect(ssid, security, password)`:

1. The user puts the device in pairing mode. The QML instructions differ by model: LF1 has a pairing button; LF2/LFC take three quick presses of power or the remote's back button, after which the LED or test card turns magenta or purple. New devices boot in pairing mode.
2. The user joins the device's Wi‑Fi AP (SSID = device name, password `lightform`). Creator watches the Mac's Wi‑Fi through CoreWLAN, using `WiFiNetworkClient` and the `asyncAdded/Removed/ModifiedWifiAccessPoint` events.
3. Creator discovers the device at 192.168.0.1 and lists networks via `GET :7341/state/network/wifi/ssids_available`, or MetaRPC `NetworkStateChanged.availableWifiNetworks`.
4. `tryConnect`: `POST :7341/control/network/connect_new_wifi {ssid,security,psk}`, or `ConnectToNetwork`. The device reboots onto the LAN. Creator asks the user to rejoin their own network and then rediscovers the device via UDP.

## 9. Firmware update

- Creator side: it lists S3 bucket **`lightform-ota`**, prefix `appcasts/`, with the embedded AWS SDK, unauthenticated or anonymous [I]. It downloads `*.xml` Sparkle-style appcasts with `appcast.item`, `partnumber`/`hardware_partnumber` and `channel`, and picks the one matching the device part number and channel [O].
- It then tells the device to update: `POST :7341/control/ota/appcastid` + `/control/ota/trigger` / `applyupdate` / `forceupdate`, or MetaRPC `SetUpdateType`, `CheckForFirmwareUpdates` and `UpdateFirmware`. **The device downloads the firmware itself over the internet** ("Device is in pairing mode. Cannot update firmware without an Internet connection") [O strings]. Progress comes from `OtaStateChanged`.
- DNS: `lightform-ota.s3.amazonaws.com` still resolves to an S3 us-west-2 endpoint. That suggests the bucket name is still registered [I], but the contents and the device-side download URLs are **[U]**. This was not fetched.

## 10. Cloud dependencies (DNS checked 2026-10-18, one lookup each)

| Endpoint | Used for | DNS | Status |
|---|---|---|---|
| `https://api.cloud.lightform.com/v/1` (`/login`, `/token`, `/users`, `/users/me`, `/users/me/password`, `/devices?embed=info`, `/devices/{id}`, `/devices/{id}/rpc` (JSON‑RPC relay), `/devices/{id}/scanState`, `/playbackState`, `/publishes/{id}`, `/scan`, `/cameraFrame`) | Login (OAuth-style `grant_type`/`refresh_token`/`access_token`, `Authorization: Bearer`), **remote device control** (`RemoteDeviceClient`, `RemoteDeviceDiscoveryClient`), remote publish, scan upload | NXDOMAIN | Dead |
| `https://api.dev.cloud.lightform.com/v/1` | Dev variant of the above | NXDOMAIN | Dead |
| `https://www.cloud.lightform.com/{account,device,register,signup,forgotPassword}.html` | Account web pages | NXDOMAIN | Dead |
| `https://analytics.cloud.lightform.com/v1/track/` | Analytics (`analytics.proto` events) | NXDOMAIN | Dead |
| `https://rainforest.lumenous3d.com` `/api/1/bug-reports/create`, `/{id}/upload-attachment` | Bug reports. A static bearer token is embedded in the binary; it's redacted here and not used. | CNAME to `rainforest.webservices.lumenous3d.com` with no A record | Dead |
| `https://s3-us-west-2.amazonaws.com/software.webservices.lumenous3d.com/Lightform-release-macos.json` | App self-updater metadata | S3 resolves; object not checked | **[U]** |
| S3 `lightform-ota` / `appcasts/*.xml` | Firmware appcasts | Resolves (us-west-2) | **[U]** |
| `https://api.videoblocks.com/api/v2/videos/{search,stock-item/details,stock-item/download}/` | Storyblocks stock-video catalog (`ServiceAssetsCatalogWindow`) | Resolves | Third party. It needs Lightform's partner credentials, so treat it as dead. |
| `https://guide.lightform.com/hc/...`, `https://lightform.com` | Help links (`urls.json`) | Resolve (WP Engine) | Links only |

Implications for a port: everything in §2–§9 except firmware download is LAN-only, unauthenticated and needs no cloud. Login, remote (cloud) devices, the stock catalog, analytics, bug reports and the updater can all be dropped.

## 11. Typical local session [I sequence, O pieces]

1. UDP discovery: send `{"api":"0.0",...}` to 7350 (multicast, broadcast, fixed IPs) every 15 s. The device replies to Creator:7351 with `data{serialNumber,name,partNumber,version,services}`.
2. Open gRPC `metarpc.MetaRPC/Talk` to `host:50050` (fallback 50051) with metadata `api-version: 1.3.2`, and compare the server's `api-version`.
3. Send `serverRequest{subscribeRequest{dataFlags: 8191}}`. Receive the 13 `clientRequest` state pushes ("Received all messages -> ONLINE"), then incremental pushes.
4. Publish: render the slides into a zip and `POST http://host:7346/playback/upload` (application/lightform+zip, Content-MD5). Wait for `PlaybackStateChanged` with the new project, then `GET :7345/playback/thumbnail`.
5. Control: `SetSlideIndex`, `Play`, `Pause`, `Stop`, `SetGlobalBrightness`, `SetParameterValue` and so on. Each one is a `serverRequest` with a `messageId`, answered by a `serverReply` with the same id and a `Response`.
6. Optional: `PeekOn` plus `GET :7343/peek/frame` polling; `StartScan`, then `ScanStateChanged`, then `GET :7345/scan/0/*`.

## 12. Needs live-device confirmation

1. Discovery reply exact JSON, including the `services` value format and whether `api`/`timestamp` matter. Does the device answer unicast to source port 7351 or to the 7351 multicast group?
2. Which gRPC port (50050 vs 50051) the user's firmware listens on, and what the server's `api-version` is. Does Creator refuse on a mismatch?
3. MetaRPC framing: is `messageId` correlation per request, and does the device require `Subscribe` before accepting commands? Is `TestConnection` used as a heartbeat?
4. Hashed oneof field numbers: confirm the device firmware uses this exact descriptor. Old firmware may only speak legacy HTTP on 7341.
5. Upload: raw vs multipart body, Content-MD5 encoding, other headers, response body, any max size per part number, and the zip layout plus `manifest.json` keys. Capture with a local-folder publish and a pcap of one upload.
6. Full legacy 7341 endpoint set: only the paths named in the binary are known, and the device may expose more. Also the response schemas.
7. Peek `/peek/stream` format (MJPEG?) and the frame format of `/peek/frame` and `/livelight/frame` (JPEG? raw RGBA with headers?).
8. Port 7500 `/bug-report` contents: likely a device log bundle, which is useful for debugging.
9. Firmware OTA: whether the device's update servers are still reachable. If not, the update path is dead regardless of Creator. Don't trigger an update until this is known.
10. Pairing-mode behaviour: whether discovery works on the device AP or Creator only uses 192.168.0.1, and whether `connect_new_wifi` is still the path for current firmware.
