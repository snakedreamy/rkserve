use std::{collections::{BTreeMap, HashSet}, fs, path::{Path, PathBuf}};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::domain::{CapabilityDefinition, CapabilityInputKind, CapabilityOutputKind, CapabilityRenderer, ConfigurationField, ConfigurationKind, ConfigurationOptionsSource, CoreMask, PluginState, PluginSummary, PluginTranslations, validate_field_value};

#[derive(Debug, Clone)]
pub struct PluginDefinition { pub summary: PluginSummary, pub plugin_dir: PathBuf, pub executable: PathBuf }

impl PluginDefinition {
    pub fn resolve_configuration(&self, supplied: &BTreeMap<String, String>) -> Result<BTreeMap<String, String>> {
        for key in supplied.keys() { anyhow::ensure!(self.summary.configuration.iter().any(|field| &field.key == key), "unknown configuration key '{key}'"); }
        let mut resolved = BTreeMap::new();
        for field in &self.summary.configuration {
            let value = supplied.get(&field.key).cloned().or_else(|| field.default.clone());
            let Some(value) = value else { anyhow::ensure!(!field.required, "configuration '{}' is required", field.key); continue; };
            validate_configuration_value(field, &value)?;
            resolved.insert(field.key.clone(), value);
        }
        Ok(resolved)
    }
}

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u32,
    plugin: PluginSection,
    resources: ResourcesSection,
    capabilities: Vec<CapabilityDefinition>,
    #[serde(default)] configuration: ConfigurationSection,
    #[serde(default)] translations: PluginTranslations,
}

#[derive(Debug, Default, Deserialize)]
struct ConfigurationSection { #[serde(default)] fields: Vec<ConfigurationField> }

#[derive(Debug, Deserialize)]
struct PluginSection { id: String, name: String, version: String, protocol_version: u32, executable: String }

#[derive(Debug, Deserialize)]
struct ResourcesSection { npu: NpuSection }

#[derive(Debug, Deserialize)]
struct NpuSection {
    allowed_masks: Vec<CoreMask>,
    default_mask: CoreMask,
    max_concurrency: u32,
    queue_size: u32,
    request_timeout_ms: u64,
}

pub fn discover_plugins(root: &Path) -> Result<Vec<PluginDefinition>> {
    if !root.exists() { return Ok(Vec::new()); }
    let mut plugins = Vec::new();
    for entry in fs::read_dir(root).context("read plugin directory")? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() { continue; }
        let plugin_dir = entry.path().canonicalize().context("canonicalize plugin directory")?;
        let path = plugin_dir.join("plugin.toml");
        if !path.is_file() { continue; }
        let raw = fs::read_to_string(&path).with_context(|| format!("read manifest {}", path.display()))?;
        let mut manifest: Manifest = toml::from_str(&raw).with_context(|| format!("parse manifest {}", path.display()))?;
        resolve_configuration_options(&mut manifest.configuration.fields, &plugin_dir)?;
        let executable = validate_manifest(&manifest, &plugin_dir)?;
        plugins.push(PluginDefinition {
            plugin_dir, executable,
            summary: PluginSummary {
                id: manifest.plugin.id, name: manifest.plugin.name, version: manifest.plugin.version, state: PluginState::Installed,
                allowed_masks: manifest.resources.npu.allowed_masks, default_mask: manifest.resources.npu.default_mask,
                max_concurrency: manifest.resources.npu.max_concurrency, queue_size: manifest.resources.npu.queue_size,
                request_timeout_ms: manifest.resources.npu.request_timeout_ms, capabilities: manifest.capabilities,
                configuration: manifest.configuration.fields, configuration_values: BTreeMap::new(),
                translations: manifest.translations,
            },
        });
    }
    plugins.sort_by(|a, b| a.summary.id.cmp(&b.summary.id));
    Ok(plugins)
}

fn resolve_configuration_options(fields: &mut [ConfigurationField], plugin_dir: &Path) -> Result<()> {
    for field in fields {
        let Some(source) = field.options_from else { continue; };
        anyhow::ensure!(field.kind == ConfigurationKind::Select, "dynamic options require select configuration '{}'", field.key);
        field.options = match source {
            ConfigurationOptionsSource::RknnModels => discover_rknn_models(plugin_dir)?,
        };
        anyhow::ensure!(!field.options.is_empty(), "configuration '{}' did not discover any options", field.key);
        if field.default.is_none() {
            field.default = field.options.first().cloned();
        }
    }
    Ok(())
}

fn discover_rknn_models(plugin_dir: &Path) -> Result<Vec<String>> {
    let models = plugin_dir.join("assets/models");
    let canonical_models = models.canonicalize().with_context(|| format!("canonicalize model directory {}", models.display()))?;
    let mut discovered = Vec::new();
    collect_rknn_models(&models, &canonical_models, &mut discovered)?;
    discovered.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    discovered.dedup_by(|left, right| left.1 == right.1);
    Ok(discovered.into_iter().map(|(_, path)| path).collect())
}

fn collect_rknn_models(directory: &Path, canonical_root: &Path, discovered: &mut Vec<(u64, String)>) -> Result<()> {
    for entry in fs::read_dir(directory).with_context(|| format!("read model directory {}", directory.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() { continue; }
        let path = entry.path();
        if file_type.is_dir() {
            collect_rknn_models(&path, canonical_root, discovered)?;
            continue;
        }
        if !file_type.is_file() || path.extension().and_then(|value| value.to_str()) != Some("rknn") { continue; }
        let metadata = path.parent().unwrap_or(directory).join("metadata.yaml");
        if !metadata.is_file() { continue; }
        let canonical = path.canonicalize().with_context(|| format!("canonicalize model {}", path.display()))?;
        anyhow::ensure!(canonical.starts_with(canonical_root), "model escapes plugin model directory: {}", path.display());
        let relative = canonical.strip_prefix(canonical_root)?.to_str()
            .context("model path is not valid UTF-8")?
            .replace(std::path::MAIN_SEPARATOR, "/");
        discovered.push((canonical.metadata()?.len(), relative));
    }
    Ok(())
}

fn validate_manifest(manifest: &Manifest, plugin_dir: &Path) -> Result<PathBuf> {
    anyhow::ensure!(manifest.schema_version == 2, "unsupported manifest schema");
    anyhow::ensure!(manifest.plugin.protocol_version == 2, "unsupported plugin protocol");
    anyhow::ensure!(!manifest.resources.npu.allowed_masks.is_empty(), "allowed_masks is empty");
    anyhow::ensure!(manifest.resources.npu.allowed_masks.contains(&manifest.resources.npu.default_mask), "default_mask is not included in allowed_masks");
    anyhow::ensure!(manifest.resources.npu.max_concurrency > 0, "max_concurrency must be positive");
    anyhow::ensure!(manifest.resources.npu.queue_size > 0, "queue_size must be positive");
    anyhow::ensure!(manifest.resources.npu.request_timeout_ms > 0, "request timeout must be positive");
    anyhow::ensure!(!manifest.capabilities.is_empty(), "plugin must declare at least one capability");
    validate_capabilities(&manifest.capabilities)?;
    validate_configuration_fields(&manifest.configuration.fields)?;

    let canonical_dir = plugin_dir.canonicalize().context("canonicalize plugin directory")?;
    let executable = plugin_dir.join(&manifest.plugin.executable);
    let canonical_executable = executable.canonicalize().with_context(|| format!("plugin executable does not exist: {}", executable.display()))?;
    anyhow::ensure!(canonical_executable.starts_with(&canonical_dir), "plugin executable escapes its plugin directory");
    anyhow::ensure!(canonical_executable.is_file(), "plugin executable is not a file");
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        anyhow::ensure!(canonical_executable.metadata()?.permissions().mode() & 0o111 != 0, "plugin executable is not executable");
    }
    Ok(canonical_executable)
}

fn validate_capabilities(capabilities: &[CapabilityDefinition]) -> Result<()> {
    let mut capability_ids = HashSet::new();
    for capability in capabilities {
        anyhow::ensure!(valid_identifier(&capability.id), "invalid capability id '{}'", capability.id);
        anyhow::ensure!(capability_ids.insert(&capability.id), "duplicate capability id '{}'", capability.id);
        anyhow::ensure!(!capability.name.trim().is_empty(), "capability name is empty");
        anyhow::ensure!(!capability.accepted_content_types.is_empty(), "capability accepted_content_types is empty");
        anyhow::ensure!(capability.max_input_bytes > 0 && capability.max_input_bytes <= 64 * 1024 * 1024, "capability max_input_bytes is outside 1..64 MiB");
        anyhow::ensure!(!capability.output_content_type.trim().is_empty(), "capability output_content_type is empty");
        if let Some(presentation) = &capability.presentation {
            if let Some(hint) = &presentation.input_hint { anyhow::ensure!(!hint.trim().is_empty(), "capability '{}' input_hint is empty", capability.id); }
            if let Some(placeholder) = &presentation.input_placeholder {
                anyhow::ensure!(capability.input_kind == CapabilityInputKind::Text, "capability '{}' input_placeholder requires text input", capability.id);
                anyhow::ensure!(!placeholder.trim().is_empty(), "capability '{}' input_placeholder is empty", capability.id);
            }
            if !presentation.text_examples.is_empty() {
                anyhow::ensure!(capability.input_kind == CapabilityInputKind::Text, "capability '{}' text_examples require text input", capability.id);
                anyhow::ensure!(presentation.text_examples.len() <= 6, "capability '{}' has too many text examples", capability.id);
                for example in &presentation.text_examples {
                    anyhow::ensure!(!example.trim().is_empty(), "capability '{}' has an empty text example", capability.id);
                    anyhow::ensure!(example.len() <= capability.max_input_bytes, "capability '{}' text example exceeds max_input_bytes", capability.id);
                }
            }
            match presentation.renderer {
                Some(CapabilityRenderer::AudioPlayer) => anyhow::ensure!(capability.output_kind == CapabilityOutputKind::Audio && capability.output_content_type.starts_with("audio/"), "capability '{}' audio_player requires audio output", capability.id),
                Some(CapabilityRenderer::ImageDetectionOverlay) => anyhow::ensure!(capability.input_kind == CapabilityInputKind::Image && capability.output_kind == CapabilityOutputKind::Detections && capability.output_content_type == "application/json", "capability '{}' image_detection_overlay requires image input and detections JSON output", capability.id),
                Some(CapabilityRenderer::ImagePoseOverlay) => anyhow::ensure!(capability.input_kind == CapabilityInputKind::Image && capability.output_kind == CapabilityOutputKind::Json && capability.output_content_type == "application/json", "capability '{}' image_pose_overlay requires image input and JSON output", capability.id),
                Some(CapabilityRenderer::SpeechTranscription) => anyhow::ensure!(capability.input_kind == CapabilityInputKind::Audio && capability.output_kind == CapabilityOutputKind::Json && capability.output_content_type == "application/json", "capability '{}' speech_transcription requires audio input and JSON output", capability.id),
                None => {}
            }
        }
        let mut parameter_keys = HashSet::new();
        for parameter in &capability.parameters {
            anyhow::ensure!(valid_identifier(&parameter.key), "invalid capability parameter '{}'", parameter.key);
            anyhow::ensure!(parameter_keys.insert(&parameter.key), "duplicate capability parameter '{}'", parameter.key);
            anyhow::ensure!(!parameter.label.trim().is_empty(), "capability parameter label is empty");
            if parameter.kind == ConfigurationKind::Select { anyhow::ensure!(!parameter.options.is_empty(), "select capability parameter '{}' has no options", parameter.key); }
            if let Some(default) = &parameter.default { validate_configuration_value(parameter, default)?; }
        }
    }
    Ok(())
}

fn validate_configuration_fields(fields: &[ConfigurationField]) -> Result<()> {
    let mut configuration_keys = HashSet::new();
    for field in fields {
        anyhow::ensure!(valid_identifier(&field.key), "invalid configuration key '{}'", field.key);
        anyhow::ensure!(configuration_keys.insert(&field.key), "duplicate configuration key '{}'", field.key);
        anyhow::ensure!(!field.label.trim().is_empty(), "configuration label is empty");
        if field.options_from.is_some() { anyhow::ensure!(field.kind == ConfigurationKind::Select, "dynamic options require select configuration '{}'", field.key); }
        if field.kind == ConfigurationKind::Select { anyhow::ensure!(!field.options.is_empty(), "select configuration '{}' has no options", field.key); }
        if let Some(default) = &field.default { validate_configuration_value(field, default)?; }
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool { !value.is_empty() && value.chars().all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')) }
fn validate_configuration_value(field: &ConfigurationField, value: &str) -> Result<()> { validate_field_value(field, value).map_err(|reason| anyhow::anyhow!("configuration '{}' {reason}", field.key)) }

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFESTS: [&str; 5] = [
        include_str!("../../../plugins/matcha-tts/plugin.toml"),
        include_str!("../../../plugins/sensevoice-asr/plugin.toml"),
        include_str!("../../../plugins/yolo26/plugin.toml"),
        include_str!("../../../plugins/yolov8-pose/plugin.toml"),
        include_str!("../../../plugins/zipformer-asr/plugin.toml"),
    ];

    fn display_fields(manifest: &Manifest) -> BTreeMap<String, String> {
        let mut fields = BTreeMap::from([("plugin.name".into(), manifest.plugin.name.clone())]);
        for capability in &manifest.capabilities {
            let prefix = format!("capabilities.{}", capability.id);
            fields.insert(format!("{prefix}.name"), capability.name.clone());
            fields.insert(format!("{prefix}.description"), capability.description.clone());
            if let Some(presentation) = &capability.presentation {
                for (key, value) in [("input_hint", &presentation.input_hint), ("input_placeholder", &presentation.input_placeholder)] {
                    if let Some(value) = value { fields.insert(format!("{prefix}.presentation.{key}"), value.clone()); }
                }
            }
            for parameter in &capability.parameters {
                fields.insert(format!("{prefix}.parameters.{}.label", parameter.key), parameter.label.clone());
                fields.insert(format!("{prefix}.parameters.{}.description", parameter.key), parameter.description.clone());
            }
        }
        for field in &manifest.configuration.fields {
            fields.insert(format!("configuration.{}.label", field.key), field.label.clone());
            fields.insert(format!("configuration.{}.description", field.key), field.description.clone());
        }
        fields
    }

    #[test]
    fn bundled_manifests_have_english_defaults_and_complete_chinese_display_text() {
        for raw in MANIFESTS {
            let manifest: Manifest = toml::from_str(raw).unwrap();
            let fields = display_fields(&manifest);
            let chinese = &manifest.translations["zh-CN"];
            assert_eq!(fields.keys().collect::<Vec<_>>(), chinese.keys().collect::<Vec<_>>(), "{}", manifest.plugin.id);
            for (key, value) in fields {
                assert!(!value.trim().is_empty(), "{key}");
                assert!(!value.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)), "{key}: {value}");
                assert!(!chinese[&key].trim().is_empty(), "{key}");
            }
            // Removing the optional section must not change any canonical display field.
            let legacy: Manifest = toml::from_str(raw.split("[translations.zh-CN]").next().unwrap()).unwrap();
            assert!(legacy.translations.is_empty());
            assert_eq!(display_fields(&manifest), display_fields(&legacy));
        }
    }

    #[test]
    fn discovery_exposes_translations_without_changing_canonical_values() {
        struct Scratch(PathBuf);
        impl Drop for Scratch { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = Scratch(std::env::temp_dir().join(format!("rkserve-translations-{}-{nonce}", std::process::id())));
        let plugin = root.0.join("matcha-tts");
        fs::create_dir_all(plugin.join("bin")).unwrap();
        let worker = plugin.join("bin/worker");
        fs::write(&worker, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&worker, fs::Permissions::from_mode(0o755)).unwrap();
        }
        for localized in [false, true] {
            let raw = if localized { MANIFESTS[0] } else { MANIFESTS[0].split("[translations.zh-CN]").next().unwrap() };
            fs::write(plugin.join("plugin.toml"), raw).unwrap();
            let plugins = discover_plugins(&root.0).unwrap();
            let summary = &plugins[0].summary;
            assert_eq!(summary.id, "matcha-tts");
            assert_eq!(summary.name, "Matcha Chinese/English TTS");
            assert_eq!(plugins[0].resolve_configuration(&BTreeMap::new()).unwrap()["speed"], "1.0");
            let json = serde_json::to_value(summary).unwrap();
            if localized {
                assert_eq!(json["translations"]["zh-CN"]["plugin.name"], "Matcha 中英文 TTS");
            } else { assert!(json.get("translations").is_none()); }
        }
    }
}
