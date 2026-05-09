use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::slice;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use wasapi::{Direction, WaveFormat};
use windows::Win32::Media::Audio::{
    IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, PKEY_AudioEngine_DeviceFormat, WAVEFORMATEX,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0, PropVariantClear,
};
use windows::Win32::System::Com::{BLOB, CLSCTX_ALL, CoCreateInstance, STGM_READ, STGM_READWRITE};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::imp::CanInto;
use windows::core::{GUID, HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, Interface, PCWSTR};

use crate::devices::select_device;
use crate::types::{
    AudioConfig, CaptureMode, DeviceSelector, SampleRate, SampleWidth, SharedFormatMode,
};

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
    let target_blob = make_waveformat_extensible_blob(config.sample_rate, config.sample_width);

    set_device_format_blob(&endpoint_id, &target_blob).with_context(|| {
        format!(
            "set capture shared-mode format to {} Hz {}-bit on {}",
            config.rate_hz(),
            config.bits_per_sample(),
            endpoint_id
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
        None => return Err(last_error.unwrap_or_else(|| anyhow!("could not read device format"))),
    };
    if !device_format_matches_config(&format, config) {
        bail!(
            "capture shared-mode format did not switch to requested format: requested {} Hz {}-bit stereo, got {} Hz {} valid / {} container bits, {} channels",
            config.rate_hz(),
            config.bits_per_sample(),
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
    format.get_samplespersec() == config.rate_hz()
        && format.get_nchannels() == CHANNELS
        && format.get_bitspersample() == config.bits_per_sample()
        && (valid_bits == 0 || valid_bits == config.bits_per_sample())
}

fn get_device_format_blob(endpoint_id: &str) -> Result<Vec<u8>> {
    let device = get_imm_device(endpoint_id)?;
    // SAFETY: `device` is a live IMMDevice obtained from the COM enumerator,
    // and STGM_READ is a valid property-store access mode.
    let store = unsafe { device.OpenPropertyStore(STGM_READ)? };
    // SAFETY: `store` is a live IPropertyStore and the property key is a
    // Windows-defined constant for the endpoint format blob.
    let mut prop = unsafe { store.GetValue(&PKEY_AudioEngine_DeviceFormat)? };
    let result = parse_blob_property(&prop);
    // SAFETY: `prop` was initialized by `GetValue`. `parse_blob_property` has
    // already copied any blob bytes before the PROPVARIANT is cleared.
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
    // SAFETY: `device` is a live IMMDevice obtained from the COM enumerator,
    // and STGM_READWRITE requests a valid writable property store.
    let store = unsafe { device.OpenPropertyStore(STGM_READWRITE)? };
    let prop = make_blob_propvariant(blob);
    // SAFETY: `store` is a live IPropertyStore. `prop` points at `blob`, which
    // remains valid for both calls, and IPropertyStore copies the PROPVARIANT
    // value during `SetValue`.
    unsafe {
        store.SetValue(&PKEY_AudioEngine_DeviceFormat, &prop)?;
        store.Commit()?;
    }
    Ok(())
}

fn set_device_format_blob_policy_config(endpoint_id: &str, blob: &[u8]) -> Result<()> {
    let mut format = WaveFormat::parse_from_blob_bytes(blob)?.wave_fmt;
    let endpoint_id = HSTRING::from(endpoint_id);
    // SAFETY: COM is initialized by `prepare_capture_shared_format`, and the
    // CLSID/IID pair is the known Windows PolicyConfig client used by the audio
    // settings UI.
    let policy: IPolicyConfig =
        unsafe { CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)? };
    // SAFETY: `endpoint_id` and `format` live for the duration of the call.
    // Both format pointers reference the same mutable WAVEFORMATEX, which is
    // accepted by IPolicyConfig when setting endpoint and mix format together.
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
    // SAFETY: COM is initialized by `prepare_capture_shared_format`, and
    // MMDeviceEnumerator is the system-provided COM class for audio endpoints.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let endpoint_id = HSTRING::from(endpoint_id);
    // SAFETY: `endpoint_id` is an HSTRING that stays alive for the call, and
    // Windows HSTRING buffers are NUL-terminated for PCWSTR interop.
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
        // SAFETY: `self` is an IPolicyConfig COM interface with the vtable
        // declared below. The caller supplies pointers that stay valid for the
        // duration of the COM call.
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

// SAFETY: `IPolicyConfig` is a transparent wrapper around `IUnknown`, and the
// IID/vtable layout below matches the Windows PolicyConfig interface used by
// the audio control panel for the methods this tool calls.
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
    SetDeviceFormat: unsafe extern "system" fn(
        *mut c_void,
        PCWSTR,
        *mut WAVEFORMATEX,
        *mut WAVEFORMATEX,
    ) -> HRESULT,
}

fn parse_blob_property(prop: &PROPVARIANT) -> Result<Vec<u8>> {
    if prop.vt() != VT_BLOB {
        bail!("PKEY_AudioEngine_DeviceFormat was not VT_BLOB");
    }
    // SAFETY: The active PROPVARIANT tag was checked as VT_BLOB, so reading the
    // `blob` union field is valid.
    let blob = unsafe { prop.Anonymous.Anonymous.Anonymous.blob };
    if blob.cbSize == 0 {
        return Ok(Vec::new());
    }
    if blob.pBlobData.is_null() {
        bail!("PKEY_AudioEngine_DeviceFormat blob had a null data pointer");
    }
    // SAFETY: `pBlobData` is non-null and owned by the live PROPVARIANT for
    // `cbSize` bytes. The caller clears the PROPVARIANT only after this slice
    // has been copied into a Vec.
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

fn make_waveformat_extensible_blob(rate: SampleRate, width: SampleWidth) -> Vec<u8> {
    let rate_hz = rate.hz();
    let bits = width.bits();
    let block_align = CHANNELS * (bits / 8);
    let avg_bytes_per_sec = rate_hz * u32::from(block_align);
    let mut blob = Vec::with_capacity(40);
    push_u16(&mut blob, WAVE_FORMAT_EXTENSIBLE);
    push_u16(&mut blob, CHANNELS);
    push_u32(&mut blob, rate_hz);
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
        let blob = make_waveformat_extensible_blob(SampleRate::R48000, SampleWidth::Bits24);

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

    #[test]
    fn waveformat_extensible_blob_matches_48k_32bit_stereo() {
        let blob = make_waveformat_extensible_blob(SampleRate::R48000, SampleWidth::Bits32);

        assert_eq!(blob.len(), 40);
        assert_eq!(&blob[8..12], &384_000u32.to_le_bytes());
        assert_eq!(&blob[12..14], &8u16.to_le_bytes());
        assert_eq!(&blob[14..16], &32u16.to_le_bytes());
        assert_eq!(&blob[18..20], &32u16.to_le_bytes());
    }
}
