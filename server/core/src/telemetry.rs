use std::{fs, path::Path};

use crate::domain::{Allocation, DeviceTelemetry, NpuCore, NpuTopology};

const DEVFREQ_ROOT: &str = "/sys/class/devfreq/27700000.npu";
// RK3576 hardware specification: two NPU cores with 6 TOPS INT8 total.
const RK3576_CORE_COUNT: u8 = 2;
const RK3576_TOTAL_TOPS_INT8: f32 = 6.0;

pub fn read_device_telemetry() -> DeviceTelemetry {
    read_device_telemetry_from(
        Path::new("/sys/class/thermal"),
        Path::new("/proc/meminfo"),
        Path::new("/sys/kernel/debug/rknpu/load"),
    )
}

fn read_device_telemetry_from(
    thermal_root: &Path,
    meminfo_path: &Path,
    npu_load_path: &Path,
) -> DeviceTelemetry {
    let meminfo = read_string(meminfo_path);
    let (memory_total_bytes, memory_available_bytes) = meminfo
        .as_deref()
        .map(parse_meminfo)
        .unwrap_or((None, None));
    let npu_core_load_percent = read_string(npu_load_path)
        .map(|raw| parse_npu_core_load(&raw, RK3576_CORE_COUNT))
        .unwrap_or_else(|| vec![None; usize::from(RK3576_CORE_COUNT)]);

    DeviceTelemetry {
        soc_temperature_c: read_thermal_celsius(thermal_root, "soc-thermal"),
        npu_temperature_c: read_thermal_celsius(thermal_root, "npu-thermal"),
        memory_total_bytes,
        memory_available_bytes,
        npu_core_load_percent,
    }
}

pub fn read_topology(allocations: &[Allocation]) -> NpuTopology {
    let current_frequency_hz = read_u64(Path::new(DEVFREQ_ROOT).join("cur_freq"));
    let available_frequencies_hz = read_string(Path::new(DEVFREQ_ROOT).join("available_frequencies"))
        .map(|raw| {
            raw.split_whitespace()
                .filter_map(|value| value.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    let governor = read_string(Path::new(DEVFREQ_ROOT).join("governor"));
    let load_percent = read_string(Path::new(DEVFREQ_ROOT).join("load"))
        .and_then(|load| load.split('@').next()?.parse::<u8>().ok());

    let cores = (0..RK3576_CORE_COUNT)
        .map(|id| {
            let allocation = allocations
                .iter()
                .find(|allocation| allocation.core_mask.bits() & (1 << id) != 0);
            NpuCore {
                id,
                label: format!("Core {id}"),
                allocation_id: allocation.map(|item| item.lease_id.clone()),
                plugin_id: allocation.map(|item| item.plugin_id.clone()),
            }
        })
        .collect();

    NpuTopology {
        device: "/dev/dri/renderD129".into(),
        platform: read_platform().unwrap_or_else(|| "rk3576".into()),
        driver: "RKNPU".into(),
        driver_version: read_string("/sys/module/rknpu/version"),
        runtime_version: None,
        total_tops_int8: RK3576_TOTAL_TOPS_INT8,
        core_count: RK3576_CORE_COUNT,
        current_frequency_hz,
        available_frequencies_hz,
        load_percent,
        governor,
        cores,
    }
}

fn read_platform() -> Option<String> {
    let uevent = read_string("/sys/class/drm/renderD129/device/uevent")?;
    let compatible = uevent
        .lines()
        .find_map(|line| line.strip_prefix("OF_COMPATIBLE_0="))?;
    Some(compatible.trim_start_matches("rockchip,").to_owned())
}

fn read_thermal_celsius(root: &Path, sensor_type: &str) -> Option<f32> {
    fs::read_dir(root).ok()?.filter_map(Result::ok).find_map(|entry| {
        let path = entry.path();
        (read_string(path.join("type")).as_deref() == Some(sensor_type))
            .then(|| read_u64(path.join("temp")).map(|value| value as f32 / 1000.0))
            .flatten()
    })
}

fn parse_meminfo(raw: &str) -> (Option<u64>, Option<u64>) {
    let value_bytes = |key: &str| {
        raw.lines().find_map(|line| {
            let value = line.strip_prefix(key)?.split_whitespace().next()?;
            value.parse::<u64>().ok()?.checked_mul(1024)
        })
    };
    (value_bytes("MemTotal:"), value_bytes("MemAvailable:"))
}

fn parse_npu_core_load(raw: &str, core_count: u8) -> Vec<Option<u8>> {
    (0..core_count)
        .map(|id| {
            let marker = format!("Core{id}:");
            let tail = raw.split_once(&marker)?.1.trim_start();
            let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u8>().ok().filter(|value| *value <= 100)
        })
        .collect()
}

fn read_string(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read_string(path)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{parse_meminfo, parse_npu_core_load};

    #[test]
    fn parses_available_memory_in_bytes() {
        let raw = "MemTotal:       16347760 kB\nMemFree:         1000000 kB\nMemAvailable:    9080312 kB\n";
        assert_eq!(
            parse_meminfo(raw),
            (Some(16_740_106_240), Some(9_298_239_488))
        );
    }

    #[test]
    fn parses_optional_per_core_npu_load() {
        assert_eq!(
            parse_npu_core_load("NPU load: Core0: 7%, Core1: 42%", 2),
            vec![Some(7), Some(42)]
        );
        assert_eq!(parse_npu_core_load("unavailable", 2), vec![None, None]);
    }

    #[test]
    fn rejects_invalid_npu_load_percentages() {
        assert_eq!(
            parse_npu_core_load("Core0: 101%, Core1: 0%", 2),
            vec![None, Some(0)]
        );
    }
}
