# IDEI Control

Desktop companion app for **IDEI** USB hardware mixers. It reads slider positions over a virtual serial port (USB CDC) and maps them to **system**, **per-app**, **microphone**, or **game** volume on your PC.

Built with **Tauri 2**, **React**, **TypeScript**, and **Rust**.

| Platform | Audio | Buttons (media / shortcuts) |
|----------|-------|-----------------------------|
| **Windows** | Core Audio | Win32 `SendInput` |
| **Linux** | PulseAudio (PipeWire-compatible) | X11 via `enigo` |

Supported devices (handshake): **`ideiMx`** (3 sliders) and **`ideiMx-max`** (5 sliders). Requires IdeiMX STM32 firmware on the device (separate firmware project).

---

## Features

- USB CDC connect / auto-reconnect, multi-device by COM port
- Map each slider to multiple targets (system volume, apps, mic, game category)
- Presets stored in `localStorage`
- Per-button actions: media keys or keyboard shortcuts
- Hardware mute vs PC-handled mute (firmware `SET_HW_MUTE_BTN_MAP`)
- System tray, optional launch at login
- Light / dark theme

---

## Requirements

### Windows (primary)

- [Node.js](https://nodejs.org/) 20+ (LTS recommended)
- [Rust](https://rustup.rs)
- **MSVC** — Visual Studio Build Tools → “Desktop development with C++” (for `link.exe`)

### Linux (build or run)

Ubuntu/Debian example:

```bash
sudo apt install -y build-essential pkg-config libssl-dev \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  libpulse-dev libx11-dev libxtst-dev
sudo usermod -aG dialout "$USER"   # serial access — log out & back in
```

- **PipeWire** or **PulseAudio** running (`pactl info`)
- **X11 / XWayland** for simulated media keys and shortcuts on Linux

> Linux installers are **not** produced by `tauri build` on native Windows. Use **WSL2**, a Linux machine, or CI. Windows `.exe` builds work normally on Windows.

---

## Quick start (development)

```bash
git clone <your-repo-url>
cd idei-control   # or whatever you name the repo root (this `app/` folder)

npm install
npm run tauri dev
```

First run compiles the Rust backend and opens the window with Vite hot-reload for the UI.

---

## Build release

```bash
npm install
npm run tauri build
```

**Windows output**

- `src-tauri/target/release/idei-control.exe`
- Installers under `src-tauri/target/release/bundle/` (NSIS / MSI)

**Linux output** (when built on Linux or WSL)

- `src-tauri/target/release/bundle/deb/`
- `src-tauri/target/release/bundle/appimage/`

---

## How to use

1. **Plug in** the IDEI device (USB). On Windows it appears as `COMx`; on Linux as `/dev/ttyACM0` (typical).
2. **Open IDEI Control** and pick the port from the dropdown.
3. Click **Connect**. The app sends `IDENTIFY` and reads device model / slider count.
4. For each slider, use **Assignments** to add targets:
   - **System** — master volume  
   - **Mic** — default microphone  
   - **App** — a running audio session (by name/PID)  
   - **Games** — all detected game processes  
5. Move hardware sliders — assigned volumes update in real time.
6. **Presets** — save/load slider + button layout (browser `localStorage`).
7. **Buttons** — map physical buttons to media keys or shortcuts; optional NeoPixel feedback via firmware commands.

Config file (assignments, last port, etc.): OS app data dir, e.g.  
`%APPDATA%\com.idei.control\config.json` on Windows.

---

## Icons

Source image: `src-tauri/app-icon.png`

Regenerate all platform sizes:

```bash
npm run icon
```

Output: `src-tauri/icons/` (`icon.ico`, PNGs, etc.). Desktop-only; you can delete `src-tauri/icons/android` and `ios` if present — not used for Windows/Linux desktop builds.

---

## Optional telemetry

Anonymous usage events via [Umami](https://umami.is/) (optional, off unless configured).

```bash
cp src-tauri/.env.example src-tauri/.env
# Edit IDEI_TELEMETRY_* — do not commit .env
```

---

## Project layout

```
.
├── src/                 # React UI
├── src-tauri/
│   ├── src/
│   │   ├── serial.rs    # USB CDC, device protocol
│   │   ├── audio.rs     # Windows volume
│   │   ├── audio_linux.rs
│   │   ├── media_keys.rs
│   │   ├── config.rs
│   │   └── lib.rs
│   ├── app-icon.png
│   ├── icons/
│   ├── capabilities/
│   └── tauri.conf.json
├── package.json
└── README.md
```

---

## Before pushing to GitHub

This repo should **not** include build artifacts. `.gitignore` already excludes:

- `node_modules/`
- `dist/`
- `src-tauri/target/`
- `.env` / `src-tauri/.env`

From the project root (`app/`):

```bash
git init
git add .
git status   # confirm no node_modules, target, or .env
git commit -m "Initial commit: IDEI Control"
git remote add origin <your-repo-url>
git push -u origin main
```

If `git status` shows huge folders, stop and check `.gitignore` before committing.

---

## Scripts

| Command | Description |
|---------|-------------|
| `npm run dev` | Vite only (web UI in browser) |
| `npm run tauri dev` | Full desktop app + hot reload |
| `npm run tauri build` | Production build + installers |
| `npm run icon` | Regenerate icons from `app-icon.png` |
| `npm run lint` | ESLint |

---

## License

Add your license file if you open-source this repo.
