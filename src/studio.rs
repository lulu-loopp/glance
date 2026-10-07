//! The studio: Glance's panel filmed off screen, frame by frame, from a
//! script of shots, for the promotional video and the README's pictures.
//! It draws with the panel's own code, over a chosen picture, with readings
//! made up to a script (a load that rises and falls) on this machine's own
//! hardware names. Built only with the `studio` feature:
//!
//! ```text
//! cargo run --release --features studio -- --studio studio\shots.json target\studio
//! ```
//!
//! Each shot becomes a folder of numbered PNG frames.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use windows::core::HSTRING;
use windows::Win32::Foundation::{GENERIC_WRITE, RECT};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{D2D1_INTERPOLATION_MODE_LINEAR, D2D1_LAYER_PARAMETERS1};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory,
    WICBitmapEncoderNoCache,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows_numerics::{Matrix3x2, Vector2};

use crate::panel::{CLOSE, OPEN, OPEN_FADE, PEN_LAG_MS};
use crate::reading::{
    BatterySample, BoardSensors, CpuSensors, DriveTemperature, GameSample, GpuLimit, GpuSample, MemorySample, NetworkInfo, ProcessSample,
    Sample, StaticInfo, SystemSample, VolumeSample,
};
use crate::settings::Edge;
use crate::ui::gfx::Gfx;
use crate::ui::prefs::Prefs;
use crate::ui::arrange::{self, GAP};
use crate::ui::render::{self, PanelLayers};
use crate::ui::seen::Seen;
use crate::ui::skins;
use crate::ui::text::Lang;
use crate::ui::theme::{self, Entrance, Skin, Theme};
use crate::ui::view::{self, Scene};
use crate::ui::wallpaper;

#[derive(Deserialize)]
struct Script {
    /// The frames' size in pixels, and how many pixels a panel DIP takes.
    width: u32,
    height: u32,
    scale: f32,
    fps: f32,
    /// The picture the panel is filmed over, as the desktop.
    wallpaper: String,
    shots: Vec<Shot>,
}

#[derive(Deserialize)]
struct Shot {
    name: String,
    /// "paper", "glass" or "fluent".
    skin: String,
    /// "light", "dark", or "backdrop" (as the picture behind decides).
    #[serde(default = "backdrop")]
    theme: String,
    /// "zh" or "en".
    #[serde(default = "zh")]
    lang: String,
    seconds: f32,
    /// When the panel slides in; shown from the start if absent.
    slide_in: Option<f32>,
    /// When it slides away again, if it does.
    slide_out: Option<f32>,
    /// How many times slower than life the sliding is filmed (1 if absent).
    #[serde(default)]
    slow: f32,
    /// The load, 0 (idle) to 1 (flat out), from the start to the end.
    #[serde(default)]
    load: (f32, f32),
    /// The lanes shown, in order; all of this machine's but "system" if absent.
    modules: Option<Vec<String>>,
    /// A game being played, by its name, if one is.
    game: Option<String>,
}

fn backdrop() -> String {
    "backdrop".into()
}

fn zh() -> String {
    "zh".into()
}

/// Films every shot in `script` into a folder of its own under `out`.
pub fn run(script: &Path, out: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(script).map_err(|e| format!("{}: {e}", script.display()))?;
    let script: Script = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", script.display()))?;
    // This machine's own names, read as Glance reads them.
    let info = crate::metrics::Sampler::new().info;
    let gfx = Gfx::new().map_err(|e| e.to_string())?;
    let desktop = wallpaper::picture(&script.wallpaper, script.width, script.height).map_err(|e| format!("{}: {e}", script.wallpaper))?;
    for shot in &script.shots {
        let folder = out.join(&shot.name);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        film(&gfx, &script, shot, &info, &desktop, &folder)?;
        println!("{}: {} frames", shot.name, (shot.seconds * script.fps).round());
    }
    Ok(())
}

fn film(gfx: &Gfx, script: &Script, shot: &Shot, info: &StaticInfo, desktop: &crate::ui::backdrop::Capture, folder: &Path) -> Result<(), String> {
    let skin = Skin::named(&shot.skin);
    let lang = if shot.lang == "en" { Lang::En } else { Lang::Zh };
    let px = script.scale;
    let (sw, sh) = (script.width as f32 / px, script.height as f32 / px);
    let mut known = vec!["game".to_string(), "cpu".to_string()];
    known.extend(info.gpu_modules());
    known.extend(["memory", "network", "disk", "processes", "storage", "board", "battery", "system"].map(String::from));
    let mut prefs = Prefs::resolve(&serde_json::Value::Null, &known);
    if let Some(shown) = &shot.modules {
        prefs.modules.sort_by_key(|entry| shown.iter().position(|id| *id == entry.id).unwrap_or(usize::MAX));
        prefs.modules.iter_mut().for_each(|entry| entry.on = shown.contains(&entry.id));
    }

    let frames = (shot.seconds * script.fps).round() as usize;
    let interval = 1000.0;
    let start_ms = 1_760_000_000_000.0;
    let backlog = prefs.chart_seconds + 5.0;
    let load = |t: f32| shot.load.0 + (shot.load.1 - shot.load.0) * (t / shot.seconds).clamp(0.0, 1.0);
    // A sample a second, from well before the shot to its end.
    let history: Vec<Sample> = (0..=(backlog + shot.seconds as f64) as usize)
        .map(|i| {
            let t = (i as f64 - backlog) as f32;
            made_up(info, start_ms + i as f64 * interval - backlog * 1000.0, i as u64, load(t.max(0.0)), shot.game.as_deref())
        })
        .collect();

    // Laid out and placed as the panel would be on a screen this size, for
    // the readings of the moment, and held through the shot as the panel
    // holds its outline while it is up (see `arrange::Opening`).
    let edge = Edge::Right;
    let opening: std::cell::RefCell<Option<arrange::Opening>> = std::cell::RefCell::new(None);
    let place = |scene: &Scene, lanes: &[view::Lane], theme: &Theme| {
        let mut held = opening.borrow_mut();
        let opening = match held.as_mut() {
            Some(opening) => {
                opening.grow(scene.seen, info, |id, seen| view::lane_height(scene, id, seen));
                opening
            }
            None => {
                let heights: Vec<(&str, f32)> = lanes.iter().map(|lane| (lane.id.as_str(), lane.height(theme))).collect();
                held.insert(arrange::Opening::new(theme, edge, &heights, (sw, sh), None, scene.seen.clone()))
            }
        };
        let (layout, zoom) = (opening.layout.clone(), opening.zoom);
        let (pw, ph) = (layout.width() * zoom, layout.height() * zoom);
        let rest = (sw - theme.inset * zoom - pw, ((sh - ph) / 2.0).max(GAP));
        (layout, zoom, rest, opening.seen.clone())
    };
    // Toned by the desktop where it first rests.
    let measure = Theme::new(skin, false);
    let first = &history[..=backlog as usize];
    let first_seen = Seen::of(first);
    let probe = Scene { info, prefs: &prefs, theme: &measure, lang, history: first, seen: &first_seen, pen_ms: 0.0, process_scroll: 0.0, hover: None, pinned: false };
    let (layout, zoom, rest, _) = place(&probe, &view::lanes(&probe), &measure);
    let (pw, ph) = (layout.width() * zoom, layout.height() * zoom);
    let behind = RECT {
        left: (rest.0 * px) as i32,
        top: (rest.1 * px) as i32,
        right: ((rest.0 + pw) * px) as i32,
        bottom: ((rest.1 + ph) * px) as i32,
    };
    let tone = desktop.luminance(behind, px);
    let dark = match shot.theme.as_str() {
        "light" => false,
        "dark" => true,
        _ => theme::is_dark(crate::ui::prefs::ThemePref::Backdrop, Some(tone.0).filter(|_| skin.sees_backdrop())),
    };
    let theme = Theme::new(skin, dark);
    let frost = if skin == Skin::Glass { skins::frost(tone.0, tone.1, dark) } else { 0.0 };
    let ids: Vec<&str> = prefs.modules.iter().filter(|entry| entry.on).map(|entry| entry.id.as_str()).collect();
    let mut placed = serde_json::Value::Null;

    let mut layers = PanelLayers::default();
    for index in 0..frames {
        let t = index as f32 / script.fps;
        let now_ms = start_ms + t as f64 * 1000.0;
        let read = history.iter().take_while(|sample| sample.t as f64 <= now_ms).count().max(1);
        let seen = Seen::of(&history[..read]);
        let scene = Scene {
            info,
            prefs: &prefs,
            theme: &theme,
            lang,
            history: &history[..read],
            seen: &seen,
            pen_ms: now_ms - interval - PEN_LAG_MS,
            process_scroll: 0.0,
            hover: None,
            pinned: false,
        };
        let (layout, zoom, rest, held) = place(&scene, &view::lanes(&scene), &theme);
        // What the panel holds as it has been up: drawn in the boxes it opened with.
        let scene = Scene { seen: &held, ..scene };
        let lanes = view::lanes(&scene);
        let travel = match theme.entrance {
            Entrance::Beyond(extra) => layout.width() + extra,
            Entrance::Slide(distance) => distance,
        };
        // Where the panel, the pieces it is drawn as (each lane and the bar
        // on glass, else one slab) and each lane are in the frame, in
        // pixels, for the compositor: where the shot leaves them.
        let rect = |r: &view::Rect| {
            serde_json::json!({ "x": (rest.0 + r.x * zoom) * px, "y": (rest.1 + r.y * zoom) * px, "w": r.w * zoom * px, "h": r.h * zoom * px })
        };
        let panel = view::Rect { x: 0.0, y: 0.0, w: layout.width(), h: layout.height() };
        let boxes = layout.lanes();
        let pieces: Vec<_> = if skin == Skin::Glass { boxes.iter().chain([&layout.bar()]).map(rect).collect() } else { vec![rect(&panel)] };
        let named: serde_json::Map<String, serde_json::Value> = ids.iter().zip(&boxes).map(|(id, r)| (id.to_string(), rect(r))).collect();
        placed = serde_json::json!({ "panel": rect(&panel), "radius": theme.radius * zoom * px, "pieces": pieces, "lanes": named });
        // Where the panel is at `at` seconds: off by `shift` of its travel,
        // at `opacity`.
        let moved = |at: f32| motion(shot, at);
        let draw = |shift: f32, opacity: f32, layers: &mut PanelLayers| {
            gfx.draw_offscreen((script.width, script.height), px, |frame| {
                let bitmap = desktop.bitmap(&frame.dc, px).ok();
                if let Some(bitmap) = &bitmap {
                    unsafe { frame.dc.DrawBitmap(bitmap, None, 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None) };
                }
                if opacity <= 0.0 {
                    return;
                }
                let everything = D2D_RECT_F { left: -f32::MAX, top: -f32::MAX, right: f32::MAX, bottom: f32::MAX };
                let layer = D2D1_LAYER_PARAMETERS1 { contentBounds: everything, opacity, ..Default::default() };
                unsafe { frame.dc.PushLayer(&layer, None) };
                // The desktop as the panel's own DIPs measure it, zoomed with
                // the panel, as the panel takes it (its pixels per DIP
                // include the zoom).
                let behind = desktop.bitmap(&frame.dc, px * zoom).ok();
                let backdrop = behind.as_ref().map(|bitmap| (bitmap, Vector2 { X: rest.0 / zoom, Y: rest.1 / zoom }, desktop.digest));
                let picture = render::Picture { scene: &scene, lanes: &lanes, layout: &layout, edge, backdrop, frost };
                let local = Matrix3x2::scale(zoom, zoom) * Matrix3x2::translation(rest.0 + shift * travel * zoom, rest.1);
                let _ = layers.draw(frame, &picture, local, px * zoom);
                unsafe { frame.dc.PopLayer() };
            })
            .map_err(|e| e.to_string())
        };
        // While the panel moves, the frame is the mean of several moments
        // across half the frame's time: a camera's motion blur.
        let pixels = if moved(t) == moved(t + 1.0 / script.fps) {
            let (shift, opacity) = moved(t);
            draw(shift, opacity, &mut layers)?
        } else {
            const MOMENTS: usize = 8;
            let mut sum = vec![0u32; (script.width * script.height * 4) as usize];
            for k in 0..MOMENTS {
                let (shift, opacity) = moved(t + k as f32 / MOMENTS as f32 * 0.5 / script.fps);
                for (total, value) in sum.iter_mut().zip(draw(shift, opacity, &mut layers)?) {
                    *total += value as u32;
                }
            }
            sum.into_iter().map(|total| ((total + MOMENTS as u32 / 2) / MOMENTS as u32) as u8).collect()
        };
        gfx.sweep();
        save_png(&folder.join(format!("{:05}.png", index + 1)), script.width, script.height, &pixels).map_err(|e| e.to_string())?;
    }
    std::fs::write(folder.join("lanes.json"), serde_json::to_string_pretty(&placed).unwrap()).map_err(|e| e.to_string())
}

/// Where a shot's panel is at `t` seconds: how far out of its travel
/// (1 out, 0 in place) and how opaque, sliding in and away as the panel
/// does, with the same curves and fades, slowed `slow` times if asked.
fn motion(shot: &Shot, t: f32) -> (f32, f32) {
    let slow = shot.slow.max(1.0);
    let (mut shift, mut opacity) = match shot.slide_in {
        Some(at) if t < at => (1.0, 0.0),
        Some(at) => {
            let into = (t - at) / slow;
            (1.0 - OPEN.1.at((into / OPEN.0.as_secs_f32()).min(1.0)), (into / OPEN_FADE.as_secs_f32()).min(1.0))
        }
        None => (0.0, 1.0),
    };
    if let Some(at) = shot.slide_out.filter(|at| t >= *at) {
        let out = ((t - at) / slow / CLOSE.0.as_secs_f32()).min(1.0);
        shift = shift.max(CLOSE.1.at(out));
        opacity = opacity.min(1.0 - out);
    }
    (shift, opacity)
}

/// Readings for a machine with `info`'s hardware, at `load` (0 idle, 1 flat
/// out), playing `game` if one is named, varying from second to second as
/// real ones do: the same for the same `seed`, so that a shot films the
/// same every time.
fn made_up(info: &StaticInfo, t_ms: f64, seed: u64, load: f32, game: Option<&str>) -> Sample {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 40) as f32 / (1u64 << 24) as f32
    };
    let wave = |period: f32, phase: f32| ((seed as f32 / period + phase) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    let busy = load.clamp(0.0, 1.0);
    let cpu = (4.0 + 62.0 * busy + 8.0 * wave(9.0, 0.1) * busy + 3.0 * noise()).clamp(0.0, 100.0);
    let threads = (0..info.threads)
        .map(|i| Some((cpu * (0.4 + 1.2 * ((i as f32 * 0.61 + seed as f32 * 0.07).sin() * 0.5 + 0.5)) + 4.0 * noise()).clamp(0.0, 100.0)))
        .collect();
    let gb = 1u64 << 30;
    let gpus = info
        .gpus
        .iter()
        .enumerate()
        .map(|(i, gpu)| {
            // The first card does the work; the others rest.
            let g = if i == 0 { busy } else { 0.02 };
            let usage = (2.0 + 92.0 * g + 4.0 * wave(5.0, 0.3) * g + 2.0 * noise()).clamp(0.0, 100.0);
            GpuSample {
                usage: Some(usage),
                engines: Some(vec![("3D".into(), usage), ("Copy".into(), 3.0 * g), ("VideoDecode".into(), 0.0)]),
                mem_used: Some(((0.12 + 0.55 * g) * gpu.mem_total as f32) as u64),
                shared_used: Some((0.2 * gb as f32) as u64),
                temp: Some(36.0 + 34.0 * g + 1.5 * noise()),
                clock_mhz: Some(if g > 0.05 { 2400.0 + 300.0 * g + 30.0 * noise() } else { 210.0 + 40.0 * noise() }),
                // A card whose fans turn even at rest, so that its lane
                // keeps its rows through a shot.
                fan_rpm: Some((780.0 + 1100.0 * g) as u32),
                power: (i == 0).then_some(18.0 + 255.0 * g + 12.0 * wave(4.0, 0.7) * g + 2.0 * noise()),
            }
        })
        .collect();
    let used = ((0.22 + 0.25 * busy + 0.02 * noise()) * info.mem_total as f32) as u64;
    let package = 38.0 + 36.0 * busy + 2.0 * wave(7.0, 0.2) + noise();
    let names = ["blender", "chrome", "code", "obs64", "glance", "explorer", "dwm", "svchost"];
    let processes = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let share = 1.0 / (i as f32 + 1.5);
            ProcessSample {
                name: name.to_string(),
                cpu: cpu * share * (0.8 + 0.4 * noise()),
                mem: ((0.5 + 3.0 * share) * gb as f32) as u64,
                io: (2.0e6 * share * (0.5 + noise())) as f64,
                gpu: Some(if i == 0 { 60.0 * busy } else { 2.0 * share }),
            }
        })
        .collect();
    // Traffic comes in bursts: now and then several times the usual.
    let mut burst = |base: f64| {
        let (level, spike) = (noise() as f64, noise());
        base * (0.3 + 1.4 * level) * if spike > 0.8 { 6.0 } else { 1.0 }
    };
    let (net_down, net_up) = (burst(180_000.0 + 2_500_000.0 * busy as f64), burst(40_000.0));
    let (disk_read, disk_write) = (burst(1_200_000.0 + 40_000_000.0 * busy as f64), burst(400_000.0));
    let (ghz, cpu_power) = (4.3 + 1.0 * busy + 0.1 * noise(), 22.0 + 150.0 * busy + 6.0 * noise());
    // Held near a 165 Hz screen's rate, with a stutter now and then.
    let game = game.map(|name| {
        let stutter = noise() > 0.9;
        GameSample {
            name: name.to_string(),
            program: name.to_string(),
            is_game: true,
            fps: 138.0 + 14.0 * wave(11.0, 0.4) + 4.0 * noise() - if stutter { 9.0 } else { 0.0 },
            low: Some(96.0 + 6.0 * noise()),
            longest_ms: if stutter { 18.0 + 14.0 * noise() } else { 7.6 + 1.6 * noise() },
            fills_screen: true,
            refresh_hz: Some(165),
            cpu: Some(14.0 + 6.0 * noise()),
            gpu: Some(93.0 + 5.0 * noise()),
            mem: Some((6.2 * gb as f32) as u64),
            vram: Some((7.9 * gb as f32) as u64),
            gpu_limit: Some(GpuLimit::Power),
            playing_s: Some(47 * 60 + seed),
        }
    });
    Sample {
        t: t_ms as u64,
        cpu: Some(cpu),
        threads,
        ghz: Some(ghz),
        memory: MemorySample { used, committed: used + 6 * gb, commit_limit: info.mem_total + 8 * gb, cached: 12 * gb },
        gpus,
        net_down: Some(net_down),
        net_up: Some(net_up),
        net_total_down: 38 * gb + seed * 900_000,
        net_total_up: 4 * gb + seed * 120_000,
        network: Some(NetworkInfo {
            name: "WLAN".into(),
            model: info.network_adapter.clone().unwrap_or_default(),
            ipv4: Some("192.168.1.23".into()),
            link_bps: 2_401_000_000,
        }),
        disk_read: Some(disk_read),
        disk_write: Some(disk_write),
        disk_active: Some(2.0 + 30.0 * busy),
        volumes: vec![
            VolumeSample { name: "C:".into(), used: 205 * gb, total: 600 * gb },
            VolumeSample { name: "D:".into(), used: 268 * gb, total: 1262 * gb },
        ],
        processes,
        system: SystemSample { uptime_s: 3 * 3600 + 25 * 60 + seed, processes: 312, threads: 4810, handles: 168_000 },
        battery: None::<BatterySample>,
        cpu_sensors: Some(CpuSensors { temp: Some(package), ccds: vec![(0, package + 2.0), (1, package - 14.0)], power: Some(cpu_power) }),
        board: Some(BoardSensors {
            temps: vec![
                ("system".into(), 33.0 + 3.0 * busy),
                ("chipset".into(), 45.0 + 4.0 * busy),
                ("cpu_socket".into(), 52.0 + 18.0 * busy),
                ("pcie_x16".into(), 36.0 + 8.0 * busy),
                ("vrm".into(), 40.0 + 14.0 * busy),
                ("vsoc".into(), 39.0 + 6.0 * busy),
            ],
            fans: vec![("cpu_fan".into(), 950.0 + 900.0 * busy), ("system_fan_2".into(), 880.0 + 500.0 * busy)],
        }),
        drive_temps: info.drives.first().map(|name| DriveTemperature { id: 0, name: name.clone(), celsius: 34.0 + 9.0 * busy }).into_iter().collect(),
        dimm_temps: vec![35.0 + 7.0 * busy, 34.0 + 7.0 * busy],
        game,
    }
}

/// Writes premultiplied BGRA `pixels` (opaque: drawn over the picture) as a PNG.
fn save_png(path: &PathBuf, width: u32, height: u32, pixels: &[u8]) -> windows::core::Result<()> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE.0)?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        let mut options = None;
        encoder.CreateNewFrame(&mut frame, &mut options)?;
        let frame = frame.unwrap();
        frame.Initialize(options.as_ref())?;
        frame.SetSize(width, height)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WritePixels(height, width * 4, pixels)?;
        frame.Commit()?;
        encoder.Commit()?;
    }
    Ok(())
}
