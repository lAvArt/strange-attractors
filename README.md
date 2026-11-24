# Attractor

Strange attractors as a **live wallpaper** and a **screensaver** for [Omarchy](https://omarchy.org), by lAvArt Studio.

Thousands of points are integrated through the Lorenz, Aizawa, Halvorsen, Thomas, Rössler, Chen-Lee, four-wing and
Dadras systems and drawn as glowing ribbons while a slow camera orbits them. Every few minutes one attractor fades out
and the next is born from a single point and grows into its shape.

![Eight attractors](docs/media/systems.jpg)

## Two modes, one binary

- **Wallpaper** (`attractor --wallpaper`): a `wlr-layer-shell` surface on the *bottom* layer, above Omarchy's own
  background and below every window. Clicks pass straight through to the desktop. It renders at half resolution and
  30 fps (20 on battery) and **pauses itself** when it cannot be seen: a fullscreen window, a locked session, or a
  sleeping monitor.
- **Screensaver** (`attractor`): a fullscreen window using Omarchy's screensaver class, so the idle service still locks on
  time and any key, click or mouse movement dismisses it. Like Gridfall it stays on the integrated GPU by default.

The plugin service starts the wallpaper at login and the screensaver after the system idle timeout
(`idle.screensaver` in `shell.json`).

## Installing

You'll need the Rust toolchain to build it once.

```bash
omarchy plugin add https://github.com/lAvArt/attractor
cargo build --release --locked --target-dir ~/.cache/attractor/build \
  --manifest-path ~/.config/omarchy/plugins/io.github.lavart.attractor/Cargo.toml
omarchy plugin enable io.github.lavart.attractor
```

Try it without the plugin: `attractor --windowed`, `attractor --wallpaper`, or render a still with
`attractor --shot out.png --system lorenz --size 1920x1080 --secs 30`.

## Looks

| preset | |
|---|---|
| `theme` (default) | the active Omarchy theme: its accent as the light, its background as the ground |
| `neon` | violet to cyan to pink light on near-black |
| `lavart` | the Twinise cyan |
| `deformation` | amber light over teal-green concrete, after the *Lorenz Deformation* renders |
| `ink` | dark ink on white paper, after the *Strange Attractors* poster series |

![Presets](docs/media/presets.jpg)

## Configuration

`~/.config/attractor/config.json` (the service restarts the wallpaper when it changes). Everything is optional:

```json
{
  "preset": "theme",
  "system": "cycle",
  "saver": "auto",
  "wallpaper": true,
  "wallpaper_options": { "fps": 30, "speed": 0.7, "overlay": false },
  "saver_options": { "preset": "neon", "fps": 60 }
}
```

| key | default | |
|---|---|---|
| `preset` | `theme` | see Looks |
| `system` | `cycle` | an attractor's name (`attractor --list`), or `cycle` to rotate through all |
| `cycle` | 150 / 90 | seconds on each attractor (wallpaper / screensaver) |
| `saver` | `auto` | `on`, `off`, or `auto`: on unless Gridfall is enabled, so two screensavers never race on idle |
| `wallpaper` | `true` | run the live wallpaper |
| `overlay` | `false` | wallpaper only: draw the trails with transparency over the desktop wallpaper instead of on their own ground |
| `fps`, `speed`, `trails`, `length`, `width`, `zoom`, `intensity`, `bloom`, `render_scale` | per mode | look and cost |
| `gpu` | `low` | `high` uses the discrete GPU; leave it on `low` on hybrid laptops |

`wallpaper_options` and `saver_options` override any of the above for that mode only. `ATTRACTOR_PRESET`,
`ATTRACTOR_SYSTEM` and `ATTRACTOR_GPU` override the config for a single run.

## How it works

Each attractor is integrated on the CPU in f64 with RK4 and fitted automatically to the unit ball (centre, radius and
step size are measured from a warm-up orbit, so there are no hand-tuned scales). Hundreds of trails keep a ring of their
latest points; each frame only the newly computed points are uploaded to a GPU storage buffer. The vertex shader reads
that ring and expands every segment into a screen-space ribbon, a two-level bloom adds the glow, and the composite
dithers by one 8-bit step so the gradient ground never bands.

Remote control: `omarchy-shell attractor preview` starts the screensaver now, `omarchy-shell attractor status` reports
the state, `omarchy-shell attractor restart` restarts the wallpaper.

MIT licensed.
