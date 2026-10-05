//! Measures what a native panel costs: a transparent, topmost window drawn
//! with Direct2D and DirectWrite onto a DirectComposition surface, redrawing
//! a rounded panel, text and scrolling charts every frame. Runs for a fixed
//! time and exits.

use std::time::{Duration, Instant};

use windows::core::{w, Interface, Result};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_RECT_F};
use windows_numerics::Vector2 as D2D_POINT_2F;
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice2, IDCompositionDesktopDevice, IDCompositionSurface, IDCompositionTarget,
    IDCompositionVisual2,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::*;

const WIDTH: i32 = 712;
const HEIGHT: i32 = 1000;
const RUN_FOR: Duration = Duration::from_secs(20);

/// Premultiplied, as the surface stores colour.
fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: r * a, g: g * a, b: b * a, a }
}

fn rect(left: f32, top: f32, width: f32, height: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right: left + width, bottom: top + height }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn main() -> Result<()> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("GlanceProbe"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("GlanceProbe"),
            w!("Glance native probe"),
            WS_POPUP | WS_VISIBLE,
            1920 - WIDTH - 12,
            40,
            WIDTH,
            HEIGHT,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        let mut d3d: Option<ID3D11Device> = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            Default::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d),
            None,
            None,
        )?;
        let dxgi: IDXGIDevice = d3d.unwrap().cast()?;
        let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let d2d: ID2D1Device = factory.CreateDevice(&dxgi)?;

        // Made from the Direct2D device, the composition surface hands out a
        // device context already aimed at itself.
        let dcomp: IDCompositionDesktopDevice = DCompositionCreateDevice2(&d2d)?;
        let target: IDCompositionTarget = dcomp.CreateTargetForHwnd(hwnd, true)?;
        let visual: IDCompositionVisual2 = dcomp.CreateVisual()?;
        let surface: IDCompositionSurface =
            dcomp.CreateSurface(WIDTH as u32, HEIGHT as u32, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED)?;
        visual.SetContent(&surface)?;
        target.SetRoot(&visual)?;
        dcomp.Commit()?;

        let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let font = |size: f32, weight: DWRITE_FONT_WEIGHT| -> Result<IDWriteTextFormat> {
            write.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("zh-cn"),
            )
        };
        let small = font(13.0, DWRITE_FONT_WEIGHT_NORMAL)?;
        let big = font(40.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;

        let started = Instant::now();
        let mut frames = 0u32;
        let mut msg = MSG::default();
        while started.elapsed() < RUN_FOR {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&msg);
            }
            let mut offset = POINT::default();
            let dc: ID2D1DeviceContext = surface.BeginDraw(None, &mut offset)?;
            let ink = dc.CreateSolidColorBrush(&color(0.08, 0.08, 0.1, 1.0), None)?;
            let faint = dc.CreateSolidColorBrush(&color(0.08, 0.08, 0.1, 0.55), None)?;
            let paper = dc.CreateSolidColorBrush(&color(0.96, 0.96, 0.97, 0.94), None)?;
            let accent = dc.CreateSolidColorBrush(&color(0.0, 0.37, 0.72, 1.0), None)?;
            let text = |s: &str, format: &IDWriteTextFormat, area: D2D_RECT_F, brush| {
                let wide: Vec<u16> = s.encode_utf16().collect();
                dc.DrawText(&wide, format, &area, brush, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            };

            dc.Clear(Some(&color(0.0, 0.0, 0.0, 0.0)));
            let (ox, oy) = (offset.x as f32, offset.y as f32);
            let panel = D2D1_ROUNDED_RECT { rect: rect(ox, oy, WIDTH as f32, HEIGHT as f32), radiusX: 16.0, radiusY: 16.0 };
            dc.FillRoundedRectangle(&panel, &paper);
            let t = started.elapsed().as_secs_f32();
            for lane in 0..6 {
                let top = oy + 24.0 + lane as f32 * 160.0;
                text("CPU  AMD Ryzen 9 9950X 16-Core 处理器", &small, rect(ox + 24.0, top, 600.0, 20.0), &faint);
                let value = 40.0 + 30.0 * (t * 1.3 + lane as f32).sin();
                text(&format!("{value:.0}%"), &big, rect(ox + 24.0, top + 24.0, 200.0, 56.0), &ink);
                let mut last = None;
                for i in 0..120 {
                    let x = ox + 230.0 + i as f32 * 3.6;
                    let wave = 0.5 + 0.5 * (t * 2.0 + i as f32 * 0.15 + lane as f32).sin();
                    let point = D2D_POINT_2F { X: x, Y: top + 70.0 - 40.0 * wave };
                    if let Some(previous) = last {
                        dc.DrawLine(previous, point, &accent, 1.5, None);
                    }
                    last = Some(point);
                }
            }
            surface.EndDraw()?;
            dcomp.Commit()?;
            dcomp.WaitForCommitCompletion()?;
            frames += 1;
        }
        println!("{frames} frames in {:?}", started.elapsed());
        let _ = DestroyWindow(hwnd);
    }
    Ok(())
}
