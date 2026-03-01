use std::env;
use std::fs;
use std::os::unix::net::UnixStream;
use std::process::Command;

use greetd_ipc::codec::SyncCodec;
use greetd_ipc::{Request, Response};
use iced::widget::{button, column, container, pick_list, row, text, text_input};
use iced::{
    alignment, color, keyboard, Alignment, Background, Border, Color, Element, Length, Shadow,
    Task, Theme,
};
use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
use iced_layershell::settings::{LayerShellSettings, Settings};
use iced_layershell::to_layer_message;

// ── Session detection ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct Session {
    name: String,
    exec: String,
}

impl std::fmt::Display for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

fn detect_sessions() -> Vec<Session> {
    let dirs = ["/usr/share/wayland-sessions", "/usr/share/xsessions"];
    let mut sessions = Vec::new();

    for dir in &dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            let mut name = None;
            let mut exec = None;
            for line in contents.lines() {
                if let Some(v) = line.strip_prefix("Name=") {
                    name = Some(v.to_string());
                }
                if let Some(v) = line.strip_prefix("Exec=") {
                    exec = Some(v.to_string());
                }
            }
            if let (Some(name), Some(exec)) = (name, exec) {
                sessions.push(Session { name, exec });
            }
        }
    }

    sessions.sort_by(|a, b| a.name.cmp(&b.name));
    sessions.dedup_by(|a, b| a.exec == b.exec);
    sessions
}

// ── greetd IPC (blocking, run in thread) ───────────────────────────

#[derive(Debug, Clone)]
enum GreetdResult {
    Success,
    Error(String),
}

/// Run the full greetd login flow on a single connection.
fn greetd_login(username: &str, password: &str, session_cmd: &[String]) -> GreetdResult {
    let sock_path = match env::var("GREETD_SOCK") {
        Ok(p) => p,
        Err(_) => return GreetdResult::Error("GREETD_SOCK not set".into()),
    };
    let mut stream = match UnixStream::connect(&sock_path) {
        Ok(s) => s,
        Err(e) => return GreetdResult::Error(format!("connect: {e}")),
    };

    // Step 1: Create session
    let req = Request::CreateSession {
        username: username.to_string(),
    };
    if let Err(e) = req.write_to(&mut stream) {
        return GreetdResult::Error(format!("write create: {e}"));
    }

    match Response::read_from(&mut stream) {
        Ok(Response::AuthMessage { .. }) => {
            // Step 2: Send password
            let req = Request::PostAuthMessageResponse {
                response: Some(password.to_string()),
            };
            if let Err(e) = req.write_to(&mut stream) {
                return GreetdResult::Error(format!("write auth: {e}"));
            }

            match Response::read_from(&mut stream) {
                Ok(Response::Success) => {
                    // Step 3: Start session
                    let req = Request::StartSession {
                        cmd: session_cmd.to_vec(),
                        env: Vec::new(),
                    };
                    if let Err(e) = req.write_to(&mut stream) {
                        return GreetdResult::Error(format!("write start: {e}"));
                    }
                    match Response::read_from(&mut stream) {
                        Ok(Response::Success) => GreetdResult::Success,
                        Ok(Response::Error { description, .. }) => GreetdResult::Error(description),
                        Ok(_) => GreetdResult::Error("unexpected response after start".into()),
                        Err(e) => GreetdResult::Error(format!("read start: {e}")),
                    }
                }
                Ok(Response::Error { description, .. }) => GreetdResult::Error(description),
                Ok(resp) => GreetdResult::Error(format!("unexpected after auth: {resp:?}")),
                Err(e) => GreetdResult::Error(format!("read auth: {e}")),
            }
        }
        Ok(Response::Success) => {
            // No auth needed, start session directly
            let req = Request::StartSession {
                cmd: session_cmd.to_vec(),
                env: Vec::new(),
            };
            if let Err(e) = req.write_to(&mut stream) {
                return GreetdResult::Error(format!("write start: {e}"));
            }
            match Response::read_from(&mut stream) {
                Ok(Response::Success) => GreetdResult::Success,
                Ok(Response::Error { description, .. }) => GreetdResult::Error(description),
                Ok(_) => GreetdResult::Error("unexpected response after start".into()),
                Err(e) => GreetdResult::Error(format!("read start: {e}")),
            }
        }
        Ok(Response::Error { description, .. }) => GreetdResult::Error(description),
        Err(e) => GreetdResult::Error(format!("read create: {e}")),
    }
}

// ── Application state ──────────────────────────────────────────────

#[to_layer_message]
#[derive(Debug, Clone)]
enum Message {
    UsernameChanged(String),
    PasswordChanged(String),
    SessionSelected(Session),
    Login,
    LoginResult(GreetdResult),
    PowerOff,
    Reboot,
    KeyboardEvent(keyboard::Event),
}

enum Focus {
    Username,
    Password,
}

struct Greeter {
    username: String,
    password: String,
    sessions: Vec<Session>,
    selected_session: Option<Session>,
    error: Option<String>,
    logging_in: bool,
    focus: Focus,
}

fn focus_widget(name: &'static str) -> Task<Message> {
    iced::widget::operation::focus(iced::widget::Id::new(name))
}

// ── Application functions ──────────────────────────────────────────

fn boot() -> (Greeter, Task<Message>) {
    let sessions = detect_sessions();
    let selected = sessions.first().cloned();
    (
        Greeter {
            username: String::new(),
            password: String::new(),
            sessions,
            selected_session: selected,
            error: None,
            logging_in: false,
            focus: Focus::Username,
        },
        focus_widget("username"),
    )
}

fn namespace() -> String {
    "barrgreet".to_string()
}

fn update(state: &mut Greeter, message: Message) -> Task<Message> {
    match message {
        Message::UsernameChanged(u) => {
            state.username = u;
            state.error = None;
            Task::none()
        }
        Message::PasswordChanged(p) => {
            state.password = p;
            state.error = None;
            Task::none()
        }
        Message::SessionSelected(s) => {
            state.selected_session = Some(s);
            Task::none()
        }
        Message::Login => {
            if state.username.is_empty() {
                state.error = Some("Username is required".into());
                return Task::none();
            }
            let Some(session) = &state.selected_session else {
                state.error = Some("No session selected".into());
                return Task::none();
            };

            state.logging_in = true;
            state.error = None;

            let username = state.username.clone();
            let password = state.password.clone();
            let cmd: Vec<String> = session
                .exec
                .split_whitespace()
                .map(String::from)
                .collect();

            Task::perform(
                async move {
                    // Run blocking IPC in a thread, await via futures channel
                    let (tx, rx) = futures::channel::oneshot::channel();
                    std::thread::spawn(move || {
                        let result = greetd_login(&username, &password, &cmd);
                        let _ = tx.send(result);
                    });
                    rx.await.unwrap_or_else(|_| {
                        GreetdResult::Error("login thread panicked".into())
                    })
                },
                Message::LoginResult,
            )
        }
        Message::LoginResult(result) => {
            state.logging_in = false;
            match result {
                GreetdResult::Success => {
                    std::process::exit(0);
                }
                GreetdResult::Error(e) => {
                    state.error = Some(e);
                    state.password.clear();
                    focus_widget("password")
                }
            }
        }
        Message::PowerOff => {
            let _ = Command::new("systemctl").arg("poweroff").spawn();
            Task::none()
        }
        Message::Reboot => {
            let _ = Command::new("systemctl").arg("reboot").spawn();
            Task::none()
        }
        Message::KeyboardEvent(event) => match event {
            keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Tab),
                ..
            } => match state.focus {
                Focus::Username => {
                    state.focus = Focus::Password;
                    focus_widget("password")
                }
                Focus::Password => {
                    state.focus = Focus::Username;
                    focus_widget("username")
                }
            },
            keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Enter),
                ..
            } => {
                if !state.logging_in {
                    update(state, Message::Login)
                } else {
                    Task::none()
                }
            }
            _ => Task::none(),
        },
        // iced_layershell action variants — not used but required by the macro
        _ => Task::none(),
    }
}

fn view(state: &Greeter) -> Element<'_, Message> {
    let username_input = text_input("Username", &state.username)
        .id(iced::widget::Id::new("username"))
        .on_input(Message::UsernameChanged)
        .padding(12)
        .size(16);

    let password_input = text_input("Password", &state.password)
        .id(iced::widget::Id::new("password"))
        .on_input(Message::PasswordChanged)
        .secure(true)
        .padding(12)
        .size(16);

    let session_picker = pick_list(
        state.sessions.as_slice(),
        state.selected_session.as_ref(),
        Message::SessionSelected,
    )
    .padding(12)
    .width(Length::Fill);

    let login_btn = button(
        text(if state.logging_in {
            "Logging in..."
        } else {
            "Login"
        })
        .width(Length::Fill)
        .align_x(alignment::Horizontal::Center),
    )
    .width(Length::Fill)
    .padding(12)
    .style(|_theme: &Theme, _status| button::Style {
        background: Some(Background::Color(color!(0x64, 0x8C, 0xF0, 0.8))),
        text_color: Color::WHITE,
        border: Border {
            radius: 10.0.into(),
            width: 1.0,
            color: color!(0x8C, 0xAA, 0xFF, 0.4),
        },
        shadow: Shadow::default(),
        snap: false,
    })
    .on_press_maybe(if state.logging_in {
        None
    } else {
        Some(Message::Login)
    });

    let error_text: Element<Message> = if let Some(ref err) = state.error {
        container(text(err).color(color!(0xFF, 0x88, 0x88)).size(14))
            .padding(8)
            .into()
    } else {
        column![].into()
    };

    let destructive_style = |_theme: &Theme, _status| button::Style {
        background: Some(Background::Color(color!(0xC8, 0x3C, 0x3C, 0.5))),
        text_color: Color::WHITE,
        border: Border {
            radius: 10.0.into(),
            width: 1.0,
            color: color!(0xC8, 0x3C, 0x3C, 0.3),
        },
        shadow: Shadow::default(),
        snap: false,
    };

    let power_row = row![
        button(text("Power Off").size(13))
            .on_press(Message::PowerOff)
            .padding([8, 16])
            .style(destructive_style),
        button(text("Reboot").size(13))
            .on_press(Message::Reboot)
            .padding([8, 16])
            .style(destructive_style),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    // Glass card
    let card = container(
        column![
            text("Welcome").size(28).color(Color::WHITE),
            username_input,
            password_input,
            session_picker,
            login_btn,
            error_text,
            power_row,
        ]
        .spacing(16)
        .align_x(Alignment::Center)
        .width(Length::Fill),
    )
    .width(450)
    .padding(32)
    .style(|_theme: &Theme| container::Style {
        background: Some(Background::Color(color!(0x14, 0x14, 0x1E, 0.55))),
        border: Border {
            radius: 20.0.into(),
            width: 1.0,
            color: color!(0xFF, 0xFF, 0xFF, 0.15),
        },
        shadow: Shadow {
            color: color!(0x00, 0x00, 0x00, 0.5),
            offset: iced::Vector::new(0.0, 8.0),
            blur_radius: 32.0,
        },
        text_color: Some(Color::WHITE),
        snap: false,
    });

    // Center on screen
    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(Color::TRANSPARENT)),
            ..Default::default()
        })
        .into()
}

fn subscription(_state: &Greeter) -> iced::Subscription<Message> {
    keyboard::listen().map(Message::KeyboardEvent)
}

fn style(_state: &Greeter, _theme: &Theme) -> iced::theme::Style {
    iced::theme::Style {
        background_color: Color::TRANSPARENT,
        text_color: Color::WHITE,
    }
}

fn main() -> iced_layershell::Result {
    iced_layershell::application(boot, namespace, update, view)
        .style(style)
        .subscription(subscription)
        .settings(Settings {
            layer_settings: LayerShellSettings {
                anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
                layer: Layer::Top,
                exclusive_zone: -1,
                keyboard_interactivity: KeyboardInteractivity::Exclusive,
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}
