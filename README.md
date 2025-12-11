# GeoGuessr Desktop

A desktop app for GeoGuessr with userscript support.

## Installation

### macOS
macOS Gatekeeper may block the app because it's not signed with an Apple Developer certificate ($99/year). This is a security feature, not a problem with the app.

**After installing, open Terminal and run:**
```bash
xattr -cr /Applications/GeoGuessrDesktop.app
```

This removes the quarantine flag that macOS adds to downloaded apps. You can then open the app normally.

### Windows
Download and run the `.msi` installer.

### Linux
Download the `.deb` or `.AppImage` file.

## Development

### Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
