# IDEI Control

Desktop companion app for **[ideiMx](https://loskubalos.eu.org/ideiMx)** USB hardware mixers — map physical sliders to system / per-app / mic volume, with button shortcuts and NeoPixel feedback.

**Repository:** [github.com/loskubalos/ideiControl](https://github.com/loskubalos/ideiControl)  
**Stack:** [Tauri 2](https://tauri.app/) · React · TypeScript · Rust  
**Hardware link:** USB **Custom HID** (`VID:PID 0483:5750`) — plug & play on Windows; Linux needs a one-time udev rule.

| | Windows | Linux |
|---|---------|-------|
| Audio | Core Audio | PulseAudio / PipeWire |
| Buttons / shortcuts | `SendInput` | X11 (`enigo`) |
| Installers | NSIS, MSI | `.deb`, AppImage |
| Auto-update | GitHub Releases + Tauri Updater | same |

---

## Download

Pre-built binaries: **[GitHub Releases](https://github.com/loskubalos/ideiControl/releases)**.

In-app updater endpoint:

```text
https://github.com/loskubalos/ideiControl/releases/latest/download/latest.json
```

---

## Quick start (users)

1. Install IDEI Control and plug in the mixer over USB.
2. The app detects the device automatically (Scan / Connect if needed).
3. Assign each slider (System, Mic, App, Games) and optionally map buttons.
4. Optional: allow anonymous error reports in Settings (opt-in).

### Linux — USB HID permissions

Without udev rules, opening the device may require root. The `.deb` package installs the rule automatically. For AppImage / source builds:

```bash
sudo cp src-tauri/extra/99-ideimx.rules /etc/udev/rules.d/
# or:
sudo bash src-tauri/extra/install-udev.sh
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Then unplug/replug the device.

---

## Local development

**Requirements (Windows):** Node.js, Rust (`rustup`), MSVC Build Tools.

```bash
npm install
npm run tauri dev
```

Production build (no updater signing required locally):

```bash
npm run tauri build
```

`bundle.createUpdaterArtifacts` stays `false` in `tauri.conf.json` so local builds do **not** need `TAURI_SIGNING_PRIVATE_KEY`. CI enables updater artifacts only on tagged releases.

Optional PocketBase error reporting — copy env and fill secrets (never commit `.env`):

```bash
cp src-tauri/.env.example src-tauri/.env
# set IDEI_ERROR_REPORT_URL and IDEI_ERROR_REPORT_TOKEN
```

---

## Auto-updater & GitHub Actions (maintainers)

Releases are built by [`.github/workflows/release.yml`](.github/workflows/release.yml) on tags matching `v*` (e.g. `v0.1.1`).

### 1. Generate a signing key pair

```bash
npm run tauri signer generate -w ~/.tauri/ideiControl.key
```

- **Private key** → GitHub Secret `TAURI_SIGNING_PRIVATE_KEY` (paste file contents; password → `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if set).
- **Public key** → paste into `src-tauri/tauri.conf.json` → `plugins.updater.pubkey` (safe to commit).

### 2. GitHub Secrets (`Settings → Secrets and variables → Actions`)

| Secret | Purpose |
|--------|---------|
| `TAURI_SIGNING_PRIVATE_KEY` | Signs installers / `latest.json` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Key password (optional) |
| `IDEI_ERROR_REPORT_URL` | PocketBase records endpoint (baked into release builds) |
| `IDEI_ERROR_REPORT_TOKEN` | `X-App-Token` for PocketBase |
| `GITHUB_TOKEN` | Provided automatically by Actions |

### 3. Publish a version

1. Bump `version` in `src-tauri/tauri.conf.json` (and `package.json` if needed).
2. Commit, then:

```bash
git tag v0.1.1
git push origin v0.1.1
```

3. The workflow builds Windows NSIS + MSI, creates a GitHub Release, and uploads signed updater metadata (`latest.json`).

---

## Project layout

```
├── .github/workflows/release.yml
├── src/                      # React UI
├── src-tauri/
│   ├── extra/99-ideimx.rules # Linux udev
│   ├── src/                  # Rust
│   └── tauri.conf.json
├── package.json
└── README.md
```

---

## Hardware

Firmware for the STM32 controller is separate from this desktop app. Product page: [ideiMx](https://loskubalos.eu.org/ideiMx).
