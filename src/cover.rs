//! The cover of what plays, as its player gives it to Windows: a small copy
//! to lay behind the lyrics, and the colours to draw them in (the cover's
//! most telling colour and a second that goes with it). Read once for each
//! track, on a thread of its own, only while something asks for it.

use std::sync::{Arc, Mutex};

use windows::core::Interface;
use windows::Win32::Graphics::Gdi::DeleteObject;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapSource, IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapIgnoreAlpha, WICBitmapInterpolationModeFant,
    WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CoCreateInstance, IStream, CLSCTX_INPROC_SERVER};
use windows::Win32::System::WinRT::CreateStreamOverRandomAccessStream;
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SHCreateMemStream, SIIGBF_BIGGERSIZEOK, SIIGBF_THUMBNAILONLY};

/// The side of the copy kept (pixels, square).
pub const SIDE: u32 = 192;

/// A track's cover: its copy (premultiplied BGRA, `SIDE` square, opaque), and its colours (straight RGB, 0 to 1).
pub struct Cover {
    pub pixels: Arc<Vec<u8>>,
    pub base: [f32; 3],
    pub accent: [f32; 3],
    /// Which cover it is, never another's: what is made of it (its picture drawn, the backdrop blurred from it) is
    /// kept by it. (By its address, a cover read later where one let go had been was taken for that one, and showed
    /// as it.)
    pub key: u64,
}

impl Cover {
    pub fn new(pixels: Vec<u8>, base: [f32; 3], accent: [f32; 3]) -> Cover {
        static MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Cover { pixels: Arc::new(pixels), base, accent, key: MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) }
    }
}

/// How long after its player gave no cover (or the cover of another track) it is asked again: a player just started,
/// or just changed tracks, gives its cover a moment after the rest.
const AGAIN_AFTER: std::time::Duration = std::time::Duration::from_secs(4);

/// What is known of a track's cover: being read, read (when: see `AGAIN_AFTER`) and none had, or it (and whether it
/// is to be read once more: see `of`).
enum Known {
    Reading,
    None(std::time::Instant),
    Found(Arc<Cover>, bool),
}

/// The track the cover was asked for (its title, artist and album), when it was first, and what is known of it.
static KNOWN: Mutex<Option<(String, std::time::Instant, Known)>> = Mutex::new(None);

/// The cover of the music playing, as its player gives it; read (on a thread of its own) the first time it is asked
/// for, and again a while after its player gave none. (By its title, artist and album: the same song on another
/// album has another cover.)
pub fn of(playing: &crate::playing::Now) -> Option<Arc<Cover>> {
    let key = key_of(playing);
    let mut known = KNOWN.lock().unwrap();
    // A cover read in the moment after its track came is read once more after it (shown meanwhile): a player that
    // tells the new title before the new cover gave the last track's with it, and that stood for the whole track.
    let first = match known.as_mut() {
        Some((of, first, Known::Found(cover, again))) if *of == key => {
            let cover = cover.clone();
            if *again && first.elapsed() >= AGAIN_AFTER {
                *again = false;
                read_on(key, track_of(playing), Some(cover.clone()));
            }
            return Some(cover);
        }
        Some((of, _, Known::Reading)) if *of == key => return None,
        Some((of, _, Known::None(since))) if *of == key && since.elapsed() < AGAIN_AFTER => return None,
        Some((of, first, _)) if *of == key => *first,
        _ => std::time::Instant::now(),
    };
    *known = Some((key.clone(), first, Known::Reading));
    read_on(key, track_of(playing), None);
    None
}

/// Whether the music playing is known to have no cover (read, and none found): not while it is being read, as
/// `of` gives none then too (what was shown of the track before is kept meanwhile, and goes only for none).
pub fn has_none(playing: &crate::playing::Now) -> bool {
    let key = key_of(playing);
    matches!(KNOWN.lock().unwrap().as_ref(), Some((of, _, Known::None(_))) if *of == key)
}

/// What a track's cover is known by: its title, artist and album.
fn key_of(playing: &crate::playing::Now) -> String {
    let crate::playing::Now { title, artist, album, .. } = playing;
    format!("{title}\u{1}{artist}\u{1}{album}")
}

/// What plays, as the music on this PC is matched against (see `library::file_of`): its first artist, as its lyrics
/// are asked for by.
fn track_of(playing: &crate::playing::Now) -> crate::lyrics::Track {
    let crate::playing::Now { title, artist, album, timeline, .. } = playing;
    let artist = artist.split(" / ").next().unwrap_or_default().to_string();
    let seconds = timeline.map_or(0, |timeline| timeline.length.round() as u32);
    crate::lyrics::Track { netease: None, title: title.clone(), artist, album: album.clone(), seconds, qq_music: false }
}

/// Reads the cover of `track` (known by `key`) on a thread of its own, and keeps it while that is still the track
/// asked about; `had`, the one read before, if this is it read once more (kept, where none is read now).
fn read_on(key: String, track: crate::lyrics::Track, had: Option<Arc<Cover>>) {
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let read = read(&track).map(Arc::new);
        let mut known = KNOWN.lock().unwrap();
        // (Kept only if the track is still the one asked about.)
        if let Some((of, first, state)) = known.as_mut() {
            if *of == key {
                let again = had.is_none() && first.elapsed() < AGAIN_AFTER;
                *state = match (read, had) {
                    (Some(cover), _) => Known::Found(cover, again),
                    (None, Some(had)) => Known::Found(had, false),
                    (None, None) => Known::None(std::time::Instant::now()),
                };
            }
        }
    });
}

/// The cover of the music playing, if it is `track` (a player changing tracks tells the new title a moment before the
/// new cover): as the player followed for music gives it; else, where it gives none (Windows' Media Player, of a file
/// whose cover is in its tags), the file's own, where it is among the music on this PC.
fn read(track: &crate::lyrics::Track) -> Option<Cover> {
    let session = crate::media::session_of(crate::media::Follow::Music).ok()?;
    let properties = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    if properties.Title().ok()?.to_string() != track.title {
        return None;
    }
    let factory: IWICImagingFactory = unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()? };
    let given = (|| unsafe {
        let stream = properties.Thumbnail().ok()?.OpenReadAsync().ok()?.join().ok()?;
        let stream: IStream = CreateStreamOverRandomAccessStream(&stream.cast::<windows::core::IUnknown>().ok()?).ok()?;
        let decoder = factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        decoder.GetFrame(0).ok()?.cast::<IWICBitmapSource>().ok()
    })();
    let picture = match given {
        Some(picture) => picture,
        None => {
            let path = crate::library::file_of(track)?;
            match crate::library::cover_of(&path).and_then(|bytes| decoded(&factory, &bytes)) {
                Some(picture) => picture,
                None => of_file(&factory, &path)?,
            }
        }
    };
    squared(&factory, &picture)
}

/// The picture in the file at `path` (a JPEG, a PNG) as a cover, as a player's would be.
#[cfg(feature = "studio")]
pub fn of_picture(path: &std::path::Path) -> Option<Cover> {
    let factory: IWICImagingFactory = unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()? };
    squared(&factory, &decoded(&factory, &std::fs::read(path).ok()?)?)
}

/// A picture as a file keeps it (a JPEG, a PNG), read.
fn decoded(factory: &IWICImagingFactory, bytes: &[u8]) -> Option<IWICBitmapSource> {
    unsafe {
        let stream = SHCreateMemStream(Some(bytes))?;
        let decoder = factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        decoder.GetFrame(0).ok()?.cast().ok()
    }
}

/// The picture Windows shows for the file at `path` where it has one of its own (a file of music: the cover in its
/// tags as Windows reads them, which is not all of them, or a picture of its album in its folder), never the icon of
/// its kind.
fn of_file(factory: &IWICImagingFactory, path: &std::path::Path) -> Option<IWICBitmapSource> {
    unsafe {
        let item: IShellItemImageFactory = SHCreateItemFromParsingName(&windows::core::HSTRING::from(path.as_os_str()), None).ok()?;
        let bitmap = item.GetImage(windows::Win32::Foundation::SIZE { cx: SIDE as i32, cy: SIDE as i32 }, SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK).ok()?;
        let picture = factory.CreateBitmapFromHBITMAP(bitmap, windows::Win32::Graphics::Gdi::HPALETTE::default(), WICBitmapIgnoreAlpha);
        let _ = DeleteObject(bitmap.into());
        picture.ok()?.cast().ok()
    }
}

/// `picture` as a cover: squared about its middle (most covers are square already), at `SIDE`, opaque, and its
/// colours.
fn squared(factory: &IWICImagingFactory, frame: &IWICBitmapSource) -> Option<Cover> {
    unsafe {
        let (mut width, mut height) = (0, 0);
        frame.GetSize(&mut width, &mut height).ok()?;
        let side = width.min(height);
        let clipper = factory.CreateBitmapClipper().ok()?;
        clipper.Initialize(frame, &windows::Win32::Graphics::Imaging::WICRect { X: ((width - side) / 2) as i32, Y: ((height - side) / 2) as i32, Width: side as i32, Height: side as i32 }).ok()?;
        let scaler = factory.CreateBitmapScaler().ok()?;
        scaler.Initialize(&clipper, SIDE, SIDE, WICBitmapInterpolationModeFant).ok()?;
        let converter = factory.CreateFormatConverter().ok()?;
        converter.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
        let mut pixels = vec![0u8; (SIDE * SIDE * 4) as usize];
        converter.CopyPixels(std::ptr::null(), SIDE * 4, &mut pixels).ok()?;
        // (Opaque: a cover with clear parts is laid on black.)
        for pixel in pixels.chunks_mut(4) {
            pixel[3] = 255;
        }
        let (base, accent) = colours(&pixels);
        Some(Cover::new(pixels, base, accent))
    }
}

/// The colours of a cover's pixels (BGRA): the most telling of its main colours (vivid, neither near black nor
/// near white), and a second of another hue that goes with it (else the first turned round the wheel).
pub fn colours(pixels: &[u8]) -> ([f32; 3], [f32; 3]) {
    let main = median_cut(pixels, 8);
    let telling = |rgb: [f32; 3]| {
        let (_, saturation, lightness) = hsl(rgb);
        saturation * (1.0 - (lightness - 0.5).abs() * 1.2)
    };
    let base = main.iter().map(|(rgb, _)| *rgb).max_by(|a, b| telling(*a).total_cmp(&telling(*b))).unwrap_or([0.5, 0.5, 0.5]);
    let (hue, saturation, lightness) = hsl(base);
    let apart = |rgb: [f32; 3]| {
        let away = (hsl(rgb).0 - hue).abs();
        away.min(360.0 - away)
    };
    let accent = main.iter().map(|(rgb, _)| *rgb).filter(|rgb| (25.0..=160.0).contains(&apart(*rgb)) && hsl(*rgb).1 > 0.15).max_by(|a, b| telling(*a).total_cmp(&telling(*b)));
    (base, accent.unwrap_or_else(|| rgb((hue + 40.0) % 360.0, saturation, lightness)))
}

/// A cover's main colours and how much of it each is: its pixels (at four bits a channel) split, box by box, along
/// the widest range of the box that holds the most range and weight, into `count` boxes, each by its weighted mean.
fn median_cut(pixels: &[u8], count: usize) -> Vec<([f32; 3], f32)> {
    let mut counted = vec![0u32; 4096];
    for pixel in pixels.chunks(4) {
        counted[(pixel[2] as usize >> 4) << 8 | (pixel[1] as usize >> 4) << 4 | pixel[0] as usize >> 4] += 1;
    }
    let colours: Vec<([u8; 3], u32)> = counted.iter().enumerate().filter(|(_, n)| **n > 0).map(|(i, n)| ([(i >> 8) as u8, (i >> 4 & 15) as u8, (i & 15) as u8], *n)).collect();
    let mut boxes = vec![colours];
    while boxes.len() < count {
        let range = |colours: &[([u8; 3], u32)], channel: usize| colours.iter().map(|c| c.0[channel]).max().unwrap_or(0) - colours.iter().map(|c| c.0[channel]).min().unwrap_or(0);
        let worth = |colours: &[([u8; 3], u32)]| (0..3).map(|c| range(colours, c)).max().unwrap_or(0) as f32 * (colours.iter().map(|c| c.1).sum::<u32>() as f32).sqrt();
        let Some((at, _)) = boxes.iter().enumerate().filter(|(_, b)| b.len() > 1).max_by(|a, b| worth(a.1).total_cmp(&worth(b.1))) else { break };
        let mut split = boxes.swap_remove(at);
        let channel = (0..3).max_by_key(|c| range(&split, *c)).unwrap_or(0);
        split.sort_by_key(|c| c.0[channel]);
        let total: u32 = split.iter().map(|c| c.1).sum();
        let mut so_far = 0;
        let middle = split.iter().position(|c| {
            so_far += c.1;
            so_far * 2 >= total
        });
        let middle = middle.unwrap_or(0).clamp(1, split.len() - 1);
        let other = split.split_off(middle);
        boxes.push(split);
        boxes.push(other);
    }
    let all: u32 = boxes.iter().flatten().map(|c| c.1).sum::<u32>().max(1);
    boxes
        .iter()
        .map(|colours| {
            let weight: u32 = colours.iter().map(|c| c.1).sum();
            let mean = |channel: usize| colours.iter().map(|c| (c.0[channel] as f32 * 16.0 + 8.0) * c.1 as f32).sum::<f32>() / weight.max(1) as f32 / 255.0;
            ([mean(0), mean(1), mean(2)], weight as f32 / all as f32)
        })
        .collect()
}

/// A colour's hue (degrees), saturation and lightness (0 to 1).
pub fn hsl([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let (most, least) = (r.max(g).max(b), r.min(g).min(b));
    let lightness = (most + least) / 2.0;
    if most - least < 1e-6 {
        return (0.0, 0.0, lightness);
    }
    let span = most - least;
    let saturation = span / (1.0 - (2.0 * lightness - 1.0).abs()).max(1e-6);
    let hue = if most == r {
        60.0 * ((g - b) / span).rem_euclid(6.0)
    } else if most == g {
        60.0 * ((b - r) / span + 2.0)
    } else {
        60.0 * ((r - g) / span + 4.0)
    };
    (hue, saturation.min(1.0), lightness)
}

/// The colour of `hue` (degrees), `saturation` and `lightness`.
pub fn rgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let x = chroma * (1.0 - ((hue / 60.0).rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match (hue.rem_euclid(360.0) / 60.0) as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = lightness - chroma / 2.0;
    [r + m, g + m, b + m]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_covers_colours() {
        // Most of it dark grey, a red part and a blue part: the vivid two, one the base and the other with it.
        let mut pixels = Vec::new();
        for i in 0..1000 {
            let bgr = if i < 600 { [40, 40, 40] } else if i < 850 { [30, 30, 220] } else { [220, 120, 30] };
            pixels.extend_from_slice(&[bgr[0], bgr[1], bgr[2], 255]);
        }
        let (base, accent) = colours(&pixels);
        let red = |rgb: [f32; 3]| rgb[0] > 0.7 && rgb[2] < 0.3;
        let blue = |rgb: [f32; 3]| rgb[2] > 0.7 && rgb[0] < 0.3;
        assert!((red(base) && blue(accent)) || (blue(base) && red(accent)), "{base:?} {accent:?}");
        let (h, s, l) = hsl([0.8, 0.2, 0.2]);
        let back = rgb(h, s, l);
        assert!((back[0] - 0.8).abs() < 1e-3 && (back[1] - 0.2).abs() < 1e-3);
    }

    /// A cover's main colours are each a colour: a white's channels no more than wholly lit (each the middle of the
    /// pixels' bucket, not a little past it).
    #[test]
    fn a_covers_main_colours_are_colours() {
        let pixels: Vec<u8> = (0..1000).flat_map(|i| if i < 500 { [255, 255, 255, 255] } else { [25, 8, 8, 255] }).collect();
        assert!(median_cut(&pixels, 8).iter().all(|(rgb, _)| rgb.iter().all(|v| (0.0..=1.0).contains(v))));
    }

    /// Reads the cover of the music playing: `cargo test -- --ignored reads_the_cover --nocapture`.
    #[test]
    #[ignore = "reads the cover of the music this PC is playing"]
    fn reads_the_cover() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let session = crate::media::session_of(crate::media::Follow::Music).expect("music playing");
        let properties = session.TryGetMediaPropertiesAsync().unwrap().join().unwrap();
        let (title, artist, album) = (properties.Title().unwrap().to_string(), properties.Artist().unwrap().to_string(), properties.AlbumTitle().unwrap().to_string());
        let seconds = (session.GetTimelineProperties().unwrap().EndTime().unwrap().Duration as f64 / 1e7).round() as u32;
        let track = crate::lyrics::Track { netease: None, title: title.clone(), artist, album, seconds, qq_music: false };
        let cover = read(&track).expect("a cover");
        println!("{title}");
        println!("base {:?}, accent {:?}, {} bytes", cover.base, cover.accent, cover.pixels.len());
    }

    /// Reads the cover in a file's tags (`GLANCE_COVER_FILE` names it): `cargo test -- --ignored reads_a_files_cover
    /// --nocapture`.
    #[test]
    #[ignore = "reads a file named by GLANCE_COVER_FILE"]
    fn reads_a_files_cover() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let path = std::path::PathBuf::from(std::env::var("GLANCE_COVER_FILE").expect("GLANCE_COVER_FILE"));
        let factory: IWICImagingFactory = unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap() };
        let cover = squared(&factory, &of_file(&factory, &path).expect("its picture")).expect("a cover");
        println!("base {:?}, accent {:?}", cover.base, cover.accent);
    }
}
