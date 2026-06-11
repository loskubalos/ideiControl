//! Linux audio via PulseAudio API (PipeWire’s pulse compatibility layer works too).

use crate::audio::{normalize_app_name, AudioSessionInfo};
use libpulse_binding as pulse;
use pulse::context::{Context, FlagSet};
use pulse::mainloop::standard::Mainloop;
use pulse::operation::State as OpState;
use pulse::volume::{ChannelVolumes, Volume};
use std::cell::RefCell;
use std::rc::Rc;

const DEFAULT_SINK: &str = "@DEFAULT_SINK@";
const DEFAULT_SOURCE: &str = "@DEFAULT_SOURCE@";

fn pulse_err(msg: &str) -> String {
    format!(
        "{} (czy PulseAudio/PipeWire działa i czy użytkownik jest w grupie audio?)",
        msg
    )
}

fn wait_context_ready(mainloop: &Mainloop, context: &Context) -> Result<(), String> {
    loop {
        mainloop.iterate(false);
        match context.get_state() {
            (pulse::context::State::Ready, _) => return Ok(()),
            (pulse::context::State::Failed, _) | (pulse::context::State::Terminated, _) => {
                return Err(pulse_err("Nie można połączyć z serwerem audio"));
            }
            _ => {}
        }
    }
}

fn wait_operation(mainloop: &Mainloop, op: &pulse::operation::Operation) {
    while op.get_state() != OpState::Done {
        mainloop.iterate(false);
    }
}

fn with_pulse<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce(&Mainloop, &mut Context) -> Result<T, String>,
{
    let mainloop = Mainloop::new().map_err(|e| pulse_err(&format!("mainloop: {:?}", e)))?;
    let mut context = Context::new(&mainloop, "idei-control")
        .map_err(|e| pulse_err(&format!("context: {:?}", e)))?;
    context
        .connect(None, FlagSet::NOFLAGS, None)
        .map_err(|e| pulse_err(&format!("connect: {:?}", e)))?;
    wait_context_ready(&mainloop, &context)?;
    f(&mainloop, &mut context)
}

fn linear_to_channels(level: f32, channels: u8) -> ChannelVolumes {
    let v = Volume::linear(level.clamp(0.0, 1.0) as f64);
    let mut cv = ChannelVolumes::default();
    for ch in 0..channels {
        cv.set(ch, v);
    }
    cv
}

fn proplist_pid(proplist: &pulse::proplist::Proplist) -> Option<u32> {
    proplist
        .get_str("application.process.id")
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|p| *p > 0)
}

fn proplist_name(proplist: &pulse::proplist::Proplist, pid: u32) -> String {
    if let Some(bin) = proplist.get_str("application.process.binary") {
        let base = std::path::Path::new(bin)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| bin.to_string());
        if !base.is_empty() {
            return base;
        }
    }
    if let Some(name) = proplist.get_str("application.name") {
        let name = name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    format!("PID {}", pid)
}

fn get_process_path(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{}/exe", pid))
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| {
            std::fs::read_to_string(format!("/proc/{}/comm", pid))
                .ok()
                .map(|s| s.trim().to_string())
        })
}

fn is_game_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    let keywords = [
        "steam",
        "steamapps",
        "epic games",
        "epicgames",
        "origin",
        "minecraft",
        "gog galaxy",
        "gog.com",
        "ubisoft",
        "ubisoft game launcher",
        "battle.net",
        "battlenet",
        "ea app",
        "ea desktop",
        "xbox game",
        "game bar",
        "/games/",
        "/game/",
    ];
    keywords.iter().any(|k| lower.contains(k))
}

struct SinkInputRow {
    index: u32,
    pid: u32,
    name: String,
    is_active: bool,
    is_game: bool,
}

fn list_sink_inputs(mainloop: &Mainloop, context: &mut Context) -> Result<Vec<SinkInputRow>, String> {
    let rows: Rc<RefCell<Vec<SinkInputRow>>> = Rc::new(RefCell::new(Vec::new()));
    let rows_cb = rows.clone();
    let done = Rc::new(RefCell::new(false));
    let done_cb = done.clone();

    {
        let mut introspector = context.introspect();
        introspector
            .get_sink_input_info_list(move |result| match result {
                pulse::callbacks::ListResult::Item(info) => {
                    let Some(pid) = proplist_pid(&info.proplist) else {
                        return;
                    };
                    let name = proplist_name(&info.proplist, pid);
                    let is_game = get_process_path(pid)
                        .as_deref()
                        .map_or(false, is_game_path);
                    rows_cb.borrow_mut().push(SinkInputRow {
                        index: info.index,
                        pid,
                        name,
                        is_active: !info.corked,
                        is_game,
                    });
                }
                pulse::callbacks::ListResult::End => {
                    *done_cb.borrow_mut() = true;
                }
                pulse::callbacks::ListResult::Error => {
                    *done_cb.borrow_mut() = true;
                }
            })
            .map_err(|e| pulse_err(&format!("list sink inputs: {:?}", e)))?;
    }

    while !*done.borrow() {
        mainloop.iterate(false);
    }

    Ok(std::mem::take(&mut *rows.borrow_mut()))
}

fn default_sink_channels(mainloop: &Mainloop, context: &mut Context) -> Result<u8, String> {
    let channels: Rc<RefCell<Option<u8>>> = Rc::new(RefCell::new(None));
    let channels_cb = channels.clone();
    let done = Rc::new(RefCell::new(false));
    let done_cb = done.clone();

    {
        let mut introspector = context.introspect();
        introspector
            .get_sink_info_by_name(DEFAULT_SINK, move |result| match result {
                pulse::callbacks::ListResult::Item(info) => {
                    *channels_cb.borrow_mut() = Some(info.channels);
                    *done_cb.borrow_mut() = true;
                }
                pulse::callbacks::ListResult::End | pulse::callbacks::ListResult::Error => {
                    *done_cb.borrow_mut() = true;
                }
            })
            .map_err(|e| pulse_err(&format!("sink info: {:?}", e)))?;
    }

    while !*done.borrow() {
        mainloop.iterate(false);
    }

    channels
        .borrow()
        .copied()
        .filter(|c| *c > 0)
        .ok_or_else(|| pulse_err("Nie znaleziono domyślnego wyjścia audio"))
}

fn default_source_channels(mainloop: &Mainloop, context: &mut Context) -> Result<u8, String> {
    let channels: Rc<RefCell<Option<u8>>> = Rc::new(RefCell::new(None));
    let channels_cb = channels.clone();
    let done = Rc::new(RefCell::new(false));
    let done_cb = done.clone();

    {
        let mut introspector = context.introspect();
        introspector
            .get_source_info_by_name(DEFAULT_SOURCE, move |result| match result {
                pulse::callbacks::ListResult::Item(info) => {
                    *channels_cb.borrow_mut() = Some(info.channels);
                    *done_cb.borrow_mut() = true;
                }
                pulse::callbacks::ListResult::End | pulse::callbacks::ListResult::Error => {
                    *done_cb.borrow_mut() = true;
                }
            })
            .map_err(|e| pulse_err(&format!("source info: {:?}", e)))?;
    }

    while !*done.borrow() {
        mainloop.iterate(false);
    }

    channels
        .borrow()
        .copied()
        .filter(|c| *c > 0)
        .ok_or_else(|| pulse_err("Nie znaleziono domyślnego mikrofonu"))
}

pub fn get_system_volume_impl() -> Result<f32, String> {
    with_pulse(|mainloop, context| {
        let level: Rc<RefCell<Option<f32>>> = Rc::new(RefCell::new(None));
        let level_cb = level.clone();
        let done = Rc::new(RefCell::new(false));
        let done_cb = done.clone();

        {
            let mut introspector = context.introspect();
            introspector
                .get_sink_info_by_name(DEFAULT_SINK, move |result| match result {
                    pulse::callbacks::ListResult::Item(info) => {
                        if info.has_volume {
                            *level_cb.borrow_mut() =
                                Some(info.volume.avg().to_linear() as f32);
                        }
                        *done_cb.borrow_mut() = true;
                    }
                    pulse::callbacks::ListResult::End | pulse::callbacks::ListResult::Error => {
                        *done_cb.borrow_mut() = true;
                    }
                })
                .map_err(|e| pulse_err(&format!("get sink volume: {:?}", e)))?;
        }

        while !*done.borrow() {
            mainloop.iterate(false);
        }

        level
            .borrow()
            .ok_or_else(|| pulse_err("Brak głośności wyjścia"))
    })
}

pub fn set_system_volume_impl(level: f32) -> Result<(), String> {
    with_pulse(|mainloop, context| {
        let channels = default_sink_channels(mainloop, context)?;
        let cv = linear_to_channels(level, channels);
        let op = {
            let mut introspector = context.introspect();
            introspector
                .set_sink_volume_by_name(DEFAULT_SINK, &cv, None)
                .map_err(|e| pulse_err(&format!("set sink volume: {:?}", e)))?
        };
        wait_operation(mainloop, &op);
        Ok(())
    })
}

pub fn set_microphone_volume_impl(level: f32) -> Result<(), String> {
    with_pulse(|mainloop, context| {
        let channels = default_source_channels(mainloop, context)?;
        let cv = linear_to_channels(level, channels);
        let op = {
            let mut introspector = context.introspect();
            introspector
                .set_source_volume_by_name(DEFAULT_SOURCE, &cv, None)
                .map_err(|e| pulse_err(&format!("set source volume: {:?}", e)))?
        };
        wait_operation(mainloop, &op);
        Ok(())
    })
}

pub fn get_audio_sessions_impl() -> Result<Vec<AudioSessionInfo>, String> {
    with_pulse(|mainloop, context| {
        let rows = list_sink_inputs(mainloop, context)?;
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for row in rows {
            if seen.insert(row.pid) {
                out.push(AudioSessionInfo {
                    pid: row.pid,
                    name: row.name,
                    is_active: row.is_active,
                    is_game: row.is_game,
                });
            }
        }
        Ok(out)
    })
}

pub fn get_game_pids_impl() -> Result<Vec<u32>, String> {
    with_pulse(|mainloop, context| {
        let rows = list_sink_inputs(mainloop, context)?;
        let mut pids: Vec<u32> = rows
            .into_iter()
            .filter(|r| r.is_game)
            .map(|r| r.pid)
            .collect();
        pids.sort_unstable();
        pids.dedup();
        Ok(pids)
    })
}

fn set_sink_input_volume(
    mainloop: &Mainloop,
    context: &mut Context,
    index: u32,
    level: f32,
) -> Result<(), String> {
    let cv = linear_to_channels(level, 2);
    let op = {
        let mut introspector = context.introspect();
        introspector
            .set_sink_input_volume(index, &cv, None)
            .map_err(|e| pulse_err(&format!("set sink input {}: {:?}", index, e)))?
    };
    wait_operation(mainloop, &op);
    Ok(())
}

pub fn set_session_volume_impl(pid: u32, level: f32) -> Result<(), String> {
    with_pulse(|mainloop, context| {
        let rows = list_sink_inputs(mainloop, context)?;
        let matches: Vec<u32> = rows
            .into_iter()
            .filter(|r| r.pid == pid)
            .map(|r| r.index)
            .collect();
        if matches.is_empty() {
            return Err(format!("Sesja dla PID {} nie znaleziona", pid));
        }
        for index in matches {
            set_sink_input_volume(mainloop, context, index, level)?;
        }
        Ok(())
    })
}

pub fn set_session_volume_by_name_impl(app_name: &str, level: f32) -> Result<(), String> {
    let wanted = normalize_app_name(app_name);
    if wanted.is_empty() {
        return Err("Pusta nazwa aplikacji".to_string());
    }

    with_pulse(|mainloop, context| {
        let rows = list_sink_inputs(mainloop, context)?;
        let matches: Vec<u32> = rows
            .into_iter()
            .filter(|r| normalize_app_name(&r.name) == wanted)
            .map(|r| r.index)
            .collect();
        if matches.is_empty() {
            return Err(format!("Sesja dla aplikacji '{}' nie znaleziona", app_name));
        }
        for index in matches {
            set_sink_input_volume(mainloop, context, index, level)?;
        }
        Ok(())
    })
}
