use std::{mem, ptr};

use winapi::um::d3d11::*;

use comptr::ComPtr;

use crate::core::*;
use crate::Result;

use super::util::d3d_usage_to_d3d11;

/// Wrapper for a vertex/index buffer.
#[derive(Clone)]
pub struct Buffer {
    buffer: ComPtr<ID3D11Buffer>,
}

impl Buffer {
    /// Creates a vertex/index buffer.
    pub fn new(
        device: &ID3D11Device,
        len: u32,
        usage: UsageFlags,
        pool: MemoryPool,
        bind_flags: u32,
    ) -> Result<Self> {
        let (usage, _, cpu_flags) = d3d_usage_to_d3d11(usage, pool)?;

        let desc = D3D11_BUFFER_DESC {
            ByteWidth: len,
            Usage: usage,
            BindFlags: bind_flags,
            CPUAccessFlags: cpu_flags,
            MiscFlags: 0,
            StructureByteStride: 0,
        };

        let buffer = unsafe {
            let mut ptr = ptr::null_mut();

            let result = device.CreateBuffer(&desc, ptr::null(), &mut ptr);
            check_hresult(result, "Failed to create buffer")?;

            ComPtr::new(ptr)
        };

        Ok(Self { buffer })
    }

    /// Retrieves this buffer as a resource.
    pub fn as_resource(&self) -> *mut ID3D11Resource {
        self.buffer.upcast().as_mut()
    }

    /// Retrieves the description of this buffer.
    pub fn desc(&self) -> D3D11_BUFFER_DESC {
        unsafe {
            let mut buf = mem::MaybeUninit::uninit().assume_init();
            self.buffer.GetDesc(&mut buf);
            buf
        }
    }
}
