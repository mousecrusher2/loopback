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
    let enumerator = DeviceEnumerator::new()?;
    let id = match direction {
        Direction::Render => selector.render_id.as_ref(),
        Direction::Capture => selector.capture_id.as_ref(),
    };
    if let Some(id) = id {
        return enumerator
            .get_device(id)
            .with_context(|| format!("open {direction:?} endpoint id {id}"));
    }

    let summaries = list_devices(direction)?;
    let query = selector.query.to_ascii_lowercase();
    let matches: Vec<_> = summaries
        .iter()
        .filter(|device| {
            let haystack = format!(
                "{}\n{}\n{}\n{}",
                device.id, device.friendly_name, device.interface_name, device.description
            )
            .to_ascii_lowercase();
            haystack.contains(&query)
        })
        .collect();

    match matches.as_slice() {
        [device] => enumerator
            .get_device(&device.id)
            .with_context(|| format!("open selected {direction:?} endpoint")),
        [] => {
            let names = summaries
                .iter()
                .map(|device| format!("  {} ({})", device.friendly_name, device.id))
                .collect::<Vec<_>>()
                .join("\n");
            bail!(
                "no {direction:?} endpoint matched {:?}\n{}",
                selector.query,
                names
            )
        }
        many => {
            let names = many
                .iter()
                .map(|device| format!("  {} ({})", device.friendly_name, device.id))
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

fn list_devices(direction: Direction) -> Result<Vec<DeviceSummary>> {
    let enumerator = DeviceEnumerator::new()?;
    let collection = enumerator.get_device_collection(&direction)?;
    let mut devices = Vec::new();
    for device in &collection {
        let device = device?;
        let format = device
            .get_device_format()
            .ok()
            .map(|format| describe_format(&format));
        devices.push(DeviceSummary {
            direction: format!("{direction:?}"),
            id: device.get_id()?,
            friendly_name: device.get_friendlyname()?,
            interface_name: device.get_interface_friendlyname().unwrap_or_default(),
            description: device.get_description().unwrap_or_default(),
            device_format: format,
        });
    }
    Ok(devices)
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
