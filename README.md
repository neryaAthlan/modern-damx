# Modern DAMX

A small, native manager for Acer Nitro/Predator laptops on Linux, written in
Rust for Debian and recent GNOME desktops. It is a modern rewrite of the parts of
[Div Acer Manager Max](https://github.com/PXDiv/Div-Acer-Manager-Max) that the
[Linuwu Sense](https://github.com/neryaAthlan/Div-Linuwu-Sense) driver actually
supports, with one addition the original does not have: a **boot preset** that
re-applies your power profile, fan speed and keyboard lighting at startup,
without opening any window.

The GUI is GTK 4 + libadwaita, so it follows the GNOME look and the system
light/dark theme.

## Features

| Area | What it does |
| --- | --- |
| Status | Live driver version, model, kernel, power source, CPU/GPU temperatures, current profile, fan speeds, lighting, battery limiter and backlight timeout |
| Temperatures | CPU and GPU readings from the Acer embedded controller, with the NVIDIA driver as a fallback; Celsius or Fahrenheit preference |
| Power profile | All five profiles the driver exposes: `low-power`, `quiet`, `balanced`, `balanced-performance`, `performance` |
| Battery limit | On battery the profile is capped (default `balanced`); plugging in makes every profile available again |
| Fan speed | Automatic (embedded controller) or manual CPU/GPU percentage, with live RPM readouts on both the status and fan pages |
| Keyboard | Per-zone colours for the four-zone RGB keyboard, brightness, and the 30-second backlight timeout |
| Battery | Battery limiter (charge to 80%) toggle |
| Boot preset | Stores profile, fan speed and/or lighting and applies them at every boot via `damxd` |
| Updates | Checks GitHub for new releases of this app and of the driver fork, on demand and once a day |
| Credits | Author and upstream projects, linked from inside the app |

Everything is also available from the command line (`damx`).

## Requirements

* Debian 13 (trixie) or another distribution with GTK 4.16+ and libadwaita 1.6+
* Rust 1.85 or newer to build
* The `linuwu_sense` kernel module from
  [neryaAthlan/Div-Linuwu-Sense](https://github.com/neryaAthlan/Div-Linuwu-Sense),
  loaded and bound to the Acer WMI device
* Membership of the `linuwu_sense` group (created by the driver installer)

The GUI never asks for a password: privileged writes go through `damxd`, a
small root daemon with a group-restricted Unix socket. This is needed because
the driver's sysfs attributes are owned by root and sysfs permissions cannot be
changed after boot.

## Build and install

```bash
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev curl
git clone https://github.com/neryaAthlan/modern-damx
cd modern-damx
sudo ./install.sh
```

Every `v*` tag is built by GitHub Actions on Debian 13; the release page
carries the three binaries, a checksum file and a self-contained tarball (source
and binaries) for offline installs.

The installer has modes:

```bash
sudo ./install.sh                        # GUI, CLI and daemon (default)
sudo ./install.sh --daemon-only          # daemon + CLI only, no GUI
sudo ./install.sh --with-driver          # also install the Linuwu Sense module
sudo ./install.sh --with-driver --driver-dir /path/to/Div-Linuwu-Sense
sudo ./install.sh --uninstall            # remove Modern DAMX (keeps the preset)
```

`--with-driver` runs `linuwu-sense.sh install` from your driver checkout, so the
kernel module is rebuilt for the running kernel and registered with DKMS.
Without it, the existing module is left alone.

The default install adds the binaries to `/usr/bin`, installs and starts
`damxd.service`, adds the desktop entry and icon, and enables a daily user timer
for the update check.

You can also run the GUI from a checkout without installing it, but changing
settings then requires `damxd` to be installed and running:

```bash
cargo run --release -p damx-gui
```

## Command line

```bash
damx status                      # human readable state (--json for scripts)
damx profile performance         # low-power | quiet | balanced | balanced-performance | performance
damx fan auto                    # hand the fans back to the EC
damx fan 45 45                   # CPU 45%, GPU 45%
damx light ff0000 00ff00 0000ff ffffff 80   # four zones + brightness
damx boot save                   # store the current settings as the boot preset
damx boot show                   # print the preset
damx boot enable | disable
damx boot apply                  # apply it right now
damx policy                      # show the battery profile limit

damx policy off | balanced       # disable, or cap at another profile
damx update                      # check GitHub for driver and app releases
```

Commands that write go through `damxd`. If the daemon is not running, `damx`
falls back to writing sysfs directly **only when run as root**.

## Boot preset

The **Boot preset** page in the GUI lets you choose which of the three settings
are re-applied at boot:

* power profile
* fan speed
* keyboard lighting

`damxd` waits for the kernel module (up to 60 seconds after boot), then applies
the preset as root, before anyone logs in. This means the laptop comes up the
way you left it and you never need to open the GUI for that.

The preset is stored in `/etc/damxd/boot.toml`:

```toml
enabled = true
profile = "balanced"

[fan]
cpu = 35
gpu = 35

[lighting]
zones = ["ff0000", "00ff00", "0000ff", "ffffff"]
brightness = 80
```

Every field is optional; omit a key (or turn its switch off in the GUI) to leave
that setting untouched at boot. The daemon unit is ordered after
`power-profiles-daemon.service` so the preset is applied after GNOME has chosen
its own power mode.

## Temperatures and units

The **Status** and **Performance** pages show CPU and GPU temperatures. They
come from the driver's own hwmon sensors (the Acer embedded controller), and the
GPU value falls back to `nvidia-smi` when the dGPU sensor is asleep.

The unit is chosen under **Status → Preferences** and stored in
`~/.config/damx/config.toml`:

```toml
[ui]
temperature_unit = "celsius" # or "fahrenheit"
```

The CLI honours the same setting (`damx status`).

## Battery profile limit

The policy is enforced by `damxd`, so it is global: the GUI only offers the
allowed profiles, the CLI refuses a higher one, and unplugging the charger
immediately switches the profile down. The boot preset obeys it too.

```toml
# /etc/damxd/policy.toml
limit_on_battery = true
battery_profile = "balanced"
```

On battery only profiles up to `battery_profile` are selectable; while the
charger is connected every profile is available. Change it from
**Performance → Battery profile limit** or with `damx policy`.

## Update checks

The app checks the latest published release or tag of two repositories:

```toml
# ~/.config/damx/config.toml
[updates]
driver_repo = "neryaAthlan/Div-Linuwu-Sense"
gui_repo = "neryaAthlan/modern-damx"
```

The defaults point at this project. Change them if you publish under other
names. Checks use the GitHub API through `curl`, run once a day from
`damx-update-check.timer` (per user), and whenever the **Updates** page is
opened. Results are cached in `~/.cache/damx/update.json` so the page opens
instantly.

If a repository has no releases or tags yet, the page says so instead of
guessing from branch commits — publish a tag (`v0.1.0`) to make updates visible.

## Credits

* **Nerya Athlan** ([neryaAthlan](https://github.com/neryaAthlan)) — author and
  maintainer of this rewrite and of the Div-Linuwu-Sense fork.
* **PXDiv** — [Div Acer Manager Max](https://github.com/PXDiv/Div-Acer-Manager-Max),
  the original GUI this rewrite is based on.
* **0x7375646F** — [Linuwu Sense](https://github.com/0x7375646F/Linuwu-Sense),
  the kernel driver that exposes fan, power and keyboard controls.

The same list is shown on the **Credits** tab of the app.

## Layout

```
crates/damx-core     sysfs driver access, boot preset, IPC protocol, update check
crates/damx-daemon   damxd: root helper, applies the boot preset, owns the socket
crates/damx-cli      damx: command line interface
crates/damx-gui      damx-gui: GTK 4 / libadwaita application
data/                systemd units, desktop entry, icon
install.sh           system-wide installer with GUI/daemon/driver modes
```

## Notes and limitations

* **GNOME power mode vs. the driver profile.** power-profiles-daemon owns
  `platform_profile` and can rewrite it when the power source changes. The boot
  preset is applied after it at boot, and `damxd` re-applies the battery cap
  after unplugging, but changing the GNOME power mode directly may still move
  the profile. This is a conflict between the driver and PPD, not something the
  GUI can solve without disabling PPD.
* **LCD override and battery calibration** are exposed by some driver builds but
  are deliberately left out of the GUI: `lcd_override` is not available on every
  model and battery calibration starts a multi-hour charge/drain cycle.
* **Secure Boot.** DKMS modules need to be signed (or Secure Boot disabled) for
  the driver to load. Modern DAMX only talks to an already-loaded driver.
* The GUI tests for a running daemon; without it, it shows a banner and works
  read-only.
