# IDEI Control

Desktop companion for **IDEI** USB hardware mixers. Slider positions are read over USB CDC and mapped to **system**, **per-app**, **microphone**, or **game** volume — with optional media keys and keyboard shortcuts on device buttons.

| | Windows | Linux |
|---|---------|-------|
| **Audio** | Core Audio | PulseAudio (PipeWire-compatible) |
| **Buttons** | Win32 `SendInput` | X11 via `enigo` |
| **Installers** | NSIS, MSI | `.deb`, AppImage |

**Stack:** [Tauri 2](https://tauri.app/) · React · TypeScript · Rust

**Supported hardware:** `ideiMx` (3 sliders) · `ideiMx-max` (5 sliders) — requires [IdeiMX](https://loskubalos.eu.org/ideiMx) STM32 firmware on the device.

---

## Download

Pre-built installers are published on **[GitHub Releases](https://github.com/loskubalos/ideiControl/releases)**.

| Platform | File | Install |
|----------|------|---------|
| **Windows** | `IDEI Control_*_x64-setup.exe` or `.msi` | Run the installer |
| **Linux (general)** | `IDEI Control_*_amd64.AppImage` | `chmod +x` → run |
| **Linux (Debian/Ubuntu)** | `idei-control_*_amd64.deb` | `sudo dpkg -i idei-control_*.deb` |
---

## Quick start

1. Install IDEI Control and plug in your mixer over USB.
2. Open the app and click **Refresh** if the port list is empty.
3. Select the port (`COMx` on Windows, `/dev/ttyACM0` on Linux) and click **Connect**.
4. Assign each slider under **Assignments** (System, Mic, App, Games).
5. Move sliders — assigned volumes update in real time.
6. Save layouts as **Presets**; map **Buttons** to media keys or shortcuts.

### Linux — USB access

Add your user to the `dialout` group, then log out and back in:

```bash
sudo usermod -aG dialout "$USER"
```

If the device is plugged in but never appears, **ModemManager** may be holding the port (common on Ubuntu). Stop it temporarily to test:

```bash
sudo systemctl stop ModemManager
```

Unplug the device, plug it back in, and refresh the port list in the app.

### Linux — audio & buttons

- **Volume:** PipeWire or PulseAudio must be running (`pactl info`).
- **Media keys / shortcuts:** require X11 or XWayland; pure Wayland sessions may not receive simulated keys.

---

## Features

- USB CDC connect, auto-reconnect, and multi-device support (one port per device)
- Multiple volume targets per slider
- Per-app and game-category volume via active audio sessions
- Presets (stored in `localStorage`)
- Per-button media keys or keyboard shortcuts
- Hardware mute vs PC-handled mute (`SET_HW_MUTE_BTN_MAP` firmware command)
- System tray and optional launch at login
- Light / dark theme

---

## Configuration

Assignments, last port, and preferences are stored in the OS app-data directory:

| OS | Path |
|----|------|
| Windows | `%APPDATA%\com.idei.control\config.json` |
| Linux | `~/.config/com.idei.control/config.json` |

---

## Development

### Prerequisites

**Windows**

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://rustup.rs)
- Visual Studio Build Tools — *Desktop development with C++*

**Linux**

```bash
sudo apt install -y build-essential pkg-config libssl-dev \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  libpulse-dev libx11-dev libxtst-dev
```

### Run locally

```bash
git clone https://github.com/loskubalos/ideiControl.git
cd idei-control
npm install
npm run tauri dev
```

### Release build

```bash
npm run tauri build
```

| Output | Location |
|--------|----------|
| Windows `.exe` | `src-tauri/target/release/idei-control.exe` |
| Windows installers | `src-tauri/target/release/bundle/nsis/` · `bundle/msi/` |
| Linux `.deb` | `src-tauri/target/release/bundle/deb/` |
| Linux AppImage | `src-tauri/target/release/bundle/appimage/` |

### Scripts

| Command | Description |
|---------|-------------|
| `npm run tauri dev` | Desktop app with hot reload |
| `npm run tauri build` | Production build and installers |
| `npm run dev` | Vite dev server (UI only) |
| `npm run build` | Frontend production bundle |
| `npm run icon` | Regenerate icons from `src-tauri/app-icon.png` |
| `npm run lint` | ESLint |

### Optional telemetry

Anonymous usage events via [Umami](https://umami.is/). Disabled unless configured at build time:

```bash
cp src-tauri/.env.example src-tauri/.env
# Set IDEI_TELEMETRY_* — do not commit .env
```

---

## Project structure

```
├── src/                      React UI
├── src-tauri/
│   ├── src/
│   │   ├── serial.rs         USB CDC and device protocol
│   │   ├── audio.rs          Windows volume control
│   │   ├── audio_linux.rs    Linux PulseAudio volume control
│   │   ├── media_keys.rs     Windows input simulation
│   │   ├── media_keys_linux.rs
│   │   ├── config.rs
│   │   └── lib.rs
│   ├── icons/
│   └── tauri.conf.json
├── package.json
└── README.md
```

---

## License

Free for **non-commercial** use. You may use, modify, and redistribute the software and your own versions, provided you keep the same license and do not use it commercially. See [LICENSE](LICENSE).
