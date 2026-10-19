# How Omega reaches the RTX 5070 on Windows 11: through WSL2

Status: research note and plan, 2026-10-19. Nothing here has run on an RTX
5070 yet. Every claim below is either quoted from a linked source or read from
code we can name; anything else is marked as not known.

**Decision (Drake, 2026-10-19): the brother's PC uses WSL2. WSL2 is the GPU
path for Omega.** The Windows app (`aien-rig.exe`) starts the engine inside
WSL2 and talks to it over `127.0.0.1`. A Linux USB stick is kept only as a
labelled fallback (last section).

## 1. What Omega needs today, and what changes under WSL2

Omega does not use CUDA or libcuda. On the Spark it opens NVIDIA's Linux
kernel driver directly (`physics/nvrm/nvrm.c` in aien-dev/physics-mv):

| Omega on Linux today (`nvrm.c`) | Under WSL2 there is instead |
|---|---|
| `/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-uvm` | `/dev/dxg` only |
| `NV_ESC_RM_ALLOC` of root client, device, VA space | `LX_DXOPENADAPTERFROMLUID`, `LX_DXCREATEDEVICE`, `LX_DXCREATEPAGINGQUEUE` |
| `NV01_MEMORY_SYSTEM` + `NV_ESC_RM_MAP_MEMORY` | `LX_DXCREATEALLOCATION` + `LX_DXLOCK2` (CPU map) + `LX_DXMAPGPUVIRTUALADDRESS` / `LX_DXRESERVEGPUVIRTUALADDRESS` |
| GPFIFO channel (`BLACKWELL_CHANNEL_GPFIFO_A`) + compute object (`BLACKWELL_COMPUTE_B`) | `LX_DXCREATEHWQUEUE` (hardware scheduling) or `LX_DXCREATECONTEXTVIRTUAL` (packet scheduling) |
| Doorbell write to the `HOPPER_USERMODE_A` page | `LX_DXSUBMITCOMMANDTOHWQUEUE` or `LX_DXSUBMITCOMMAND` (an ioctl per submit; no doorbell ioctls exist in WSL's dxgkrnl) |
| Semaphore release + CPU poll | Monitored fence sync objects: `LX_DXCREATESYNCHRONIZATIONOBJECT`, `LX_DXWAITFORSYNCHRONIZATIONOBJECTFROMCPU` |
| `NV_ESC_RM_CONTROL` (class list, GPU info) | `LX_DXQUERYADAPTERINFO`, and `LX_DXESCAPE` for anything driver private |

The left column is our code. The right column is the public WSL kernel
interface: `include/uapi/misc/d3dkmthk.h` in Microsoft's WSL kernel
(branch `linux-msft-wsl-6.6.y`) defines 64 `LX_DX*` ioctls, all with magic
`0x47`, implemented as `dxgkio_*` functions in `drivers/hv/dxgkrnl/ioctl.c`.
Sources:
https://github.com/microsoft/WSL2-Linux-Kernel/blob/linux-msft-wsl-6.6.y/include/uapi/misc/d3dkmthk.h
and https://github.com/microsoft/WSL2-Linux-Kernel/tree/linux-msft-wsl-6.6.y/drivers/hv/dxgkrnl

What stays the same: the GPU itself. Omega's instruction encoder, its
push-buffer (method) writer and its scheduling logic do not care whether the
buffer reaches the GPU through `nvidiactl` or `dxg`. What changes is the layer
`nvrm.c` implements today. The plan is a second backend, `dxg.c`, behind the
same `nvrm.h` interface (open, alloc, map, submit, wait, free), written in C
like the rest of Omega.

## 2. How `/dev/dxg` works (public sources)

- Microsoft: "/dev/dxg exposes a set of IOCTL that closely mimic the native
  WDDM D3DKMT kernel service layer on Windows." The Linux driver forwards each
  call over the Hyper-V VM bus to the Windows graphics kernel, which hands it to
  the real NVIDIA WDDM driver (`nvlddmkm.sys`). "There is no partitioning of
  resources between Linux and Windows."
  https://devblogs.microsoft.com/directx/directx-heart-linux/
- The driver's design note, sent upstream by Microsoft: user mode drivers "open
  the device and send ioctls to create various device objects ... and to submit
  work to the device." It separates `dxgcontext` (packet scheduling mode) from
  `dxghwqueue` (hardware scheduling mode).
  https://lwn.net/Articles/881311/
- Submit ioctls, from the patch series: `LX_DXSUBMITCOMMAND` "is used to submit
  GPU commands when the device supports the packet scheduling mode";
  `LX_DXSUBMITCOMMANDTOHWQUEUE` submits to a hardware queue when the GPU
  schedules its own queues.
  https://patchew.org/linux/cover.1641937419.git.iourit@linux.microsoft.com/
  and https://patchew.org/linux/cover.1644025661.git.iourit@linux.microsoft.com/
- The newest revision of the series (v4, 55 patches) was posted in 2026.
  https://lkml.iu.edu/2603.2/09609.html
- `dxgkio_submit_command` and `dxgkio_submit_command_hwqueue` pass the caller's
  arguments, including the private driver data blob, to the host through
  `dxgvmb_send_submit_command` / `dxgvmb_send_submit_command_hwqueue`
  (`ioctl.c` lines 2265 to 2375 on `linux-msft-wsl-6.6.y`). The Linux side does
  not interpret the blob; NVIDIA's Windows driver does.
- `libdxcore.so` (closed, Microsoft) "is also used to host a flat version of the
  D3DKMT API". Microsoft also publishes `libdxg` (open source): the official
  WDDM kernel API headers plus a library that "translates the WDDM interface
  calls to the /dev/dxg drive IOCTLs".
  https://devblogs.microsoft.com/directx/directx-heart-linux/ and
  https://github.com/microsoft/libdxg
- Omega can skip both libraries and call the ioctls itself with the uapi
  header, the same way it skips libcuda on Linux today.

### The hard part: NVIDIA's private data

Every WDDM object NVIDIA's driver cares about carries a vendor blob:
`pPrivateDriverData` on allocation creation, hardware queue and context
creation, every submit, and every escape. Microsoft: D3D12 "wouldn't be able to
operate without a GPU specific user mode driver (UMD) provided by our GPU
manufacturer partners. The UMD is responsible for ... translating API rendering
requests into actual GPU instructions in command buffers."
(https://devblogs.microsoft.com/directx/directx-heart-linux/). On the Windows
side the kernel driver checks these blobs (`DxgkDdiValidateSubmitCommand`,
https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/d3dkmddi/nc-d3dkmddi-dxgkddi_validatesubmitcommand).

NVIDIA has not published the layout of its blobs. So the dxg backend has two
parts: the ioctl plumbing (public, straightforward) and NVIDIA's private
formats (must be measured on the real card).

## 3. Every public source on raw submission over `/dev/dxg` I found

| Source | What it shows | Use to us |
|---|---|---|
| Microsoft WSL kernel, `drivers/hv/dxgkrnl` + `include/uapi/misc/d3dkmthk.h` | The exact ioctl numbers and structures | The interface `dxg.c` will call |
| Microsoft `libdxg` (https://github.com/microsoft/libdxg) | Open D3DKMT-to-ioctl translation layer and WDDM headers, synced with Windows 25H2 | Reference for argument packing |
| **AMD `librocdxg`** (open source, now in `ROCm/rocm-systems`, `projects/rocr-runtime/libhsakmt/src/dxg`; https://github.com/ROCm/librocdxg) | A complete open-source compute runtime over `/dev/dxg`. It loads `libdxcore.so`, then uses `D3DKMTEnumAdapters3`, `D3DKMTCreateDevice`, `D3DKMTCreatePagingQueue`, `D3DKMTCreateAllocation2`, `D3DKMTMapGpuVirtualAddress`, `D3DKMTLock2`, `D3DKMTCreateHwQueue`, `D3DKMTSubmitCommandToHwQueue` and `D3DKMTEscape` with `D3DKMT_ESCAPE_DRIVERPRIVATE`. It gets a user-mode queue doorbell location back from the AMD driver through private data at hardware queue creation (`wddm/device.cpp`). AMD calls it production with Adrenalin 26.2.2 + ROCm 7.2.1 (https://rocm.docs.amd.com/projects/radeon-ryzen/en/latest/docs/install/installrad/wsl/howto_wsl.html) | **The closest prior art: the exact sequence of calls a raw compute runtime makes over dxg.** AMD wrote both sides, so its private blobs are AMD's own; ours must match NVIDIA's |
| Intel compute-runtime, WDDM on Linux (https://github.com/intel/compute-runtime/blob/master/documentation/WSL.md) | Open-source Level Zero / OpenCL runtime that runs over `/dev/dxg` in WSL2. Microsoft names it plus libdxg as the "fully open source implementation of our virtualized compute stack inside of WSL" (https://lwn.net/Articles/881311/) | Second working example of the call sequence |
| NVIDIA CUDA on WSL user guide (https://docs.nvidia.com/cuda/wsl-user-guide/index.html) | "The CUDA driver installed on Windows host will be stubbed inside the WSL 2 as libcuda.so"; do not install any Linux NVIDIA driver in WSL2 | NVIDIA's own user mode stack is closed and lives in `/usr/lib/wsl/lib` |
| NVIDIA, "Leveling up CUDA performance on WSL2" (https://developer.nvidia.com/blog/leveling-up-cuda-performance-on-wsl2-with-new-enhancements/) | Launch latency fell thanks to "Microsoft's asynchronous Submit call and hardware-accelerated GPU scheduling"; asynchronous paging avoids "VMBUS calls to the host kernel"; gaps remain for "very small, sequential GPU workloads" | Per-submit cost is a VM bus round trip unless batched. Omega must batch many kernels per submit (it already builds long push buffers) |
| `libnvdxgdmal.so.1` (nvidia-container-toolkit issue 520, https://github.com/NVIDIA/nvidia-container-toolkit/issues/520) | Since Windows driver 555, CUDA in WSL2 needs this extra NVIDIA library | NVIDIA added a WSL-only layer of its own; its name suggests a dxg memory layer. Not documented |
| Files in `/usr/lib/wsl/lib` reported by users (https://renenyffenegger.ch/notes/Linux/fhs/usr/lib/wsl/lib/index, https://forums.developer.nvidia.com/t/failure-to-install-cuda-on-wsl-2-ubuntu/128592?page=4) | `libcuda.so*`, `libnvwgf2umx.so` (NVIDIA's D3D12 user mode driver built for Linux), `libnvidia-ml.so.1`, `nvidia-smi`, `libnvdxdlkernels.so`, `libd3d12.so`, `libdxcore.so` | Which NVIDIA user mode pieces talk to `/dev/dxg` and could be observed |
| Mesa D3D12 Gallium and Dozen (Vulkan on D3D12) (https://www.phoronix.com/news/Vulkan-On-Direct3D-12-Dzn-Merge) | Mesa reaches the GPU in WSL through Microsoft's D3D12, not through raw submission | Not a raw path, but a D3D12 compute program is an observable NVIDIA workload that does not involve CUDA |
| WSL issue 41900 (https://github.com/microsoft/WSL/issues/41900) | A user's `strace` of DXCore/D3D12 showing individual `/dev/dxg` ioctls (`_IOC(..., 0x47, 0x13, ...)`) | Shows tracing `/dev/dxg` from user space is ordinary and works |
| Black Hat 2022 "DirectX: The New Hyper-V Attack Surface" (https://i.blackhat.com/USA-22/Thursday/US-22-Hong-DirectX-The-New-Hyper-V-Attack-Surface.pdf) and Fluid Attacks follow-up (https://fluidattacks.com/blog/offensive-hyperv-directx-1) | Security research on the dxg ioctls and the VM bus messages behind them | Background on the transport, not on NVIDIA's blobs |
| tinygrad | Its NV backend talks to NVIDIA's kernel driver without libcuda, like Omega, but only on Linux (`nvidiactl` or its own PCI driver). A code search of tinygrad/tinygrad for `dxg` finds no WSL path (https://docs.tinygrad.org/runtime/) | No prior art for NVIDIA over dxg |

**Gap:** I found no public project, writeup or discussion that submits its own
command buffers to an NVIDIA GPU through `/dev/dxg` (or D3DKMT on Windows)
without NVIDIA's user mode driver. AMD and Intel prove the transport works for
raw compute runtimes. For NVIDIA, Omega would be first, and the private blob
formats have to be learned on the card.

## 4. The plan for Omega's dxg backend

1. **Probe (ours, public interface only).** A small C program opens
   `/dev/dxg` and calls the ioctls directly. It enumerates adapters, reads
   adapter facts (hardware scheduling on or off, compute engine node, memory
   segments, driver version), creates a device, a paging queue and a fence,
   then tries `CREATEHWQUEUE` / `CREATECONTEXTVIRTUAL` on the compute node and
   `CREATEALLOCATION` with empty private data, recording each status. This tells
   us how much of NVIDIA's private data is required, before any reverse
   engineering.
2. **Observe NVIDIA's own user mode driver, without CUDA.** An `LD_PRELOAD`
   shim of ours (C) wraps `ioctl()` on the `/dev/dxg` file descriptor and logs
   every `LX_DX*` call: the structures, the private blobs, and at each submit
   the command buffer bytes. The workload is a D3D12 compute dispatch through
   Microsoft's `libd3d12.so`, which drives NVIDIA's `libnvwgf2umx.so`.
   `nvidia-smi` (NVML) gives a second, query-only trace. This does not use
   CUDA, but it does run NVIDIA's closed D3D12 driver as the thing we watch:
   **Drake to confirm that is acceptable.**
3. **Replay.** Rebuild the smallest traced sequence in `dxg.c`: one allocation,
   one queue, one push buffer with a semaphore release, one fence wait. Same
   proof as the first GB10 channel test.
4. **Port Omega's push buffers.** Swap `nvrm.c` for `dxg.c` under `nvrm.h`,
   batch many kernels per submit to keep VM bus round trips rare, then run
   Omega's existing gates.

### Other Omega changes the 5070 needs (read from the code, not tested)

- **CPU architecture.** 16 files in `omega/src` contain AArch64-only code
  (`dsb sy`, `dmb`, `__aarch64__` paths), for example `omega_blackwell_submit.c`,
  `omega_blackwell_engine.c`, `runtime/rx_resident_gpu.c`. The PC is x86_64.
- **Instruction encoding.** `omega_blackwell_codegen.c` encodes sm_121 (GB10),
  checked with `nvdisasm -b SM121`. The RTX 5070 is sm_120. Every encoding needs
  a check against sm_120.
- **Memory.** `nvrm.c` only allocates system memory, which is right for the
  GB10's shared memory. The 5070 has 12 GB of its own memory; weights and the KV
  cache must live there. Under WSL2 that means a video-memory segment
  allocation, and the game shares the same 12 GB, which is why the app stops
  the engine while a game runs.

## 5. First experiment on the real 5070 under WSL2

Goal: facts, not speed. Everything goes into one receipt file for `evidence/`.

1. **Windows side (record only):** Windows build (`winver`), NVIDIA driver
   version (`nvidia-smi` in PowerShell), whether "Hardware-accelerated GPU
   scheduling" is on (Settings, System, Display, Graphics), `wsl --version`.
2. **WSL side, install:** `wsl --install -d Ubuntu-24.04`. Do not install any
   NVIDIA driver inside WSL (NVIDIA's rule above).
3. **WSL side, facts:** `uname -a`, `cat /proc/version`, `ls -l /dev/dxg`,
   `ls -l /usr/lib/wsl/lib /usr/lib/wsl/drivers`, `/usr/lib/wsl/lib/nvidia-smi -q`,
   `dmesg | grep -i dxg`, `lscpu`.
4. **Probe (step 1 of the plan):** build and run the probe; save its output:
   adapter LUID and count, `KMTQAITYPE_WDDM_2_7_CAPS` (hardware scheduling
   supported/enabled), node metadata (which node is compute), segment sizes,
   and the NTSTATUS of each create call with empty private data.
5. **Trace (step 2, only if Drake agrees):** run the `LD_PRELOAD` shim around a
   minimal D3D12 compute dispatch and around `nvidia-smi -q`; save the logs.

Who builds what: the probe and the shim belong to the engine stream
(engine/core). The app stream ships the WSL launcher; this document is the
contract between them.

## 6. How the app uses the engine in WSL2

- `aien-rig.exe` runs on Windows. Its Chat tab and the game watcher start the
  engine with `wsl.exe -d <distro> -- bash -lc '<engine command>'`, where the
  default command is
  `aien-serve-http --engine omega --bind 127.0.0.1 --port 18120`, and talk to
  `http://127.0.0.1:18120/v1/...` (the same address rig-create uses by default). WSL2 forwards `localhost` from Windows to
  Linux by default.
- The engine's process id is written to `~/.aien-rig/engine.pid` inside WSL so
  the app can stop exactly that process and nothing else.
- When a game starts, the app stops the engine so the game gets the whole
  12 GB, and starts it again when the game exits (if it was running before).
- Until Omega's dxg backend exists, the engine cannot use the 5070 inside WSL2.
  The app says so on the Chat tab instead of pretending.

## 7. Fallback (labelled): Linux from a USB drive

Kept only in case the dxg route stalls on NVIDIA's private formats. It runs
Omega's existing `nvrm.c` unchanged in kind, because the driver is the same
family:

- NVIDIA: "The proprietary flavor supports ... GPUs until Blackwell. Blackwell
  and later are only supported by the open kernel modules."
  https://download.nvidia.com/XFree86/Linux-x86_64/580.105.08/README/kernel_open.html
- RTX 5070 on Ubuntu 24.04 needs the open modules and a 580-or-newer driver
  (community guide:
  https://github.com/adamn1225/FIXED-NVIDIA-RTX-5070-on-Ubuntu-24.04---Working-580-open-Driver-Guide).
- Install Ubuntu onto an external USB SSD (install target and boot loader both
  on it) so the Windows disk is never touched; pin the open driver to
  580.173.02, the version of Omega's headers. Save the BitLocker recovery key
  first and turn off Fast Startup
  (https://windows-now.com/linux/dual-boot-windows-and-linux/).

## 8. Not known yet

- Whether NVIDIA's driver accepts allocations, queues or submits from a client
  that is not its own user mode driver, and what the private blobs contain.
- Whether hardware-accelerated GPU scheduling must be on for hardware queues
  on the 5070 under WSL2.
- Per-submit cost on the brother's PC (VM bus round trip), measured, not
  guessed.
- Whether sm_121 encodings Omega uses are identical on sm_120.
