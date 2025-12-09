# Development Notes

## Future Architecture Improvement

### Idea: Load GeoGuessr on Startup

Instead of the current flow:
- Main window (script manager) → Button click → Opens GeoGuessr webview

Consider:
- GeoGuessr webview loads immediately on app startup
- Script configuration handled separately (popup, overlay, system tray, or separate config window)

**Benefits:**
- Simpler architecture
- Faster time-to-game for users
- No intermediate launcher window

**Configuration options to explore:**
- Keyboard shortcut to open config panel
- Small floating button overlay on GeoGuessr page
- System tray menu
- Separate config window that opens on demand

---

## Technical Notes

### Script Injection

The app uses Tauri's `initialization_script` which runs in an isolated JavaScript context. To interact with the page's main world (needed for fetch interception), we inject `<script>` tags into the DOM.

Current flow:
1. `initialization_script` runs in isolated context
2. Waits for `document.documentElement` to exist
3. Injects script tags into page DOM
4. Scripts execute in page's main world

### Fetch Interception

The GeoGuessr Event Framework intercepts `fetch` to detect game events (round start, round end, etc.). This only works when running in the page's main world, hence the script tag injection approach.
