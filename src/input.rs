use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::{env, path::PathBuf, thread};

use anyhow::{Context, Result, bail};
use smithay_client_toolkit::reexports::calloop::channel::Sender;

use crate::spritesheet::Animation;

// input threading stuff to read the hyprland socket

// the custom>> is kinda weird, i deduced by watching the hyprland socket itself.
// grepping in actual hyprland source code shows these sources which lets me
// hopefully assume this will be stable.
//
// : src/config/shared/actions/ConfigActions.cpp    (found by ripgrepping for custom)
// ActionResult Actions::event(const std::string& data) {
//    g_pEventManager->postEvent(SHyprIPCEvent{.event = "custom", .data = data});
//    return {};
// }
//
// : src/managers/EventManager.cpp    (found this by ripgrep '\{\}>>\{\}')
// std::string CEventManager::formatEvent(const SHyprIPCEvent& event) const {
//    std::string_view data        = event.data;
//    auto             eventString = std::format("{}>>{}\n", event.event, data.substr(0, 1024));
//    std::replace(eventString.begin() + event.event.length() + 2, eventString.end() - 1, '\n', ' ');
//    return eventString;
// }
const EVENT_PREFIX: &str = "custom>>femtanyl:";

// read key presses/releases
pub struct InputEvent {
    pub animation: Animation,
    pub pressed: bool,
}

// reading hyprland socket on background w blocking reads is cheaper than async
pub fn spawn_hyprland_events(sender: Sender<InputEvent>) {
    thread::spawn(move || {
        if let Err(error) = run_hyprland_events(&sender) {
            eprintln!("Hyprland input unavailable: {error:#}");
        }
    });
}

fn run_hyprland_events(sender: &Sender<InputEvent>) -> Result<()> {
    let socket = hyprland_event_socket()?;
    let stream = UnixStream::connect(&socket)
        .with_context(|| format!("could not connect to {}", socket.display()))?;

    for line in BufReader::new(stream).lines() {
        let line = line?;
        let Some((animation, action)) = parse_event(&line) else {
            continue;
        };
        let pressed = match action {
            "down" => true,
            "up" => false,
            other => {
                // dont bail, better to assume harmlessly bad client input
                eprintln!("unknown input action: {other}");
                continue;
            }
        };
        if sender.send(InputEvent { animation, pressed }).is_err() {
            break; // event loop is gone; nothing left to do
        }
    }

    bail!("Hyprland event socket closed")
}

// this might need dbus-run-session start-hyprland
fn hyprland_event_socket() -> Result<PathBuf> {
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is unset")?;
    let instance = env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
        .context("HYPRLAND_INSTANCE_SIGNATURE is unset")?;
    Ok(PathBuf::from(runtime_dir)
        .join("hypr")
        .join(instance)
        .join(".socket2.sock"))
}

fn animation_for_event(direction: &str) -> Option<Animation> {
    match direction {
        "up" => Some(Animation::Up),
        "left" => Some(Animation::Left),
        "down" => Some(Animation::Down),
        "right" => Some(Animation::Right),
        _ => None,
    }
}

fn parse_event(line: &str) -> Option<(Animation, &str)> {
    let event = line.strip_prefix(EVENT_PREFIX)?;
    let (direction, action) = event.split_once(':')?;
    Some((animation_for_event(direction)?, action))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_movement_events() {
        assert_eq!(
            parse_event("custom>>femtanyl:up:down"),
            Some((Animation::Up, "down"))
        );
        assert_eq!(
            parse_event("custom>>femtanyl:right:up"),
            Some((Animation::Right, "up"))
        );
        assert_eq!(parse_event("custom>>femtanyl:unknown:down"), None);
    }
}
