use std::{
    mem, ptr,
    sync::atomic::{AtomicU32, Ordering},
};
use std::arch::asm;
use comptr::ComPtr;
use winapi::ctypes::c_void;
use winapi::shared::d3d9::*;
use winapi::shared::d3d9caps::D3DCAPS9;
use winapi::shared::d3d9types::*;
use winapi::shared::dxgi;
use winapi::shared::windef::{HMONITOR, HWND};
use winapi::um::winuser;
use winapi::Interface;
use winapi::{
    shared::d3d9::{IDirect3D9, IDirect3D9Vtbl},
    um::unknwnbase::{IUnknown, IUnknownVtbl},
};

use com_impl::{implementation, interface, ComInterface};

use super::{
    fmt::{is_depth_stencil_format, is_display_mode_format},
    *,
};
use crate::{dev::Device, Error, Result};

/// D3D9 interface which stores all application context.
///
/// Similar in role to a DXGI factory.
#[interface(IDirect3D9)]
pub struct Context {
    refs: AtomicU32,
    factory: ComPtr<dxgi::IDXGIFactory>,
    adapters: Vec<Adapter>,
}

impl Context {
    /// Creates a new D3D9 context.
    pub fn new() -> Result<ComPtr<Context>> {
        // We first have to create a factory, which is the equivalent of this interface in DXGI terms.
        let factory = unsafe {
            let uuid = dxgi::IDXGIFactory::uuidof();
            let mut factory: *mut dxgi::IDXGIFactory = ptr::null_mut();

            let result = dxgi::CreateDXGIFactory(&uuid, &mut factory as *mut _ as usize as *mut _);
            check_hresult(result, "Failed to create DXGI factory")?;

            ComPtr::new(factory)
        };

        // Now we can enumerate all the graphics adapters on the system.
        let adapters = (0..)
            .scan(ptr::null_mut(), |adapter, id| unsafe {
                let result = factory.EnumAdapters(id, adapter);
                if result == 0 {
                    Adapter::new(id, *adapter).ok()
                } else {
                    None
                }
            }).fuse()
            .collect();

        let ctx = Self {
            __vtable: Box::new(Self::create_vtable()),
            refs: AtomicU32::new(1),
            factory,
            adapters,
        };

        Ok(unsafe { new_com_interface(ctx) })
    }

    fn check_adapter(&self, adapter: u32) -> Result<&Adapter> {
        self.adapters
            .get(adapter as usize)
            .ok_or(Error::InvalidCall)
    }

    fn check_devty(&self, dev_ty: D3DDEVTYPE) -> Error {
        match dev_ty {
            D3DDEVTYPE_HAL => Error::Success,
            _ => Error::InvalidCall,
        }
    }
}

impl_iunknown!(struct Context: IUnknown, IDirect3D9);

#[implementation(IDirect3D9)]
impl Context {
    /// Used to register a software rasterizer.
    fn register_software_device(&self, init_fn: *mut c_void) -> Error {
        check_not_null(init_fn)?;

        warn!("Application tried to register software device");

        // We don't suppor software rendering, but we report success here since
        // this call would simply allow software rasterization in cases where
        // the graphics adapter does not support it.
        Error::Success
    }

    /// Returns the number of GPUs installed on the system.
    fn get_adapter_count(&self) -> u32 {
        self.adapters.len() as u32
    }

    /// Returns a description of a GPU.
    fn get_adapter_identifier(
        &self,
        adapter: u32,
        // Note: we ignore the flag, since it's only possible value, D3DENUM_WHQL_LEVEL,
        // is deprecated and irrelevant on Wine / newer versions of Windows.
        _flags: u32,
        ident: *mut D3DADAPTER_IDENTIFIER9,
    ) -> Error {
        let adapter = self.check_adapter(adapter)?;
        let ident = check_mut_ref(ident)?;

        *ident = adapter.identifier();

        Error::Success
    }

    /// Returns the number of display modes with a certain format an adapter supports.
    fn get_adapter_mode_count(&self, adapter: u32, fmt: D3DFORMAT) -> u32 {
        self.adapters
            .get(adapter as usize)
            .map(|adapter| adapter.mode_count(fmt))
            .unwrap_or_default()
    }

    /// Retrieves the list of display modes.
    fn enum_adapter_modes(
        &self,
        adapter: u32,
        fmt: D3DFORMAT,
        i: u32,
        mode: *mut D3DDISPLAYMODE,
    ) -> Error {
        let adapter = self.check_adapter(adapter)?;
        let mode = check_mut_ref(mode)?;

        *mode = adapter.mode(fmt, i).ok_or(Error::NotAvailable)?;

        Error::Success
    }

    /// Retrieve the current display mode of the GPU.
    fn get_adapter_display_mode(&self, adapter: u32, mode: *mut D3DDISPLAYMODE) -> Error {
        let monitor = self.get_adapter_monitor(adapter);
        let mode = check_mut_ref(mode)?;

        let mi = unsafe {
            let mut mi: winuser::MONITORINFO = mem::MaybeUninit::uninit().assume_init();
            mi.cbSize = mem::size_of_val(&mi) as u32;
            let result = winuser::GetMonitorInfoW(monitor, &mut mi);
            assert_ne!(result, 0, "Failed to retrieve monitor info");
            mi
        };

        let rc = mi.rcMonitor;

        mode.Width = (rc.right - rc.left) as u32;
        mode.Height = (rc.bottom - rc.top) as u32;
        // 0 indicates an adapter-default rate.
        mode.RefreshRate = 0;
        // This format is usually what modern displays use internally.
        mode.Format = D3DFMT_X8R8G8B8;

        Error::Success
    }

    /// Checks if an adapter is hardware accelerated.
    fn check_device_type(
        &self,
        adapter: u32,
        ty: D3DDEVTYPE,
        adapter_fmt: D3DFORMAT,
        _bb_fmt: D3DFORMAT,
        _windowed: u32,
    ) -> Error {
        self.check_adapter(adapter)?;
        self.check_devty(ty)?;

        // We support hardware accel with all valid formats.
        if is_display_mode_format(adapter_fmt) {
            Error::Success
        } else {
            Error::NotAvailable
        }
    }

    /// Checks if a certain format can be used for something.
    fn check_device_format(
        &self,
        adapter: u32,
        ty: D3DDEVTYPE,
        _adapter_fmt: D3DFORMAT,
        usage: UsageFlags,
        rt: ResourceType,
        check_fmt: D3DFORMAT,
    ) -> Error {
        let adapter = self.check_adapter(adapter)?;
        self.check_devty(ty)?;

        if adapter.is_format_supported(check_fmt, rt, usage) {
            Error::Success
        } else {
            Error::NotAvailable
        }
    }

    /// Checks if a format can be used with multisampling.
    fn check_device_multi_sample_type(
        &self,
        adapter: u32,
        ty: D3DDEVTYPE,
        surface_fmt: D3DFORMAT,
        _windowed: u32,
        mst: D3DMULTISAMPLE_TYPE,
        quality: *mut u32,
    ) -> Error {
        let adapter = self.check_adapter(adapter)?;
        self.check_devty(ty)?;

        let quality = check_mut_ref(quality);

        let q = adapter.is_multisampling_supported(surface_fmt, mst);

        // Return the maximum quality level, if requested.
        if let Ok(quality) = quality {
            *quality = q;
        }

        // Max quality of 0 would mean no support for MS.
        if q == 0 {
            Error::NotAvailable
        } else {
            Error::Success
        }
    }

    /// Checks if a depth/stencil format can be used with a RT format.
    fn check_depth_stencil_match(
        &self,
        adapter: u32,
        ty: D3DDEVTYPE,
        _adapter_fmt: D3DFORMAT,
        _rt_fmt: D3DFORMAT,
        ds_fmt: D3DFORMAT,
    ) -> Error {
        self.check_adapter(adapter)?;
        self.check_devty(ty)?;

        // We don't check the adapter fmt / render target fmt since on modern GPUs
        // basically any valid combination of formats is allowed.

        // We only have to check that the format which was passed in
        // can be used with d/s buffers.
        if is_depth_stencil_format(ds_fmt) {
            Error::Success
        } else {
            Error::NotAvailable
        }
    }

    /// Checks if a conversion between two given formats is supported.
    fn check_device_format_conversion(
        &self,
        adapter: u32,
        ty: D3DDEVTYPE,
        _src_fmt: D3DFORMAT,
        _tgt_fmt: D3DFORMAT,
    ) -> Error {
        self.check_adapter(adapter)?;
        self.check_devty(ty)?;

        // For most types we can simply convert them to the right format on-the-fly.
        // TODO: we should at least validate the formats to make sure they are valid for back buffers.

        Error::Success
    }

    /// Returns a structure describing the features and limits of an adapter.
    fn get_device_caps(&self, adapter: u32, ty: D3DDEVTYPE, caps: *mut D3DCAPS9) -> Error {
        let adapter = self.check_adapter(adapter)?;
        self.check_devty(ty)?;
        let caps = check_mut_ref(caps)?;

        *caps = adapter.caps();

        Error::Success
    }

    /// Retrieves the monitor associated with an adapter.
    fn get_adapter_monitor(&self, adapter: u32) -> HMONITOR {
        self.check_adapter(adapter)
            .map(|adapter| adapter.monitor())
            .unwrap_or(ptr::null_mut())
    }

    /// Creates a logical device from an adapter.
    fn create_device(
        &self,
        adapter: u32,
        ty: D3DDEVTYPE,
        focus: HWND,
        flags: u32,
        pp: *mut D3DPRESENT_PARAMETERS,
        device: *mut *mut Device,
    ) -> Error {
        self.check_devty(ty)?;
        let ret = check_mut_ref(device)?;

        // TODO: support using multiple GPUs
        if flags & D3DCREATE_ADAPTERGROUP_DEVICE != 0 {
            warn!("Application requested the creation of a multi-GPU logical device");
        }

        if flags & D3DCREATE_FPU_PRESERVE == 0 {
            // We need to set the right x87 control word to disable FPU exceptions.
            unsafe {
                // First we need to retrieve its current value.
                let mut c = 0u16;
                asm!("fnstcw $0" : "=*m"(&c) : : : "volatile");

                // Clear (some of) the control word's bits:
                // - Sets rounding mode to nearest even.
                // - Enable single precision floats.
                c &= 0b11_11_00_00_11 << 6;

                // Mask all exceptions.
                c |= (1 << 6) - 1;

                asm!("fldcw $0" : "*m"(&c) : : : "volatile")
            }
        }

        // This struct stores the original device creation parameters.
        let cp = D3DDEVICE_CREATION_PARAMETERS {
            AdapterOrdinal: adapter,
            DeviceType: D3DDEVTYPE_HAL,
            hFocusWindow: focus,
            BehaviorFlags: flags,
        };

        // This structure describes some settings for the back buffer(s).
        // Since we don't support multiple adapters, we only use the first param in the array.
        let pp = check_mut_ref(pp)?;

        // Create the actual device.
        *ret = crate::Device::new(
            self,
            self.check_adapter(adapter)?,
            cp,
            pp,
            self.factory.clone(),
        )?.into();

        Error::Success
    }
}
