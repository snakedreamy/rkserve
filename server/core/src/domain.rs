use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreMask { Core0, Core1, Core0_1, Auto }

impl CoreMask {
    pub const fn bits(self) -> u8 { match self { Self::Core0 => 0b01, Self::Core1 => 0b10, Self::Core0_1 => 0b11, Self::Auto => 0 } }
    pub const fn label(self) -> &'static str { match self { Self::Core0 => "Core 0", Self::Core1 => "Core 1", Self::Core0_1 => "Core 0 + Core 1", Self::Auto => "Auto" } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginState { Installed, Starting, Loading, Ready, Draining, Stopped, Failed, Backoff }

#[derive(Debug, Clone, Serialize)]
pub struct NpuCore { pub id: u8, pub label: String, pub allocation_id: Option<String>, pub plugin_id: Option<String> }

#[derive(Debug, Clone, Serialize)]
pub struct NpuTopology {
    pub device: String, pub platform: String, pub driver: String, pub driver_version: Option<String>, pub runtime_version: Option<String>,
    pub total_tops_int8: f32, pub core_count: u8, pub current_frequency_hz: Option<u64>, pub available_frequencies_hz: Vec<u64>, pub load_percent: Option<u8>, pub governor: Option<String>, pub cores: Vec<NpuCore>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceTelemetry {
    pub soc_temperature_c: Option<f32>,
    pub npu_temperature_c: Option<f32>,
    pub memory_total_bytes: Option<u64>,
    pub memory_available_bytes: Option<u64>,
    pub npu_core_load_percent: Vec<Option<u8>>,
}

/// Optional display translations keyed by locale, then by stable field path.
/// Base manifest fields remain the canonical English API values.
pub type PluginTranslations = BTreeMap<String, BTreeMap<String, String>>;

#[derive(Debug, Clone, Serialize)]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub state: PluginState,
    pub allowed_masks: Vec<CoreMask>,
    pub default_mask: CoreMask,
    pub max_concurrency: u32,
    pub queue_size: u32,
    pub request_timeout_ms: u64,
    pub capabilities: Vec<CapabilityDefinition>,
    pub configuration: Vec<ConfigurationField>,
    pub configuration_values: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub translations: PluginTranslations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityInputKind { Image, Audio, Text, Binary }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityOutputKind { Detections, Audio, Text, Json, Binary }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityRenderer { AudioPlayer, ImageDetectionOverlay, ImagePoseOverlay, SpeechTranscription }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityPresentation {
    pub renderer: Option<CapabilityRenderer>,
    pub input_hint: Option<String>,
    pub input_placeholder: Option<String>,
    #[serde(default)]
    pub text_examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_kind: CapabilityInputKind,
    pub accepted_content_types: Vec<String>,
    pub max_input_bytes: usize,
    pub output_kind: CapabilityOutputKind,
    pub output_content_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<CapabilityPresentation>,
    #[serde(default)]
    pub parameters: Vec<ConfigurationField>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationKind { String, Number, Boolean, Select }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationOptionsSource { RknnModels }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigurationField {
    pub key: String, pub label: String, pub description: String, pub kind: ConfigurationKind,
    #[serde(default)] pub required: bool,
    pub default: Option<String>,
    #[serde(default)] pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options_from: Option<ConfigurationOptionsSource>,
    pub min: Option<f64>, pub max: Option<f64>,
}

pub fn validate_field_value(field: &ConfigurationField, value: &str) -> Result<(), &'static str> {
    match field.kind {
        ConfigurationKind::String => {}
        ConfigurationKind::Boolean if !matches!(value, "true" | "false") => return Err("must be true or false"),
        ConfigurationKind::Select if !field.options.iter().any(|option| option == value) => return Err("is not an allowed option"),
        ConfigurationKind::Number => {
            let number: f64 = value.parse().map_err(|_| "must be a number")?;
            if !number.is_finite() || field.min.is_some_and(|min| number < min) || field.max.is_some_and(|max| number > max) { return Err("is outside its allowed range"); }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Allocation {
    pub lease_id: String,
    pub plugin_id: String,
    pub core_mask: CoreMask,
    pub created_at_unix_ms: u64,
}
