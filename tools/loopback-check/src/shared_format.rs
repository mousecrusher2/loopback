use std::mem::ManuallyDrop;
use std::ffi::c_void;
use std::slice;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use wasapi::{Direction, WaveFormat};
use windows::Win32::Media::Audio::{
    IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, PKEY_AudioEngine_DeviceFormat,
    WAVEFORMATEX,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0, PropVariantClear,
};
use windows::Win32::System::Com::{BLOB, CLSCTX_ALL, CoCreateInstance, STGM_READ, STGM_READWRITE};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::imp::CanInto;
use windows::core::{GUID, HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, Interface, PCWSTR};

use crate::devices::select_device;
use crate::types::{AudioConfig, CaptureMode, DeviceSelector, SharedFormatMode};

const CHANNELS: u16 = 2;
const CHANNEL_MASK_STEREO: u32 = 0x3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xfffe;
const WAVEFORMATEXTENSIBLE_CB_SIZE: u16 = 22;
const SUBTYPE_PCM_BYTES: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];

pub struct SharedFormatGuard {
    endpoint_id: String,
    original_blob: Option<Vec<u8>>,
}

impl SharedFormatGuard {
    pub fn restore(mut self) -> Result<()> {
        if let Some(blob) = self.original_blob.take() {
            set_device_format_blob(&self.endpoint_id, &blob).with_context(|| {
                format!("restore capture shared-mode format on {}", self.endpoint_id)
            })?;
            wait_for_audio_service_reconfigure();
        }
        Ok(())
    }
}

pub fn prepare_capture_shared_format(
    selector: &DeviceSelector,
    config: &AudioConfig,
    mode: SharedFormatMode,
) -> Result<Option<SharedFormatGuard>> {
    if config.capture_mode != CaptureMode::Shared || mode == SharedFormatMode::Leave {
        return Ok(None);
    }

    wasapi::initialize_sta().ok()?;
    let capture = select_device(Direction::Capture, selector)?;
    let endpoint_id = capture.get_id()?;
    if device_format_matches_config(&capture.get_device_format()?, config) {
        return Ok(None);
    }
    let original_blob = get_device_format_blob(&endpoint_id)
        .with_context(|| format!("read capture shared-mode format from {endpoint_id}"))?;
    let target_blob = make_waveformat_extensible_blob(config.rate, config.bits);

    set_device_format_blob(&endpoint_id, &target_blob).with_context(|| {
        format!(
            "set capture shared-mode format to {} Hz {}-bit on {}",
            config.rate, config.bits, endpoint_id
        )
    })?;
    wait_for_audio_service_reconfigure();
    if let Err(err) = verify_device_format(selector, config) {
        if mode == SharedFormatMode::SetRestore {
            let _ = set_device_format_blob(&endpoint_id, &original_blob);
            wait_for_audio_service_reconfigure();
        }
        return Err(err);
    }

    let original_blob = if mode == SharedFormatMode::SetRestore {
        Some(original_blob)
    } else {
        None
    };
    Ok(Some(SharedFormatGuard {
        endpoint_id,
        original_blob,
    }))
}

fn verify_device_format(selector: &DeviceSelector, config: &AudioConfig) -> Result<()> {
    let mut last_error = None;
    let mut format = None;
    for _ in 0..8 {
        match select_device(Direction::Capture, selector).and_then(|device| {
            device
                .get_device_format()
                .map_err(|err| anyhow::anyhow!("{err:#}"))
        }) {
            Ok(value) => {
                format = Some(value);
                break;
            }
            Err(err) => {
                last_error = Some(err);
                wait_for_audio_service_reconfigure();
            }
        }
    }
    let format = match format {
        Some(format) => format,
        None => return Err(last_error.expect("format retry records the last error")),
    };
    if !device_format_matches_config(&format, config) {
        bail!(
            "capture shared-mode format did not switch to requested format: requested {} Hz {}-bit stereo, got {} Hz {} valid / {} container bits, {} channels",
            config.rate,
            config.bits,
            format.get_samplespersec(),
            format.get_validbitspersample(),
            format.get_bitspersample(),
            format.get_nchannels()
        );
    }
    Ok(())
}

fn device_format_matches_config(format: &WaveFormat, config: &AudioConfig) -> bool {
    let valid_bits = format.get_validbitspersample();
    format.get_samplespersec() == config.rate
        && format.get_nchannels() == CHANNELS
        && format.get_bitspersample() == config.bits
        && (valid_bits == 0 || valid_bits == config.bits)
}

fn get_device_format_blob(endpoint_id: &str) -> Result<Vec<u8>> {
    let device = get_imm_device(endpoint_id)?;
    let store = unsafe { device.OpenPropertyStore(STGM_READ)? };
    let mut prop = unsafe { store.GetValue(&PKEY_AudioEngine_DeviceFormat)? };
    let result = parse_blob_property(&prop);
    unsafe { PropVariantClear(&mut prop) }?;
    result
}

fn set_device_format_blob(endpoint_id: &str, blob: &[u8]) -> Result<()> {
    match set_device_format_blob_property_store(endpoint_id, blob) {
        Ok(()) => Ok(()),
        Err(store_err) => set_device_format_blob_policy_config(endpoint_id, blob).with_context(
            || {
                format!(
                    "property-store write failed first: {store_err:#}; IPolicyConfig fallback also failed"
                )
            },
        ),
    }
}

fn set_device_format_blob_property_store(endpoint_id: &str, blob: &[u8]) -> Result<()> {
    let device = get_imm_device(endpoint_id)?;
    let store = unsafe { device.OpenPropertyStore(STGM_READWRITE)? };
    let prop = make_blob_propvariant(blob);
    unsafe {
        store.SetValue(&PKEY_AudioEngine_DeviceFormat, &prop)?;
        store.Commit()?;
    }
    Ok(())
}

fn set_device_format_blob_policy_config(endpoint_id: &str, blob: &[u8]) -> Result<()> {
    let mut format = WaveFormat::parse_from_blob_bytes(blob)?.wave_fmt;
    let endpoint_id = HSTRING::from(endpoint_id);
    let policy: IPolicyConfig =
        unsafe { CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)? };
    unsafe {
        policy.set_device_format(
            PCWSTR::from_raw(endpoint_id.as_ptr()),
            &mut format.Format,
            &mut format.Format,
        )
    }?;
    Ok(())
}

fn get_imm_device(endpoint_id: &str) -> Result<IMMDevice> {
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let endpoint_id = HSTRING::from(endpoint_id);
    let device = unsafe { enumerator.GetDevice(PCWSTR::from_raw(endpoint_id.as_ptr()))? };
    Ok(device)
}

const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

#[repr(transparent)]
#[derive(Clone, PartialEq, Eq)]
struct IPolicyConfig(IUnknown);

impl CanInto<IUnknown> for IPolicyConfig {}

impl IPolicyConfig {
    unsafe fn set_device_format(
        &self,
        device_name: PCWSTR,
        endpoint_format: *mut WAVEFORMATEX,
        mix_format: *mut WAVEFORMATEX,
    ) -> windows::core::Result<()> {
        unsafe {
            (Interface::vtable(self).SetDeviceFormat)(
                Interface::as_raw(self),
                device_name,
                endpoint_format,
                mix_format,
            )
            .ok()
        }
    }
}

unsafe impl Interface for IPolicyConfig {
    type Vtable = IPolicyConfigVtbl;
    const IID: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
}

#[repr(C)]
#[allow(non_snake_case)]
struct IPolicyConfigVtbl {
    base__: IUnknown_Vtbl,
    GetMixFormat: unsafe extern "system" fn(*mut c_void, PCWSTR, *mut *mut WAVEFORMATEX) -> HRESULT,
    GetDeviceFormat:
        unsafe extern "system" fn(*mut c_void, PCWSTR, i32, *mut *mut WAVEFORMATEX) -> HRESULT,
    ResetDeviceFormat: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    SetDeviceFormat:
        unsafe extern "system" fn(*mut c_void, PCWSTR, *mut WAVEFORMATEX, *mut WAVEFORMATEX) -> HRESULT,
}

fn parse_blob_property(prop: &PROPVARIANT) -> Result<Vec<u8>> {
    if prop.vt() != VT_BLOB {
        bail!("PKEY_AudioEngine_DeviceFormat was not VT_BLOB");
    }
    let blob = unsafe { prop.Anonymous.Anonymous.Anonymous.blob };
    let blob_slice = unsafe { slice::from_raw_parts(blob.pBlobData, blob.cbSize as usize) };
    Ok(blob_slice.to_vec())
}

fn make_blob_propvariant(blob: &[u8]) -> PROPVARIANT {
    PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: blob.len() as u32,
                        pBlobData: blob.as_ptr() as *mut u8,
                    },
                },
            }),
        },
    }
}

fn make_waveformat_extensible_blob(rate: u32, bits: u16) -> Vec<u8> {
    let block_align = CHANNELS * (bits / 8);
    let avg_bytes_per_sec = rate * u32::from(block_align);
    let mut blob = Vec::with_capacity(40);
    push_u16(&mut blob, WAVE_FORMAT_EXTENSIBLE);
    push_u16(&mut blob, CHANNELS);
    push_u32(&mut blob, rate);
    push_u32(&mut blob, avg_bytes_per_sec);
    push_u16(&mut blob, block_align);
    push_u16(&mut blob, bits);
    push_u16(&mut blob, WAVEFORMATEXTENSIBLE_CB_SIZE);
    push_u16(&mut blob, bits);
    push_u32(&mut blob, CHANNEL_MASK_STEREO);
    blob.extend_from_slice(&SUBTYPE_PCM_BYTES);
    blob
}

fn push_u16(blob: &mut Vec<u8>, value: u16) {
    blob.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(blob: &mut Vec<u8>, value: u32) {
    blob.extend_from_slice(&value.to_le_bytes());
}

fn wait_for_audio_service_reconfigure() {
    thread::sleep(Duration::from_millis(750));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveformat_extensible_blob_matches_48k_24bit_stereo() {
        let blob = make_waveformat_extensible_blob(48_000, 24);

        assert_eq!(blob.len(), 40);
        assert_eq!(&blob[0..2], &WAVE_FORMAT_EXTENSIBLE.to_le_bytes());
        assert_eq!(&blob[2..4], &CHANNELS.to_le_bytes());
        assert_eq!(&blob[4..8], &48_000u32.to_le_bytes());
        assert_eq!(&blob[8..12], &288_000u32.to_le_bytes());
        assert_eq!(&blob[12..14], &6u16.to_le_bytes());
        assert_eq!(&blob[14..16], &24u16.to_le_bytes());
        assert_eq!(&blob[16..18], &WAVEFORMATEXTENSIBLE_CB_SIZE.to_le_bytes());
        assert_eq!(&blob[18..20], &24u16.to_le_bytes());
        assert_eq!(&blob[20..24], &CHANNEL_MASK_STEREO.to_le_bytes());
        assert_eq!(&blob[24..40], &SUBTYPE_PCM_BYTES);
    }
}
