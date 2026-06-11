//! Media keys and keyboard shortcuts on Linux (X11 via enigo).

use crate::media_keys::{ButtonBinding, MediaKeyAction, ModMask};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

fn open_enigo() -> Result<Enigo, String> {
    Enigo::new(&Settings::default()).map_err(|e| {
        format!(
            "Nie można wysłać klawisza (X11/Wayland): {:?}. Na Waylandzie skróty mogą wymagać sesji X11.",
            e
        )
    })
}

fn media_key(action: MediaKeyAction) -> Option<Key> {
    match action {
        MediaKeyAction::None => None,
        MediaKeyAction::PlayPause => Some(Key::MediaPlayPause),
        MediaKeyAction::NextTrack => Some(Key::MediaNextTrack),
        MediaKeyAction::PreviousTrack => Some(Key::MediaPrevTrack),
        MediaKeyAction::Stop => Some(Key::MediaStop),
        MediaKeyAction::Mute => Some(Key::VolumeMute),
        MediaKeyAction::VolumeUp => Some(Key::VolumeUp),
        MediaKeyAction::VolumeDown => Some(Key::VolumeDown),
    }
}

/// Map Windows virtual-key codes (from UI capture on Windows) to enigo keys on Linux.
fn vk_to_key(vk: u16) -> Option<Key> {
    match vk {
        0x08 => Some(Key::Backspace),
        0x09 => Some(Key::Tab),
        0x0D => Some(Key::Return),
        0x1B => Some(Key::Escape),
        0x20 => Some(Key::Space),
        0x21 => Some(Key::PageUp),
        0x22 => Some(Key::PageDown),
        0x23 => Some(Key::End),
        0x24 => Some(Key::Home),
        0x25 => Some(Key::LeftArrow),
        0x26 => Some(Key::UpArrow),
        0x27 => Some(Key::RightArrow),
        0x28 => Some(Key::DownArrow),
        0x2D => Some(Key::Insert),
        0x2E => Some(Key::Delete),
        0x30..=0x39 => Some(Key::Unicode((b'0' + (vk - 0x30) as u8) as char)),
        0x41..=0x5A => Some(Key::Unicode((b'a' + (vk - 0x41) as u8) as char)),
        // Numpad digits: enigo only exposes Numpad0–9 on Windows; use Unicode on Linux.
        0x60..=0x69 => Some(Key::Unicode((b'0' + (vk - 0x60) as u8) as char)),
        0x70 => Some(Key::F1),
        0x71 => Some(Key::F2),
        0x72 => Some(Key::F3),
        0x73 => Some(Key::F4),
        0x74 => Some(Key::F5),
        0x75 => Some(Key::F6),
        0x76 => Some(Key::F7),
        0x77 => Some(Key::F8),
        0x78 => Some(Key::F9),
        0x79 => Some(Key::F10),
        0x7A => Some(Key::F11),
        0x7B => Some(Key::F12),
        _ => None,
    }
}

fn press_modifier(enigo: &mut Enigo, mods: ModMask, bit: u8, key: Key) -> Result<(), String> {
    if (mods & bit) != 0 {
        enigo
            .key(key, Direction::Press)
            .map_err(|e| format!("modifier: {:?}", e))?;
    }
    Ok(())
}

fn release_modifier(enigo: &mut Enigo, mods: ModMask, bit: u8, key: Key) -> Result<(), String> {
    if (mods & bit) != 0 {
        enigo
            .key(key, Direction::Release)
            .map_err(|e| format!("modifier: {:?}", e))?;
    }
    Ok(())
}

fn tap_shortcut(vk: u16, mods: ModMask) -> Result<(), String> {
    if vk == 0 {
        return Ok(());
    }
    let main = vk_to_key(vk).unwrap_or(Key::Other(vk as u32));
    let mut enigo = open_enigo()?;

    press_modifier(&mut enigo, mods, 1, Key::Control)?;
    press_modifier(&mut enigo, mods, 2, Key::Shift)?;
    press_modifier(&mut enigo, mods, 4, Key::Alt)?;
    press_modifier(&mut enigo, mods, 8, Key::Meta)?;

    enigo
        .key(main, Direction::Click)
        .map_err(|e| format!("shortcut: {:?}", e))?;

    release_modifier(&mut enigo, mods, 8, Key::Meta)?;
    release_modifier(&mut enigo, mods, 4, Key::Alt)?;
    release_modifier(&mut enigo, mods, 2, Key::Shift)?;
    release_modifier(&mut enigo, mods, 1, Key::Control)?;
    Ok(())
}

pub fn send_media_action(action: MediaKeyAction) {
    if let Some(key) = media_key(action) {
        if let Ok(mut enigo) = open_enigo() {
            let _ = enigo.key(key, Direction::Click);
        }
    }
}

pub fn send_button_binding(b: &ButtonBinding) {
    match b {
        ButtonBinding::None => {}
        ButtonBinding::Media { action } => send_media_action(*action),
        ButtonBinding::Shortcut { vk, mods, .. } => {
            let _ = tap_shortcut(*vk, *mods);
        }
    }
}
