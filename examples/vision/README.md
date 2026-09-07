# Visual Provider plugin protocol

Copy a manifest such as `scap-provider.manifest.json` into
`{HUB_APP_DATA_ROOT}/vision/plugins`, change `executable` to an existing
absolute path, then call `POST /api/vision/providers/reload` or use the UI.

The process receives exactly one JSON object on stdin and returns one JSON
object on stdout. Logs belong on stderr. Hub rejects non-zero exits, invalid
JSON, output above 1 MiB and executions beyond `timeoutSeconds`.

## Screenshot

- `{"operation":"monitors"}` ->
  `{"monitors":[{"id":"1","name":"Display","width":1920,"height":1080,"primary":true}]}`
- `{"operation":"capture","monitorId":"1","outputPath":"...uuid.png"}` ->
  write a PNG to `outputPath`, then return `{"width":1920,"height":1080}`.

## OCR

- `{"operation":"recognize","imagePath":"...uuid.png"}` ->
  `{"text":"...","blocks":[{"text":"...","confidence":0.98}]}`

## Translation

- Request fields: `operation=translate`, `text`, `sourceLanguage`,
  `targetLanguage`, and optional `model`.
- Response: `{"translatedText":"...","detectedSourceLanguage":"en"}`.

Plugin paths are issued by Hub. A plugin should not accept a second path or
silently upload artifacts. Credentials remain private to the plugin process and
must never be printed to stdout or stderr.
