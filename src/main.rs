mod config;

use std::env;
use std::fs;
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use config::{hex_to_color, Config};
use futures::StreamExt;
use greetd_ipc::codec::SyncCodec;
use greetd_ipc::{AuthMessageType, Request, Response};
use iced::widget::{button, column, container, pick_list, row, text, text_input};
use iced::{
    alignment, color, keyboard, Alignment, Background, Border, Color, Element, Length, Shadow,
    Task, Theme,
};
use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
use iced_layershell::settings::{LayerShellSettings, Settings};
use iced_layershell::to_layer_message;

// ── Locale helpers ─────────────────────────────────────────────────

fn locale_preferences() -> Vec<String> {
    let raw = env::var("LC_ALL")
        .or_else(|_| env::var("LC_MESSAGES"))
        .or_else(|_| env::var("LANG"))
        .unwrap_or_default();
    let base = raw
        .split(['.', '@'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let mut prefs = Vec::new();
    if !base.is_empty() {
        prefs.push(base.clone());
        if let Some(lang) = base.split('_').next() {
            if lang != base && !lang.is_empty() {
                prefs.push(lang.to_string());
            }
        }
    }
    prefs
}

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

fn detect_sessions(dirs: &[String]) -> Vec<Session> {
    let locale_prefs = locale_preferences();
    let mut sessions = Vec::new();

    for dir in dirs {
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

            let mut default_name: Option<String> = None;
            let mut localized_names: Vec<(String, String)> = Vec::new();
            let mut exec: Option<String> = None;
            let mut try_exec: Option<String> = None;
            let mut hidden = false;
            let mut no_display = false;
            let mut in_entry = false;

            for line in contents.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if trimmed.starts_with('[') && trimmed.ends_with(']') {
                    in_entry = trimmed == "[Desktop Entry]";
                    continue;
                }
                if !in_entry {
                    continue;
                }

                if let Some(rest) = trimmed.strip_prefix("Name") {
                    if let Some(inside) = rest.strip_prefix('[') {
                        if let Some(end) = inside.find(']') {
                            let locale = &inside[..end];
                            if let Some(v) = inside[end + 1..].strip_prefix('=') {
                                localized_names.push((locale.to_string(), v.to_string()));
                            }
                        }
                    } else if let Some(v) = rest.strip_prefix('=') {
                        default_name = Some(v.to_string());
                    }
                } else if let Some(v) = trimmed.strip_prefix("Exec=") {
                    exec = Some(v.to_string());
                } else if let Some(v) = trimmed.strip_prefix("TryExec=") {
                    try_exec = Some(v.to_string());
                } else if let Some(v) = trimmed.strip_prefix("Hidden=") {
                    hidden = v.eq_ignore_ascii_case("true");
                } else if let Some(v) = trimmed.strip_prefix("NoDisplay=") {
                    no_display = v.eq_ignore_ascii_case("true");
                }
            }

            if hidden || no_display {
                continue;
            }
            if let Some(ref tx) = try_exec {
                if !binary_exists(tx) {
                    continue;
                }
            }

            let name = locale_prefs
                .iter()
                .find_map(|l| {
                    localized_names
                        .iter()
                        .find_map(|(k, v)| if k == l { Some(v.clone()) } else { None })
                })
                .or(default_name);

            if let (Some(name), Some(exec)) = (name, exec) {
                sessions.push(Session { name, exec });
            }
        }
    }

    sessions.sort_by(|a, b| a.name.cmp(&b.name));
    sessions.dedup_by(|a, b| a.exec == b.exec);
    sessions
}

fn binary_exists(bin: &str) -> bool {
    let path = std::path::Path::new(bin);
    if path.is_absolute() {
        return path.is_file();
    }
    if let Ok(path_env) = env::var("PATH") {
        for dir in path_env.split(':') {
            if std::path::Path::new(dir).join(bin).is_file() {
                return true;
            }
        }
    }
    false
}

// ── greetd auth worker ─────────────────────────────────────────────

#[derive(Debug, Clone)]
enum AuthEvent {
    Prompt { text: String, secret: bool },
    Info(String),
    Error(String),
    Completed,
    Failed(String),
}

/// Best-effort CancelSession so greetd's server-side state is cleared. Without
/// this, a subsequent CreateSession is rejected and the greeter appears stuck.
fn send_cancel(stream: &mut UnixStream) {
    let _ = Request::CancelSession.write_to(stream);
    let _ = Response::read_from(stream);
}

/// Full PAM conversation with greetd. Runs on a dedicated thread; prompts are
/// surfaced via `event_tx` and user responses come back via `response_rx`.
fn run_auth_worker(
    username: String,
    session_cmd: Vec<String>,
    response_rx: mpsc::Receiver<String>,
    event_tx: futures::channel::mpsc::UnboundedSender<AuthEvent>,
) {
    macro_rules! emit {
        ($e:expr) => {{
            if event_tx.unbounded_send($e).is_err() {
                return;
            }
        }};
    }

    let sock_path = match env::var("GREETD_SOCK") {
        Ok(p) => p,
        Err(_) => {
            emit!(AuthEvent::Failed("GREETD_SOCK not set".into()));
            return;
        }
    };
    let mut stream = match UnixStream::connect(&sock_path) {
        Ok(s) => s,
        Err(e) => {
            emit!(AuthEvent::Failed(format!("connect: {e}")));
            return;
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));

    if let Err(e) = (Request::CreateSession { username }).write_to(&mut stream) {
        emit!(AuthEvent::Failed(format!("write create: {e}")));
        return;
    }

    loop {
        let resp = match Response::read_from(&mut stream) {
            Ok(r) => r,
            Err(e) => {
                emit!(AuthEvent::Failed(format!("read: {e}")));
                return;
            }
        };
        match resp {
            Response::Success => break,
            Response::Error { description, .. } => {
                send_cancel(&mut stream);
                emit!(AuthEvent::Failed(description));
                return;
            }
            Response::AuthMessage {
                auth_message_type,
                auth_message,
            } => {
                let reply = match auth_message_type {
                    AuthMessageType::Secret | AuthMessageType::Visible => {
                        let secret = matches!(auth_message_type, AuthMessageType::Secret);
                        emit!(AuthEvent::Prompt {
                            text: auth_message.clone(),
                            secret,
                        });
                        match response_rx.recv() {
                            Ok(s) => Some(s),
                            Err(_) => {
                                send_cancel(&mut stream);
                                return;
                            }
                        }
                    }
                    AuthMessageType::Info => {
                        eprintln!("[barrgreet] pam info: {auth_message}");
                        emit!(AuthEvent::Info(auth_message));
                        None
                    }
                    AuthMessageType::Error => {
                        eprintln!("[barrgreet] pam error: {auth_message}");
                        emit!(AuthEvent::Error(auth_message));
                        None
                    }
                };
                let req = Request::PostAuthMessageResponse { response: reply };
                if let Err(e) = req.write_to(&mut stream) {
                    send_cancel(&mut stream);
                    emit!(AuthEvent::Failed(format!("write auth: {e}")));
                    return;
                }
            }
        }
    }

    let req = Request::StartSession {
        cmd: session_cmd,
        env: Vec::new(),
    };
    if let Err(e) = req.write_to(&mut stream) {
        send_cancel(&mut stream);
        emit!(AuthEvent::Failed(format!("write start: {e}")));
        return;
    }
    match Response::read_from(&mut stream) {
        Ok(Response::Success) => emit!(AuthEvent::Completed),
        Ok(Response::Error { description, .. }) => {
            send_cancel(&mut stream);
            emit!(AuthEvent::Failed(description));
        }
        Ok(_) => {
            send_cancel(&mut stream);
            emit!(AuthEvent::Failed("unexpected response after start".into()));
        }
        Err(e) => emit!(AuthEvent::Failed(format!("read start: {e}"))),
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
    Auth(AuthEvent),
    PowerOff,
    Reboot,
    KeyboardEvent(keyboard::Event),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusedWidget {
    Username,
    Password,
}

impl FocusedWidget {
    fn toggle(self) -> Self {
        match self {
            FocusedWidget::Username => FocusedWidget::Password,
            FocusedWidget::Password => FocusedWidget::Username,
        }
    }
    fn id(self) -> &'static str {
        match self {
            FocusedWidget::Username => "username",
            FocusedWidget::Password => "password",
        }
    }
}

#[derive(Debug, Clone)]
struct AuthPrompt {
    text: String,
    secret: bool,
}

struct Greeter {
    username: String,
    password: String,
    sessions: Vec<Session>,
    selected_session: Option<Session>,
    error: Option<String>,
    pam_messages: Vec<String>,
    auth_prompt: Option<AuthPrompt>,
    pending_initial_response: Option<String>,
    response_tx: Option<mpsc::Sender<String>>,
    logging_in: bool,
    caps_lock: bool,
    focus: FocusedWidget,
    config: Config,
}

fn focus_task(name: &'static str) -> Task<Message> {
    iced::widget::operation::focus(iced::widget::Id::new(name))
}

// ── Application functions ──────────────────────────────────────────

fn boot() -> (Greeter, Task<Message>) {
    let config = Config::load();
    let sessions = detect_sessions(&config.general.session_dirs);
    let selected = sessions.first().cloned();
    (
        Greeter {
            username: String::new(),
            password: String::new(),
            sessions,
            selected_session: selected,
            error: None,
            pam_messages: Vec::new(),
            auth_prompt: None,
            pending_initial_response: None,
            response_tx: None,
            logging_in: false,
            caps_lock: false,
            focus: FocusedWidget::Username,
            config,
        },
        focus_task("username"),
    )
}

fn namespace() -> String {
    "barrgreet".to_string()
}

fn start_auth_session(state: &mut Greeter) -> Task<Message> {
    if state.username.is_empty() {
        state.error = Some("Username is required".into());
        return Task::none();
    }
    let Some(session) = state.selected_session.clone() else {
        state.error = Some("No session selected".into());
        return Task::none();
    };

    state.logging_in = true;
    state.error = None;
    state.pam_messages.clear();
    state.auth_prompt = None;

    let (event_tx, event_rx) = futures::channel::mpsc::unbounded::<AuthEvent>();
    let (resp_tx, resp_rx) = mpsc::channel::<String>();
    state.response_tx = Some(resp_tx);
    // Auto-submit the pre-typed password on the first Secret/Visible prompt
    // so the normal password-only flow requires a single click, not two.
    state.pending_initial_response = Some(std::mem::take(&mut state.password));

    let username = state.username.clone();
    let cmd: Vec<String> = vec!["sh".into(), "-c".into(), session.exec.clone()];

    std::thread::spawn(move || {
        run_auth_worker(username, cmd, resp_rx, event_tx);
    });

    Task::stream(event_rx.map(Message::Auth))
}

fn submit_prompt_response(state: &mut Greeter) {
    if let Some(tx) = state.response_tx.as_ref() {
        let response = std::mem::take(&mut state.password);
        let _ = tx.send(response);
        state.auth_prompt = None;
    }
}

fn reset_auth_state(state: &mut Greeter) {
    state.logging_in = false;
    state.pending_initial_response = None;
    state.auth_prompt = None;
    state.response_tx = None;
    state.password.clear();
}

fn update(state: &mut Greeter, message: Message) -> Task<Message> {
    match message {
        Message::UsernameChanged(u) => {
            if !state.logging_in {
                state.username = u;
                state.error = None;
            }
            Task::none()
        }
        Message::PasswordChanged(p) => {
            state.password = p;
            state.error = None;
            Task::none()
        }
        Message::SessionSelected(s) => {
            if !state.logging_in {
                state.selected_session = Some(s);
            }
            Task::none()
        }
        Message::Login => {
            if state.logging_in {
                if state.auth_prompt.is_some() {
                    submit_prompt_response(state);
                }
                Task::none()
            } else {
                start_auth_session(state)
            }
        }
        Message::Auth(evt) => match evt {
            AuthEvent::Prompt { text, secret } => {
                state.auth_prompt = Some(AuthPrompt { text, secret });
                if let Some(initial) = state.pending_initial_response.take() {
                    if let Some(tx) = state.response_tx.as_ref() {
                        let _ = tx.send(initial);
                        state.auth_prompt = None;
                    }
                    Task::none()
                } else {
                    state.focus = FocusedWidget::Password;
                    focus_task("password")
                }
            }
            AuthEvent::Info(m) | AuthEvent::Error(m) => {
                state.pam_messages.push(m);
                Task::none()
            }
            AuthEvent::Completed => {
                std::process::exit(0);
            }
            AuthEvent::Failed(err) => {
                eprintln!("[barrgreet] login failed: {err}");
                state.error = Some(err);
                reset_auth_state(state);
                state.focus = FocusedWidget::Password;
                focus_task("password")
            }
        },
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
            } => {
                state.focus = state.focus.toggle();
                focus_task(state.focus.id())
            }
            keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Enter),
                ..
            } => update(state, Message::Login),
            keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::CapsLock),
                ..
            } => {
                state.caps_lock = !state.caps_lock;
                Task::none()
            }
            _ => Task::none(),
        },
        _ => Task::none(),
    }
}

fn view(state: &Greeter) -> Element<'_, Message> {
    let cfg = &state.config;
    let style_cfg = &cfg.style;
    let layout_cfg = &cfg.layout;

    let text_color = hex_to_color(&style_cfg.text_color, 1.0);
    let error_color = hex_to_color(&style_cfg.error_color, 1.0);
    let button_bg = hex_to_color(&style_cfg.button_color, style_cfg.button_opacity);
    let button_border_color = hex_to_color(&style_cfg.button_color, style_cfg.button_opacity * 0.5);
    let destructive_bg = hex_to_color(&style_cfg.destructive_color, style_cfg.destructive_opacity);
    let destructive_border_color =
        hex_to_color(&style_cfg.destructive_color, style_cfg.destructive_opacity * 0.6);
    let card_bg = hex_to_color(&style_cfg.background_color, style_cfg.background_opacity);
    let card_border_color = hex_to_color(&style_cfg.border_color, style_cfg.border_opacity);
    let card_border_width = style_cfg.border_width;
    let card_border_radius = layout_cfg.card_border_radius;

    let username_input = text_input("Username", &state.username)
        .id(iced::widget::Id::new("username"))
        .on_input(Message::UsernameChanged)
        .padding(12)
        .size(16);

    let (password_placeholder, password_secure): (&str, bool) = match state.auth_prompt.as_ref() {
        Some(p) => (p.text.as_str(), p.secret),
        None => ("Password", true),
    };
    let password_input = text_input(password_placeholder, &state.password)
        .id(iced::widget::Id::new("password"))
        .on_input(Message::PasswordChanged)
        .secure(password_secure)
        .padding(12)
        .size(16);

    let caps_indicator: Element<Message> = if state.caps_lock {
        text("⚠ Caps Lock is on").size(12).color(error_color).into()
    } else {
        text("").into()
    };

    let pam_text: Element<Message> = if state.pam_messages.is_empty() {
        text("").into()
    } else {
        let msg = state.pam_messages.join("\n");
        container(text(msg).color(text_color).size(13))
            .padding(8)
            .into()
    };

    let session_picker = pick_list(
        state.sessions.as_slice(),
        state.selected_session.as_ref(),
        Message::SessionSelected,
    )
    .padding(12)
    .width(Length::Fill);

    let login_label = if !state.logging_in {
        "Login"
    } else if state.auth_prompt.is_some() {
        "Submit"
    } else {
        "Authenticating..."
    };
    let login_button_active = !state.logging_in || state.auth_prompt.is_some();

    let login_btn = button(
        text(login_label)
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Center),
    )
    .width(Length::Fill)
    .padding(12)
    .style(move |_theme: &Theme, _status| button::Style {
        background: Some(Background::Color(button_bg)),
        text_color: Color::WHITE,
        border: Border {
            radius: 10.0.into(),
            width: 1.0,
            color: button_border_color,
        },
        ..Default::default()
    })
    .on_press_maybe(if login_button_active {
        Some(Message::Login)
    } else {
        None
    });

    let error_text: Element<Message> = if let Some(ref err) = state.error {
        container(text(err).color(error_color).size(14))
            .padding(8)
            .into()
    } else {
        text("").into()
    };

    let destructive_style = move |_theme: &Theme, _status| button::Style {
        background: Some(Background::Color(destructive_bg)),
        text_color: Color::WHITE,
        border: Border {
            radius: 10.0.into(),
            width: 1.0,
            color: destructive_border_color,
        },
        ..Default::default()
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

    let card = container(
        column![
            text(&cfg.general.welcome_text)
                .size(28)
                .color(text_color),
            username_input,
            password_input,
            caps_indicator,
            pam_text,
            session_picker,
            login_btn,
            error_text,
            power_row,
        ]
        .spacing(layout_cfg.spacing as f32)
        .align_x(Alignment::Center)
        .width(Length::Fill),
    )
    .width(layout_cfg.card_width as f32)
    .padding(layout_cfg.card_padding as f32)
    .style(move |_theme: &Theme| container::Style {
        background: Some(Background::Color(card_bg)),
        border: Border {
            radius: card_border_radius.into(),
            width: card_border_width,
            color: card_border_color,
        },
        shadow: Shadow {
            color: color!(0x00, 0x00, 0x00, 0.5),
            offset: iced::Vector::new(0.0, 8.0),
            blur_radius: 32.0,
        },
        text_color: Some(text_color),
        snap: false,
    });

    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: layout_cfg.margin_top as f32,
            right: layout_cfg.margin_right as f32,
            bottom: layout_cfg.margin_bottom as f32,
            left: layout_cfg.margin_left as f32,
        })
        .align_x(layout_cfg.position.horizontal())
        .align_y(layout_cfg.position.vertical())
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
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "--init") {
        print!("{}", config::DEFAULT_CONFIG);
        std::process::exit(0);
    }

    eprintln!("[barrgreet] starting");

    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        eprintln!("[barrgreet] WAYLAND_DISPLAY={display}");
    } else {
        eprintln!("[barrgreet] WARNING: WAYLAND_DISPLAY is not set");
    }

    match env::var("GREETD_SOCK") {
        Ok(sock) => eprintln!("[barrgreet] GREETD_SOCK={sock}"),
        Err(_) => eprintln!("[barrgreet] WARNING: GREETD_SOCK is not set — login will fail"),
    }

    let config = Config::load();

    let sessions = detect_sessions(&config.general.session_dirs);
    if sessions.is_empty() {
        eprintln!(
            "[barrgreet] WARNING: no sessions found in: {}",
            config.general.session_dirs.join(", ")
        );
    } else {
        eprintln!(
            "[barrgreet] found {} session(s): {}",
            sessions.len(),
            sessions
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    eprintln!("[barrgreet] launching layer-shell UI");

    let result = iced_layershell::application(boot, namespace, update, view)
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
        .run();

    if let Err(ref e) = result {
        eprintln!("[barrgreet] ERROR: {e}");
    }

    result
}
