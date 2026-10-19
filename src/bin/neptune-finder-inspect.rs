//! Developer-only client for an explicitly enabled native egui inspection server.
use std::{env, fs, net::TcpStream, path::Path, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use eframe::egui::{Event, Key, Modifiers, PointerButton, pos2};
use egui_inspection::{Request, Response, protocol};

const HELP: &str = "neptune-finder-inspect [--addr HOST:PORT] COMMAND\n\
\n\
  info                         Print application/protocol information\n\
  tree                         Print the current AccessKit tree as JSON\n\
  screenshot PATH              Save a native screenshot at 1 px/logical point\n\
  key KEY [--ctrl --shift --alt --cmd]\n\
                               Press/release a key or chord (ctrl+shift+t)\n\
  text TEXT                    Inject text, including Unicode\n\
  move X Y                     Move pointer using logical coordinates\n\
  click X Y                    Click using logical window coordinates\n\
  context X Y                  Secondary-click, e.g. to open a context menu\n\
  double-click X Y             Two primary clicks within one frame\n\
  drag X1 Y1 X2 Y2             Press at the first point, move, release at the second\n\
  press X Y                    Press and hold the primary button, e.g. to inspect a drag\n\
  release X Y                  Move to the point and release the primary button\n\
  resize WIDTH HEIGHT          Resize using logical dimensions\n\
  settle [MAX_STEPS]            Wait for an idle frame (default: 60)\n\
\n\
Enable the app with EGUI_INSPECTION=1 and --features inspection.\n\
Default address: 127.0.0.1:5719. This client never launches the app.";

struct Client {
    stream: TcpStream,
}

impl Client {
    fn connect(address: &str) -> Result<Self> {
        let mut stream = TcpStream::connect(address)
            .with_context(|| format!("connect to inspection server {address}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(20)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        stream.set_nodelay(true)?;
        let version = protocol::read_handshake(&mut stream).context("inspection handshake")?;
        ensure!(
            version == protocol::PROTOCOL_VERSION,
            "incompatible inspection protocol {version}; expected {}",
            protocol::PROTOCOL_VERSION
        );
        Ok(Self { stream })
    }

    fn request(&mut self, request: Request) -> Result<Response> {
        protocol::write_message(&mut self.stream, &request).context("send inspection request")?;
        let response: Response =
            protocol::read_message(&mut self.stream).context("read inspection response")?;
        if let Response::Error { message } = &response {
            bail!("inspection server: {message}");
        }
        Ok(response)
    }

    fn events(&mut self, events: Vec<Event>) -> Result<()> {
        let response = self.request(Request::ApplyEvents { events })?;
        ensure!(
            matches!(response, Response::Done),
            "unexpected response: {response:?}"
        );
        Ok(())
    }
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn parse_key(arguments: &[String]) -> Result<(Key, Modifiers)> {
    let mut modifiers = Modifiers::NONE;
    let mut key = None;
    for argument in arguments {
        for token in argument.split('+') {
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "--ctrl" => modifiers.ctrl = true,
                "shift" | "--shift" => modifiers.shift = true,
                "alt" | "option" | "--alt" => modifiers.alt = true,
                "cmd" | "command" | "meta" | "super" | "--cmd" => {
                    modifiers.mac_cmd = true;
                }
                _ => {
                    ensure!(key.is_none(), "provide exactly one key");
                    // egui accepts canonical names plus conventional single-letter keys.
                    let candidate = Key::from_name(token).or_else(|| {
                        Key::ALL
                            .iter()
                            .copied()
                            .find(|candidate| candidate.name().eq_ignore_ascii_case(token))
                    });
                    key = Some(candidate.with_context(|| format!("unknown key {token:?}"))?);
                }
            }
        }
    }
    modifiers.command = modifiers.ctrl || modifiers.mac_cmd;
    Ok((key.context("provide a key or chord")?, modifiers))
}

/// A key press as the native window reports it. egui-winit turns the
/// clipboard chords into Cut, Copy and Paste events instead of pressed keys,
/// and Paste carries the system clipboard's text. This client reads no
/// clipboard, so it presses Paste as with nothing on it: no event until the
/// key is released.
fn key_press(key: Key, modifiers: Modifiers) -> Option<Event> {
    let windows = cfg!(target_os = "windows");
    let command = |letter| modifiers.command && key == letter;
    if key == Key::Cut || command(Key::X) || (windows && modifiers.shift && key == Key::Delete) {
        Some(Event::Cut)
    } else if key == Key::Copy
        || command(Key::C)
        || (windows && modifiers.ctrl && key == Key::Insert)
    {
        Some(Event::Copy)
    } else if key == Key::Paste
        || command(Key::V)
        || (windows && modifiers.shift && key == Key::Insert)
    {
        None
    } else {
        Some(Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        })
    }
}

fn number<T: std::str::FromStr>(argument: &str, name: &str) -> Result<T> {
    argument
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid {name}: {argument:?}"))
}

fn main() -> Result<()> {
    let mut arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.is_empty()
        || arguments
            .first()
            .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!("{HELP}");
        return Ok(());
    }
    let address = if arguments.first().is_some_and(|arg| arg == "--addr") {
        ensure!(
            arguments.len() >= 3,
            "--addr requires HOST:PORT and a command"
        );
        arguments.remove(0);
        arguments.remove(0)
    } else {
        egui_inspection::DEFAULT_INSPECTION_ADDR.to_owned()
    };
    let command = arguments.remove(0);
    let mut client = Client::connect(&address)?;
    match command.as_str() {
        "info" | "tree" => {
            ensure!(arguments.is_empty(), "{command} takes no arguments");
            let request = if command == "tree" {
                Request::GetTree
            } else {
                Request::GetInfo
            };
            print_json(&client.request(request)?)?;
        }
        "screenshot" => {
            ensure!(arguments.len() == 1, "screenshot requires PATH");
            let response = client.request(Request::GetScreenshot {
                pixels_per_point: Some(1.0),
            })?;
            let Response::Screenshot(image) = response else {
                bail!("unexpected screenshot response: {response:?}");
            };
            let path = Path::new(&arguments[0]);
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, &image.bytes).with_context(|| format!("save {}", path.display()))?;
            print_json(&serde_json::json!({"path": path, "size": image.size}))?;
        }
        "key" => {
            let (key, modifiers) = parse_key(&arguments)?;
            client.events(
                std::iter::once(Event::ModifiersChanged(modifiers))
                    .chain(key_press(key, modifiers))
                    .collect(),
            )?;
            client.events(vec![
                Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: false,
                    repeat: false,
                    modifiers,
                },
                Event::ModifiersChanged(Modifiers::NONE),
            ])?;
            print_json(&Response::Done)?;
        }
        "text" => {
            ensure!(
                arguments.len() == 1,
                "text requires a single quoted TEXT argument"
            );
            client.events(vec![
                Event::ModifiersChanged(Modifiers::NONE),
                Event::Text(arguments.remove(0)),
            ])?;
            print_json(&Response::Done)?;
        }
        "move" | "click" | "context" => {
            ensure!(
                arguments.len() == 2,
                "{command} requires X Y logical coordinates"
            );
            let x: f32 = number(&arguments[0], "X")?;
            let y: f32 = number(&arguments[1], "Y")?;
            ensure!(x.is_finite() && y.is_finite(), "coordinates must be finite");
            let pos = pos2(x, y);
            // egui resolves interaction against the preceding frame's widget
            // geometry. Establish hover first so a remote click uses the target
            // widget even when the native OS pointer was elsewhere.
            client.events(vec![
                Event::ModifiersChanged(Modifiers::NONE),
                Event::PointerMoved(pos),
            ])?;
            if command != "move" {
                let button = if command == "context" {
                    PointerButton::Secondary
                } else {
                    PointerButton::Primary
                };
                client.events(vec![
                    Event::ModifiersChanged(Modifiers::NONE),
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                    Event::PointerButton {
                        pos,
                        button,
                        pressed: false,
                        modifiers: Modifiers::NONE,
                    },
                ])?;
            }
            print_json(&Response::Done)?;
        }
        "double-click" => {
            ensure!(
                arguments.len() == 2,
                "double-click requires X Y logical coordinates"
            );
            let pos = pos2(number(&arguments[0], "X")?, number(&arguments[1], "Y")?);
            ensure!(
                pos.x.is_finite() && pos.y.is_finite(),
                "coordinates must be finite"
            );
            client.events(vec![
                Event::ModifiersChanged(Modifiers::NONE),
                Event::PointerMoved(pos),
            ])?;
            // Both clicks share a frame, so they fall inside the toolkit's
            // double-click interval regardless of client latency.
            let mut events = vec![Event::PointerMoved(pos)];
            for pressed in [true, false, true, false] {
                events.push(Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                });
            }
            client.events(events)?;
            print_json(&Response::Done)?;
        }
        "drag" => {
            ensure!(
                arguments.len() == 4,
                "drag requires X1 Y1 X2 Y2 logical coordinates"
            );
            let from = pos2(number(&arguments[0], "X1")?, number(&arguments[1], "Y1")?);
            let to = pos2(number(&arguments[2], "X2")?, number(&arguments[3], "Y2")?);
            ensure!(
                [from.x, from.y, to.x, to.y]
                    .iter()
                    .all(|value| value.is_finite()),
                "coordinates must be finite"
            );
            client.events(vec![
                Event::ModifiersChanged(Modifiers::NONE),
                Event::PointerMoved(from),
            ])?;
            client.events(vec![Event::PointerButton {
                pos: from,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }])?;
            // Separate frames let the toolkit recognise the motion as a drag.
            for step in 1..=4 {
                client.events(vec![Event::PointerMoved(from.lerp(to, step as f32 / 4.0))])?;
            }
            client.events(vec![Event::PointerButton {
                pos: to,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }])?;
            print_json(&Response::Done)?;
        }
        "press" | "release" => {
            ensure!(
                arguments.len() == 2,
                "{command} requires X Y logical coordinates"
            );
            let pos = pos2(number(&arguments[0], "X")?, number(&arguments[1], "Y")?);
            ensure!(
                pos.x.is_finite() && pos.y.is_finite(),
                "coordinates must be finite"
            );
            // The pointer arrives a frame before the button changes, so the
            // toolkit resolves the press or release against that position.
            client.events(vec![
                Event::ModifiersChanged(Modifiers::NONE),
                Event::PointerMoved(pos),
            ])?;
            client.events(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: command == "press",
                modifiers: Modifiers::NONE,
            }])?;
            print_json(&Response::Done)?;
        }
        "resize" => {
            ensure!(arguments.len() == 2, "resize requires WIDTH HEIGHT");
            let width: u32 = number(&arguments[0], "WIDTH")?;
            let height: u32 = number(&arguments[1], "HEIGHT")?;
            ensure!(width > 0 && height > 0, "dimensions must be positive");
            print_json(&client.request(Request::Resize { width, height })?)?;
        }
        "settle" => {
            ensure!(arguments.len() <= 1, "settle takes at most one MAX_STEPS");
            let max_steps = arguments
                .first()
                .map(|arg| number(arg, "MAX_STEPS"))
                .transpose()?
                .unwrap_or(60);
            ensure!(max_steps > 0, "MAX_STEPS must be positive");
            print_json(&client.request(Request::Settle { max_steps })?)?;
        }
        _ => bail!("unknown command {command:?}\n{HELP}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_include_command_semantics() {
        let (key, modifiers) = parse_key(&["ctrl+shift+t".into()]).unwrap();
        assert_eq!(key, Key::T);
        assert!(modifiers.ctrl && modifiers.shift && modifiers.command);
        assert!(!modifiers.alt && !modifiers.mac_cmd);
        let (key, modifiers) = parse_key(&["Comma".into(), "--cmd".into()]).unwrap();
        assert_eq!(key, Key::Comma);
        assert!(modifiers.mac_cmd && modifiers.command);
    }

    #[test]
    fn clipboard_chords_arrive_as_the_native_window_reports_them() {
        let press = |chord: &str| {
            let (key, modifiers) = parse_key(&[chord.into()]).unwrap();
            key_press(key, modifiers)
        };
        assert!(matches!(press("ctrl+x"), Some(Event::Cut)));
        assert!(matches!(press("cmd+c"), Some(Event::Copy)));
        assert!(press("ctrl+v").is_none());
        assert!(matches!(
            press("ctrl+a"),
            Some(Event::Key {
                key: Key::A,
                pressed: true,
                ..
            })
        ));
        assert!(matches!(
            press("x"),
            Some(Event::Key {
                key: Key::X,
                pressed: true,
                ..
            })
        ));
    }

    #[test]
    fn malformed_chords_are_rejected() {
        assert!(parse_key(&["ctrl+shift".into()]).is_err());
        assert!(parse_key(&["ctrl+a+b".into()]).is_err());
        assert!(parse_key(&["unknown-key".into()]).is_err());
    }
}
