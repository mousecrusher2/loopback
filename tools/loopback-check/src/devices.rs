use anyhow::{Context, Result, bail};
use wasapi::{Device, DeviceEnumerator, Direction, WaveFormat};

use crate::types::{DeviceSelector, DeviceSummary};

pub fn list_command() -> Result<()> {
    for direction in [Direction::Render, Direction::Capture] {
        println!("{direction:?}:");
        for (index, device) in list_devices(direction)?.iter().enumerate() {
            println!("  [{index}] {}", device.friendly_name);
            println!("      id: {}", device.id);
            println!("      interface: {}", device.interface_name);
            println!("      description: {}", device.description);
            if let Some(format) = &device.device_format {
                println!("      shared format: {format}");
            }
        }
    }
    Ok(())
}

pub fn select_device(direction: Direction, selector: &DeviceSelector) -> Result<Device> {
    let id = match direction {
        Direction::Render => selector.render_id.as_ref(),
        Direction::Capture => selector.capture_id.as_ref(),
    };

    let devices = active_devices(direction)?;
    if let Some(id) = id {
        for selected in devices {
            if selected.summary.id == *id {
                return Ok(selected.device);
            }
        }
        bail!("no active {direction:?} endpoint id {id}");
    }

    let query = selector.query.to_ascii_lowercase();
    let mut matches = Vec::new();
    let mut names = Vec::new();
    for selected in devices {
        names.push(format!(
            "  {} ({})",
            selected.summary.friendly_name, selected.summary.id
        ));
        let is_match = {
            let haystack = format!(
                "{}\n{}\n{}\n{}",
                selected.summary.id,
                selected.summary.friendly_name,
                selected.summary.interface_name,
                selected.summary.description
            )
            .to_ascii_lowercase();
            haystack.contains(&query)
        };
        if is_match {
            matches.push(selected);
        }
    }

    match matches.as_slice() {
        [_] => Ok(matches.remove(0).device),
        [] => {
            bail!(
                "no {direction:?} endpoint matched {:?}\n{}",
                selector.query,
                names.join("\n")
            )
        }
        many => {
            let names = many
                .iter()
                .map(|device| {
                    format!(
                        "  {} ({})",
                        device.summary.friendly_name, device.summary.id
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            bail!(
                "multiple {direction:?} endpoints matched {:?}; pass --{}-id\n{}",
                selector.query,
                if direction == Direction::Render {
                    "render"
                } else {
                    "capture"
                },
                names
            )
        }
    }
}

struct SelectedDevice {
    device: Device,
    summary: DeviceSummary,
}

fn active_devices(direction: Direction) -> Result<Vec<SelectedDevice>> {
    let enumerator = DeviceEnumerator::new()?;
    let collection = enumerator.get_device_collection(&direction)?;
    let mut devices = Vec::new();
    for device in &collection {
        let device = device?;
        let summary = summarize_device(direction, &device)?;
        devices.push(SelectedDevice { device, summary });
    }
    Ok(devices)
}

pub fn probe_command(selector: &DeviceSelector) -> Result<()> {
    for direction in [Direction::Render, Direction::Capture] {
        let device = select_device(direction, selector)?;
        let client = device
            .get_iaudioclient()
            .with_context(|| format!("activate IAudioClient for {direction:?} endpoint"))?;
        let format = client
            .get_mixformat()
            .with_context(|| format!("get {direction:?} mix format"))?;
        println!("{direction:?}: {}", describe_format(&format));
    }
    Ok(())
}

fn list_devices(direction: Direction) -> Result<Vec<DeviceSummary>> {
    active_devices(direction).map(|devices| {
        devices
            .into_iter()
            .map(|selected| selected.summary)
            .collect()
    })
}

fn summarize_device(direction: Direction, device: &Device) -> Result<DeviceSummary> {
    let format = device
        .get_device_format()
        .ok()
        .map(|format| describe_format(&format));
    Ok(DeviceSummary {
        direction: format!("{direction:?}"),
        id: device.get_id()?,
        friendly_name: device.get_friendlyname()?,
        interface_name: device.get_interface_friendlyname().unwrap_or_default(),
        description: device.get_description().unwrap_or_default(),
        device_format: format,
    })
}

fn describe_format(format: &WaveFormat) -> String {
    let sample_type = format
        .get_subformat()
        .map(|ty| ty.to_string())
        .unwrap_or_else(|_| "unknown".to_owned());
    format!(
        "{} Hz, {} ch, {} valid / {} container bits, {}, block {}",
        format.get_samplespersec(),
        format.get_nchannels(),
        format.get_validbitspersample(),
        format.get_bitspersample(),
        sample_type,
        format.get_blockalign()
    )
}
