//! Klawisze mediów i skróty klawiszowe po zdarzeniach `btn` z urządzenia.

use serde::{Deserialize, Serialize};

/// Liczba przycisków mapowanych w UI (zgodnie z maks. `i` w JSON z firmware).
pub const MEDIA_BUTTON_SLOTS: usize = 5;

/// Modyfikatory (bitmask): Ctrl=1, Shift=2, Alt=4, Win=8
pub type ModMask = u8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKeyAction {
    /// Nic — używane tylko w wariancie `ButtonBinding::Media` po normalizacji nie występuje.
    None,
    PlayPause,
    NextTrack,
    PreviousTrack,
    Stop,
    Mute,
    VolumeUp,
    VolumeDown,
}

/// Co robi przycisk fizyczny po stronie PC (gdy włączone „Media keys from device”).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ButtonBinding {
    /// Brak akcji (tylko event w UI).
    None,
    /// Jedna z klawiszy multimedialnych (jak dotąd).
    Media {
        action: MediaKeyAction,
    },
    /// Skrót: `vk` = Windows virtual-key, `mods` = bitmask, `label` = tekst do UI.
    Shortcut {
        vk: u16,
        mods: ModMask,
        label: String,
    },
}

impl ButtonBinding {
    /// Po wczytaniu z JSON / migracji: `Media { None }` → `None`.
    pub fn normalized(self) -> Self {
        match self {
            ButtonBinding::Media {
                action: MediaKeyAction::None,
            } => ButtonBinding::None,
            other => other,
        }
    }
}

pub fn default_media_actions() -> [MediaKeyAction; MEDIA_BUTTON_SLOTS] {
    [
        MediaKeyAction::PlayPause,
        MediaKeyAction::NextTrack,
        MediaKeyAction::PreviousTrack,
        MediaKeyAction::None,
        MediaKeyAction::None,
    ]
}

pub fn default_button_bindings() -> [ButtonBinding; MEDIA_BUTTON_SLOTS] {
    let m = default_media_actions();
    std::array::from_fn(|i| match m[i] {
        MediaKeyAction::None => ButtonBinding::None,
        a => ButtonBinding::Media { action: a },
    })
}

pub fn bindings_from_legacy_media(actions: &[MediaKeyAction]) -> [ButtonBinding; MEDIA_BUTTON_SLOTS] {
    let mut out = default_button_bindings();
    for i in 0..MEDIA_BUTTON_SLOTS.min(actions.len()) {
        out[i] = match actions[i] {
            MediaKeyAction::None => ButtonBinding::None,
            a => ButtonBinding::Media { action: a },
        };
    }
    out
}

#[cfg(windows)]
mod win {
    use super::{ButtonBinding, MediaKeyAction, ModMask};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC_EX, KEYBD_EVENT_FLAGS, VIRTUAL_KEY,
    };

    fn tap_vk(vk: u16) {
        let down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let up = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        unsafe {
            let _ = SendInput(&[down, up], std::mem::size_of::<INPUT>() as i32);
        }
    }

    /// VK które na Windows często wymagają flagi extended (strzałki, PgUp…).
    fn vk_needs_extended(vk: u16) -> bool {
        matches!(
            vk,
            0x21..=0x28 | // prior, next, end, home, arrows
            0x2D | 0x2E // insert, delete
        )
    }

    /// Lewe modyfikatory — spójne z typowym nagrywaniem Ctrl+Shift+… (Discord / Electron).
    const VK_LCONTROL: u16 = 0xA2;
    const VK_LSHIFT: u16 = 0xA0;
    const VK_LMENU: u16 = 0xA4;
    const VK_LWIN: u16 = 0x5B;

    /// `MAPVK_VK_TO_VSC_EX` — poprawne kody skanowania (m.in. rozszerzone).
    unsafe fn vk_to_scan(vk: u16) -> u16 {
        MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC_EX) as u16
    }

    /// Jedno zdarzenie klawiatury: preferuj scan code (lepiej rozpoznawane przez aplikacje z globalnymi skrótami).
    fn input_key(vk: u16, key_up: bool, extended: bool) -> INPUT {
        let scan = unsafe { vk_to_scan(vk) };
        let mut flags = if key_up {
            KEYEVENTF_KEYUP
        } else {
            KEYBD_EVENT_FLAGS(0)
        };
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if scan != 0 {
            flags |= KEYEVENTF_SCANCODE;
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: scan,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            }
        } else {
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(vk),
                        wScan: 0,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            }
        }
    }

    fn tap_shortcut(vk: u16, mods: ModMask) {
        if vk == 0 {
            return;
        }
        let main_ext = vk_needs_extended(vk);
        let mut inputs: Vec<INPUT> = Vec::with_capacity(16);

        if (mods & 1) != 0 {
            inputs.push(input_key(VK_LCONTROL, false, false));
        }
        if (mods & 2) != 0 {
            inputs.push(input_key(VK_LSHIFT, false, false));
        }
        if (mods & 4) != 0 {
            inputs.push(input_key(VK_LMENU, false, false));
        }
        if (mods & 8) != 0 {
            inputs.push(input_key(VK_LWIN, false, false));
        }

        inputs.push(input_key(vk, false, main_ext));
        inputs.push(input_key(vk, true, main_ext));

        if (mods & 8) != 0 {
            inputs.push(input_key(VK_LWIN, true, false));
        }
        if (mods & 4) != 0 {
            inputs.push(input_key(VK_LMENU, true, false));
        }
        if (mods & 2) != 0 {
            inputs.push(input_key(VK_LSHIFT, true, false));
        }
        if (mods & 1) != 0 {
            inputs.push(input_key(VK_LCONTROL, true, false));
        }

        unsafe {
            let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    fn vk_for(action: MediaKeyAction) -> Option<u16> {
        let vk = match action {
            MediaKeyAction::None => return None,
            MediaKeyAction::PlayPause => 0xB3u16,
            MediaKeyAction::NextTrack => 0xB0,
            MediaKeyAction::PreviousTrack => 0xB1,
            MediaKeyAction::Stop => 0xB2,
            MediaKeyAction::Mute => 0xAD,
            MediaKeyAction::VolumeUp => 0xAF,
            MediaKeyAction::VolumeDown => 0xAE,
        };
        Some(vk)
    }

    pub fn send_media_action(action: MediaKeyAction) {
        if let Some(vk) = vk_for(action) {
            tap_vk(vk);
        }
    }

    pub fn send_button_binding(b: &ButtonBinding) {
        match b {
            ButtonBinding::None => {}
            ButtonBinding::Media { action } => {
                if let Some(vk) = vk_for(*action) {
                    tap_vk(vk);
                }
            }
            ButtonBinding::Shortcut { vk, mods, .. } => {
                tap_shortcut(*vk, *mods);
            }
        }
    }
}

/// Pomocniczo (np. testy); produkcyjnie używaj `send_button_binding`.
#[cfg(windows)]
#[allow(dead_code)]
pub fn send_media_action(action: MediaKeyAction) {
    win::send_media_action(action);
}

#[cfg(windows)]
pub fn send_button_binding(b: &ButtonBinding) {
    win::send_button_binding(b);
}

#[cfg(target_os = "linux")]
pub fn send_media_action(action: MediaKeyAction) {
    crate::media_keys_linux::send_media_action(action);
}

#[cfg(target_os = "linux")]
pub fn send_button_binding(b: &ButtonBinding) {
    crate::media_keys_linux::send_button_binding(b);
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn send_media_action(_action: MediaKeyAction) {}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn send_button_binding(_b: &ButtonBinding) {}
