//! A tiny Direct3D 11 program that presents frames at a fixed rate, used to
//! check the ETW frame counter end to end (CI runs it on a GPU-less Windows
//! machine with Microsoft's WARP software renderer). It writes its own count
//! of presents so the measured FPS can be compared with the truth.

use super::wide;
use anyhow::{Context, Result};
use serde::Serialize;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D,
    D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_DESC, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    IDXGISwapChain, DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::QueryPerformanceCounter;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, PeekMessageW, RegisterClassW, ShowWindow, TranslateMessage,
    MSG, PM_REMOVE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

#[derive(Serialize)]
pub struct ProbeResult {
    pub target_fps: u32,
    pub presents: u64,
    pub seconds: f64,
    pub fps: f64,
    pub renderer: String,
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, m, w, l) }
}

fn qpc() -> i64 {
    let mut v = 0i64;
    unsafe {
        let _ = QueryPerformanceCounter(&mut v);
    }
    v
}

pub fn run(seconds: u32, target_fps: u32) -> Result<ProbeResult> {
    unsafe {
        let hinst = GetModuleHandleW(PCWSTR::null())?;
        let class = wide("AienRigPresentProbe");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let title = wide("AIEN Rig present probe");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            50,
            50,
            336,
            279,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .context("creating the probe window")?;
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        let desc = DXGI_SWAP_CHAIN_DESC {
            BufferDesc: DXGI_MODE_DESC { Width: 320, Height: 240, Format: DXGI_FORMAT_B8G8R8A8_UNORM, ..Default::default() },
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            OutputWindow: hwnd,
            Windowed: true.into(),
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            ..Default::default()
        };
        let mut swap: Option<IDXGISwapChain> = None;
        let mut device: Option<ID3D11Device> = None;
        let mut ctx: Option<ID3D11DeviceContext> = None;
        D3D11CreateDeviceAndSwapChain(
            None,
            D3D_DRIVER_TYPE_WARP,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            None,
            D3D11_SDK_VERSION,
            Some(&desc),
            Some(&mut swap),
            Some(&mut device),
            None,
            Some(&mut ctx),
        )
        .context("creating a WARP Direct3D 11 device")?;
        let swap = swap.context("no swap chain")?;
        let device = device.context("no device")?;
        let ctx = ctx.context("no context")?;
        let back: ID3D11Texture2D = swap.GetBuffer(0)?;
        let mut rtv: Option<ID3D11RenderTargetView> = None;
        device.CreateRenderTargetView(&back, None, Some(&mut rtv))?;
        let rtv = rtv.context("no render target")?;

        let freq = super::etw::qpc_frequency();
        let period = freq / target_fps.max(1) as i64;
        let start = qpc();
        let end = start + freq * seconds as i64;
        let mut next = start;
        let mut presents = 0u64;
        let mut msg = MSG::default();
        while qpc() < end {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let shade = (presents % 256) as f32 / 255.0;
            ctx.ClearRenderTargetView(&rtv, &[shade, 0.2, 0.4, 1.0]);
            let _ = swap.Present(0, DXGI_PRESENT(0));
            presents += 1;
            next += period;
                loop {
                let now = qpc();
                if now >= next {
                    break;
                }
                // Spin with yields: Sleep() can overshoot by a whole 15.6 ms
                // timer tick, which would make the probe's own pace uneven.
                std::thread::yield_now();
            }
        }
        let secs = (qpc() - start) as f64 / freq as f64;
        Ok(ProbeResult {
            target_fps,
            presents,
            seconds: secs,
            fps: presents as f64 / secs,
            renderer: "Direct3D 11 WARP, flip-discard swap chain".into(),
        })
    }
}
