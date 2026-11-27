<div align="center">

<img src="data/io.github.neryaathlan.Damx.svg" width="128" height="128" alt="Modern DAMX logo">

# Modern DAMX

**Acer Nitro and Predator controls for Linux.**

Tune your cooling, choose separate power profiles for AC and battery, and bring
back your keyboard lighting before you sign in.

[Download a release](https://github.com/neryaAthlan/modern-damx/releases) · [Install from source](#build-from-source) · [Boot preset](#boot-preset) · [Command line](#command-line)

</div>

Modern DAMX is a native Rust rewrite of Div Acer Manager Max for Debian and
recent GNOME desktops. The GTK 4 and libadwaita interface follows your system
theme. Use the desktop app for everyday controls or the `damx` CLI for scripts
and headless machines. [Original Div Acer Manager Max](https://github.com/PXDiv/Div-Acer-Manager-Max)

## Controls

| | |
| --- | --- |
| **Cooling** | Let the embedded controller manage the fans or set CPU and GPU fan speeds separately. Read temperatures and RPM live. |
| **Power** | Choose a profile for AC and another for battery. Modern DAMX switches between them as you plug in or unplug. |
| **Keyboard** | Set four RGB zones, brightness, and the idle timeout on supported keyboards. Keyboard activity restores lighting after an idle timeout. |
| **Battery** | Toggle the firmware's 80% charge limit on supported models. |
| **Boot preset** | Restore your selected lighting, fan speeds, and power profile before the login screen, with the app closed. |

## Install

Modern DAMX targets Debian 13. Other distributions need GTK 4.16+ and
libadwaita 1.6+ for the GUI. Hardware controls require the `linuwu_sense`
module and vary by Acer model. The driver fork documents tested hardware and
kernel requirements. [Div-Linuwu-Sense](https://github.com/neryaAthlan/Div-Linuwu-Sense)

### Install a release

Download the Linux archive from the [releases page](https://github.com/neryaAthlan/modern-damx/releases),
verify it against `SHA256SUMS`, and extract it. Run this inside the extracted
directory to install its bundled binaries. If the original DAMX daemon is
installed, disable it before installing Modern DAMX:

```sh
sudo systemctl disable --now damx-daemon.service
```

Install from the extracted directory:

```sh
sudo ./install.sh
```

The installer starts `damxd.service` and adds your user to the
`linuwu_sense` group. Log out and back in if your group membership changed,
then open **Modern DAMX** from the application menu.

### Build from source

Use Rust 1.85 or newer. On Debian 13:

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev curl
git clone https://github.com/neryaAthlan/modern-damx.git
cd modern-damx
cargo build --release --locked --workspace
sudo ./install.sh --use-built
```

Use the source build to try changes from `main` before they appear in a
release.

### Installer options

| Option | Behavior |
| --- | --- |
| `--use-built` | Install the binaries from `target/release`. |
| `--build` | Build from source and install in one step. |
| `--daemon-only` | Install the daemon and CLI for headless use. |
| `--with-driver` | Build and install Linuwu Sense from a local driver checkout. Use `--driver-dir /path/to/Div-Linuwu-Sense` to select it explicitly. |
| `--uninstall` | Remove Modern DAMX and keep the driver and boot preset. |

The existing kernel module is only replaced when you pass `--with-driver`.
If Secure Boot is enabled, the module needs a signature trusted by your system.

## Boot preset

Open **Boot preset** and enable the settings you want restored. When you change
a selected setting elsewhere in Modern DAMX, the preset updates with it. To
restore keyboard lighting alone, enable **Keyboard lighting** and leave the
other fields off.

Systemd runs the restore before starting `damxd` and before the display manager.
Modern DAMX uses direct HID for ENEK5130 keyboards and Linuwu Sense sysfs for
other supported four-zone keyboards. It tries the saved settings immediately
and gives unavailable hardware a short retry window. A missed setting is logged
and startup continues. The RGB boot restore was verified on an Acer Nitro V 16S
(ANV16S-71).

The preset lives at `/etc/damxd/boot.toml`. Omitted fields are skipped by
the preset. Your AC and battery profile preferences still apply separately.

```toml
enabled = true

[lighting]
zones = ["ffffff", "ffffff", "ffffff", "ffffff"]
brightness = 80
```

To check what happened on the current boot:

```sh
damx boot show
journalctl -b -u damxd.service
```

## Command line

```sh
damx status
damx status --json
damx profiles ac performance
damx profiles battery balanced
damx fan auto
damx fan 45 45
damx light ff0000 00ff00 0000ff ffffff 80
damx boot save
damx boot apply
damx update
```

The CLI and GUI send privileged writes to `damxd` over a group-restricted Unix
socket. The GUI can read status when the daemon is stopped. Root can also use
the CLI to write directly when the daemon is unavailable.

## Settings and compatibility

| Setting | Details |
| --- | --- |
| **Temperatures** | Readings come from the Acer embedded controller, with `nvidia-smi` as a GPU fallback. Choose Celsius or Fahrenheit under **Status → Preferences**. The CLI uses the same preference. |
| **Power profiles** | AC and battery choices live in `/etc/damxd/profiles.toml`. GNOME can also change the active profile. Modern DAMX reapplies its preference after `power-profiles-daemon` starts and when the power source changes. |
| **Updates** | The **Updates** page checks published app and driver releases or tags. The user timer checks daily when enabled. Repository choices live in `~/.config/damx/config.toml`. |
| **Model support** | Fan, battery and RGB controls depend on the laptop and loaded driver. Battery calibration and LCD override are outside the Modern DAMX GUI. |

## Project layout

| Component | Role |
| --- | --- |
| `crates/damx-gui` | GTK 4 and libadwaita desktop app |
| `crates/damx-cli` | Command line tool and system boot restore |
| `crates/damx-daemon` | Privileged helper and power source monitoring |
| `crates/damx-core` | Hardware access, configuration, IPC and update checks |
| `data/` | Systemd units, desktop entry and icon |

## Credits

Modern DAMX and the driver fork are maintained by Nerya Athlan. The project
builds on the work of PXDiv and 0x7375646F. [Nerya Athlan](https://github.com/neryaAthlan) · [Div Acer Manager Max](https://github.com/PXDiv/Div-Acer-Manager-Max) · [Linuwu Sense](https://github.com/0x7375646F/Linuwu-Sense)

[License](LICENSE)
