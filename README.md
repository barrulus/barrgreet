# barrgreet

A minimal login greeter for [greetd](https://sr.ht/~kennylevinsen/greetd/), built with Rust using [iced](https://iced.rs) and [iced_layershell](https://github.com/waycrate/exwlshelleern).

![screenshot](screenshot.png)

## Features

- Wayland-native via layer shell (covers the full screen on the `Top` layer)
- Auto-detects available Wayland and X11 sessions from `.desktop` files
- Glass-card UI with keyboard navigation (Tab to switch fields, Enter to login)
- Configurable layout, colors, and position via TOML config file
- Power off and reboot buttons

## Building

### With Nix (recommended)

```sh
nix build
```

### With Cargo

Requires `wayland`, `libxkbcommon`, and `vulkan-loader` development libraries.

```sh
cargo build --release
```

## Usage

Configure greetd to run barrgreet as its greeter. For example, in `/etc/greetd/config.toml`:

```toml
[default_session]
command = "barrgreet"
```

### NixOS

```nix
{
  services.greetd = {
    enable = true;
    settings.default_session.command = "${pkgs.barrgreet}/bin/barrgreet";
  };
}
```

Or if using the flake directly:

```nix
{
  services.greetd = {
    enable = true;
    settings.default_session.command = "${inputs.barrgreet.packages.x86_64-linux.default}/bin/barrgreet";
  };
}
```

## Running under a compositor (e.g. niri)

barrgreet has a transparent background, so you'll want a compositor to provide wallpaper and window management. Here's an example using [niri](https://github.com/YaLTeR/niri):

**greetd config** (NixOS `configuration.nix`):

```nix
services.greetd.enable = true;
services.greetd.settings.default_session.command = lib.mkForce
  "${pkgs.dbus}/bin/dbus-run-session ${lib.getExe pkgs.niri} --config /etc/greetd/niri-greeter.kdl";
```

**niri-greeter.kdl** (minimal example):

```kdl
hotkey-overlay {
    skip-at-startup
}

// Launch barrgreet, quit niri when it exits
spawn-at-startup "barrgreet" ; "niri" "msg" "action" "quit" "--skip-confirmation"
```

### Wallpaper

barrgreet is transparent by design — add a wallpaper process in your niri config.

**Animated wallpaper with [mpvpaper](https://github.com/GhostNaN/mpvpaper):**

```kdl
spawn-at-startup "mpvpaper" "*" "/path/to/wallpaper.gif" "-o" "no-audio --loop"
```

**Static wallpaper with [swaybg](https://github.com/swaywm/swaybg):**

```kdl
spawn-at-startup "swaybg" "-i" "/path/to/wallpaper.png" "-m" "fill"
```

### Troubleshooting

barrgreet logs startup diagnostics to stderr. Check the greetd journal for errors:

```sh
journalctl -u greetd -e
```

You should see output like:

```
[barrgreet] starting
[barrgreet] WAYLAND_DISPLAY=wayland-1
[barrgreet] GREETD_SOCK=/run/greetd.sock
[barrgreet] found 3 session(s): Niri, Plasma (Wayland), Sway
[barrgreet] launching layer-shell UI
```

## Configuration

barrgreet is configured via a TOML file at `/etc/barrgreet/config.toml`. All values are optional — barrgreet works with no config file, using sensible defaults.

Generate a default config file:

```sh
sudo mkdir -p /etc/barrgreet
barrgreet --init | sudo tee /etc/barrgreet/config.toml
```

Or use a custom path:

```sh
barrgreet -c /path/to/config.toml
```

See [`config.toml.example`](config.toml.example) for all available options. Key settings include:

- **`[layout]`** — card position (`center`, `top-left`, `bottom-right`, etc.), margins, card width/padding/border radius
- **`[style]`** — background, border, text, button, and error colors with opacity controls
- **`[general]`** — welcome text, session directory search paths

### NixOS

Place your config in your NixOS repo and add to `configuration.nix`:

```nix
environment.etc."barrgreet/config.toml".source = ./path/to/barrgreet.toml;
```

## Session Detection

Sessions are discovered from `.desktop` files in the directories listed in `general.session-dirs` (defaults shown):

- `/usr/share/wayland-sessions`
- `/usr/share/xsessions`
- `/run/current-system/sw/share/wayland-sessions` (NixOS)
- `/run/current-system/sw/share/xsessions` (NixOS)
- `/usr/local/share/wayland-sessions`
- `/usr/local/share/xsessions`

### NixOS session discovery

On NixOS, you need to symlink session `.desktop` files into the system profile so barrgreet can find them. Add to your `configuration.nix`:

```nix
environment.pathsToLink = [
  "/share/wayland-sessions"
  "/share/xsessions"
];
```

Some NixOS modules (e.g. `programs.steam.gamescopeSession`) register sessions via `services.displayManager.sessionPackages` rather than adding them to `environment.systemPackages`. Display managers like GDM/SDDM consume these automatically, but greetd does not. To make these sessions visible to barrgreet, add them to your system packages:

```nix
environment.systemPackages = config.services.displayManager.sessionPackages ++ [
  # ... your other packages
];
```

## License

[MIT](LICENSE)
