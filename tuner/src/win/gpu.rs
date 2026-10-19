//! Graphics card facts: DXGI for every adapter, NVML (nvml.dll, shipped with
//! the NVIDIA driver; this is not CUDA) for Resizable BAR and the PCIe link.

use crate::findings::{GpuFacts, NvmlFacts};
use std::ffi::{c_char, c_void, CStr};
use windows::core::{s, w, Interface, PCWSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIDevice, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

pub fn adapters() -> Vec<GpuFacts> {
    let mut out = Vec::new();
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
        let mut i = 0;
        while let Ok(a) = factory.EnumAdapters1(i) {
            i += 1;
            let Ok(d) = a.GetDesc1() else { continue };
            if d.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }
            let drv = a.CheckInterfaceSupport(&IDXGIDevice::IID).ok().map(|v| {
                let v = v as u64;
                format!("{}.{}.{}.{}", v >> 48, (v >> 32) & 0xffff, (v >> 16) & 0xffff, v & 0xffff)
            });
            out.push(GpuFacts {
                name: super::from_wide(&d.Description),
                vendor_id: d.VendorId,
                dedicated_vram_mb: (d.DedicatedVideoMemory / (1024 * 1024)) as u64,
                dxgi_driver_version: drv,
                nvml: None,
            });
        }
    }
    out
}

#[repr(C)]
#[derive(Default)]
struct Bar1Memory {
    total: u64,
    free: u64,
    used: u64,
}

#[repr(C)]
#[derive(Default)]
struct Utilization {
    gpu: u32,
    memory: u32,
}

type Device = *mut c_void;
type FnVoid = unsafe extern "C" fn() -> i32;
type FnStr = unsafe extern "C" fn(*mut c_char, u32) -> i32;
type FnCount = unsafe extern "C" fn(*mut u32) -> i32;
type FnHandle = unsafe extern "C" fn(u32, *mut Device) -> i32;
type FnDevU32 = unsafe extern "C" fn(Device, *mut u32) -> i32;
type FnBar1 = unsafe extern "C" fn(Device, *mut Bar1Memory) -> i32;
type FnTemp = unsafe extern "C" fn(Device, u32, *mut u32) -> i32;
type FnUtil = unsafe extern "C" fn(Device, *mut Utilization) -> i32;

fn load_nvml() -> Option<HMODULE> {
    unsafe {
        LoadLibraryW(w!("nvml.dll")).ok().or_else(|| {
            LoadLibraryW(w!(r"C:\Program Files\NVIDIA Corporation\NVSMI\nvml.dll")).ok()
        })
    }
}

macro_rules! sym {
    ($lib:expr, $name:expr, $ty:ty) => {
        GetProcAddress($lib, $name).map(|f| std::mem::transmute::<_, $ty>(f))
    };
}

/// Read NVML facts for the first NVIDIA GPU. None if nvml.dll is not there.
pub fn nvml_facts() -> Option<NvmlFacts> {
    let lib = load_nvml()?;
    unsafe {
        let init: FnVoid = sym!(lib, s!("nvmlInit_v2"), FnVoid)?;
        let shutdown: Option<FnVoid> = sym!(lib, s!("nvmlShutdown"), FnVoid);
        if init() != 0 {
            return None;
        }
        let mut f = NvmlFacts::default();
        if let Some(drv) = sym!(lib, s!("nvmlSystemGetDriverVersion"), FnStr) {
            let mut buf = [0 as c_char; 96];
            if drv(buf.as_mut_ptr(), buf.len() as u32) == 0 {
                f.driver_version = Some(CStr::from_ptr(buf.as_ptr()).to_string_lossy().to_string());
            }
        }
        let count: Option<FnCount> = sym!(lib, s!("nvmlDeviceGetCount_v2"), FnCount);
        let by_index: Option<FnHandle> = sym!(lib, s!("nvmlDeviceGetHandleByIndex_v2"), FnHandle);
        let mut n = 0u32;
        let mut dev: Device = std::ptr::null_mut();
        let have_dev = match (count, by_index) {
            (Some(c), Some(h)) => c(&mut n) == 0 && n > 0 && h(0, &mut dev) == 0,
            _ => false,
        };
        if have_dev {
            if let Some(bar1) = sym!(lib, s!("nvmlDeviceGetBAR1MemoryInfo"), FnBar1) {
                let mut m = Bar1Memory::default();
                if bar1(dev, &mut m) == 0 {
                    f.bar1_total_mb = Some(m.total / (1024 * 1024));
                }
            }
            let get_u32 = |name: windows::core::PCSTR| -> Option<u32> {
                let func: FnDevU32 = sym!(lib, name, FnDevU32)?;
                let mut v = 0u32;
                (func(dev, &mut v) == 0).then_some(v)
            };
            f.pcie_gen_current = get_u32(s!("nvmlDeviceGetCurrPcieLinkGeneration"));
            f.pcie_gen_max = get_u32(s!("nvmlDeviceGetMaxPcieLinkGeneration"));
            f.pcie_width_current = get_u32(s!("nvmlDeviceGetCurrPcieLinkWidth"));
            f.pcie_width_max = get_u32(s!("nvmlDeviceGetMaxPcieLinkWidth"));
            if let Some(temp) = sym!(lib, s!("nvmlDeviceGetTemperature"), FnTemp) {
                let mut t = 0u32;
                if temp(dev, 0, &mut t) == 0 {
                    f.temperature_c = Some(t);
                }
            }
            if let Some(util) = sym!(lib, s!("nvmlDeviceGetUtilizationRates"), FnUtil) {
                let mut u = Utilization::default();
                if util(dev, &mut u) == 0 {
                    f.gpu_util_pct = Some(u.gpu);
                }
            }
        }
        if let Some(sd) = shutdown {
            sd();
        }
        let _ = PCWSTR::null();
        Some(f)
    }
}

pub fn scan_gpus() -> Vec<GpuFacts> {
    let mut gpus = adapters();
    if gpus.iter().any(|g| g.is_nvidia()) {
        let nv = nvml_facts();
        if let Some(g) = gpus.iter_mut().find(|g| g.is_nvidia()) {
            g.nvml = nv;
        }
    }
    gpus
}
